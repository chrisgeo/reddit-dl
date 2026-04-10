mod api;
mod config;
mod download;
mod error;
mod post;
mod progress;
mod sources;
mod storage;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "reddit-dl",
    about = "Bulk Reddit downloader with incremental sync"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Path to config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(long, short, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Sync posts from configured sources
    Sync {
        /// Sources to sync. Repeatable. Examples: "friends", "saved", "subreddit:pics", "user:spez"
        /// When omitted, syncs the default sources from config (friends, follows, saved).
        #[arg(long)]
        source: Vec<String>,

        /// Ignore cursors and re-scan everything (still deduplicates)
        #[arg(long)]
        full: bool,

        /// Maximum number of posts to fetch per source
        #[arg(long)]
        limit: Option<u32>,

        /// Override output directory
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Show sync status and cursor positions
    Status,
    /// Authenticate with Reddit and verify credentials
    Auth,
}

fn init_tracing(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("reddit_dl=debug")
    } else {
        EnvFilter::new("reddit_dl=info")
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn resolve_config_path(cli_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(path) = cli_path {
        return Ok(path.clone());
    }
    if let Some(found) = config::find_config() {
        return Ok(found);
    }
    let search_paths = config::config_search_paths();
    let paths_display: Vec<String> = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect();
    bail!(
        "No config file found. Specify one with --config or place it at one of:\n{}\n\
         See config.example.toml for the expected format.",
        paths_display.join("\n")
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let config_path = resolve_config_path(cli.config.as_ref())?;
    let config = config::Config::load(&config_path)?;
    tracing::debug!("Loaded config from {}", config_path.display());

    match cli.command {
        Command::Sync {
            source,
            full,
            limit,
            output,
        } => {
            let output_dir = output.unwrap_or_else(|| config.resolve_output_dir());
            tracing::info!("Syncing to {}", output_dir.display());
            tracing::debug!(?source, full, ?limit, "Sync parameters");

            tracing::info!("Authenticating...");
            let client = api::RedditClient::new(&config.auth)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let me = client
                .get_json::<api::MeResponse>("/api/v1/me", &[])
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            tracing::info!("Authenticated as {}", me.name);

            let active_sources = sources::build_sources(&config.sources, &me.name, &source);

            if active_sources.is_empty() {
                println!("No sources configured or matched the filter.");
                return Ok(());
            }

            std::fs::create_dir_all(&output_dir)?;

            let fs = storage::filesystem::OutputManager::new(
                output_dir.clone(),
                config.download.file_naming.clone(),
            );

            let db =
                storage::db::Database::open(&fs.db_path()).map_err(|e| anyhow::anyhow!("{}", e))?;

            // HTTP client for non-API downloads (images, videos)
            let http_client = reqwest::Client::builder()
                .user_agent("reddit-dl/0.1.0")
                .timeout(std::time::Duration::from_secs(60))
                .build()?;

            // RedGifs client with token caching for the session
            let redgifs_client = download::redgifs::RedGifsClient::new(http_client.clone());

            let mut total_downloaded = 0u32;
            let mut total_skipped = 0u32;
            let mut source_errors = 0u32;

            // Progress tracking: disabled when --verbose (conflicts with tracing output)
            let tracker = if !cli.verbose {
                Some(progress::ProgressTracker::new())
            } else {
                None
            };

            for src in &active_sources {
                tracing::info!("Syncing {}/{}", src.source_type(), src.source_name());

                // Show fetch spinner while we retrieve the post list
                let spinner = tracker
                    .as_ref()
                    .map(|t| t.add_fetch_spinner(src.source_type(), src.source_name()));

                // Load cursor unless --full
                let cursor_data = if full {
                    None
                } else {
                    db.get_cursor(src.source_type(), src.source_name())
                        .map_err(|e| anyhow::anyhow!("{}", e))?
                };
                let cursor_id = cursor_data.as_ref().map(|c| c.last_post_id.as_str());

                match src.fetch_posts(&client, cursor_id, limit).await {
                    Ok(posts) => {
                        // Retire the fetch spinner
                        if let Some(sp) = spinner {
                            sp.finish_and_clear();
                        }

                        if posts.is_empty() {
                            tracing::info!(
                                "  No new posts from {}/{}",
                                src.source_type(),
                                src.source_name()
                            );
                            continue;
                        }

                        tracing::info!(
                            "  Found {} posts from {}/{}",
                            posts.len(),
                            src.source_type(),
                            src.source_name()
                        );

                        // Switch to a deterministic progress bar now that we know the total
                        let bar = tracker.as_ref().map(|t| {
                            t.add_source_bar(
                                src.source_type(),
                                src.source_name(),
                                posts.len() as u64,
                            )
                        });

                        db.begin_transaction()
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        let mut batch_downloaded = 0u32;
                        let mut batch_skipped = 0u32;
                        let mut newest_post: Option<(&str, f64)> = None;

                        for post in &posts {
                            // Track newest post for cursor update
                            if newest_post.is_none() {
                                newest_post = Some((&post.name, post.created_utc));
                            }

                            // Dedup check
                            let already = db
                                .is_downloaded(&post.name)
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            if already {
                                batch_skipped += 1;
                                if let Some(pb) = &bar {
                                    pb.inc(1);
                                    pb.set_message(format!(
                                        "{} downloaded, {} skipped",
                                        batch_downloaded, batch_skipped
                                    ));
                                }
                                continue;
                            }

                            // Download
                            let file_count = download::download_post(
                                &client,
                                &http_client,
                                &redgifs_client,
                                post,
                                src.source_type(),
                                src.source_name(),
                                &fs,
                                &config.download,
                            )
                            .await
                            .unwrap_or_else(|e| {
                                tracing::warn!("Download failed for {}: {}", post.id, e);
                                0
                            });

                            // Record in DB
                            db.record_post(crate::storage::db::RecordPost {
                                post_id: &post.name,
                                source_type: src.source_type(),
                                source_name: src.source_name(),
                                title: &post.title,
                                author: &post.author,
                                permalink: &post.permalink,
                                created_utc: post.created_utc,
                                media_count: file_count,
                            })
                            .map_err(|e| anyhow::anyhow!("{}", e))?;

                            batch_downloaded += 1;
                            if let Some(pb) = &bar {
                                pb.inc(1);
                                pb.set_message(format!(
                                    "{} downloaded, {} skipped",
                                    batch_downloaded, batch_skipped
                                ));
                            }
                        }

                        // Update cursor to newest post
                        if let Some((name, utc)) = newest_post {
                            db.update_cursor(
                                src.source_type(),
                                src.source_name(),
                                name,
                                utc as i64,
                            )
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        }

                        db.commit().map_err(|e| anyhow::anyhow!("{}", e))?;

                        if let Some(pb) = bar {
                            pb.finish_with_message(format!(
                                "{} downloaded, {} skipped",
                                batch_downloaded, batch_skipped
                            ));
                        }

                        tracing::info!(
                            "  Downloaded: {}, Skipped (dedup): {}",
                            batch_downloaded,
                            batch_skipped
                        );
                        total_downloaded += batch_downloaded;
                        total_skipped += batch_skipped;
                    }
                    Err(e) => {
                        if let Some(sp) = spinner {
                            sp.finish_and_clear();
                        }
                        tracing::error!(
                            "Failed to fetch from {}/{}: {}",
                            src.source_type(),
                            src.source_name(),
                            e
                        );
                        source_errors += 1;
                    }
                }
            }

            println!(
                "Sync complete. Downloaded: {}, Skipped: {}, Errors: {}",
                total_downloaded, total_skipped, source_errors
            );
        }
        Command::Status => {
            let output_dir = config.resolve_output_dir();
            let db_path = output_dir.join("reddit-dl.db");

            if !db_path.exists() {
                println!("No sync data found. Run 'reddit-dl sync' first.");
                return Ok(());
            }

            let db = storage::db::Database::open(&db_path).map_err(|e| anyhow::anyhow!("{}", e))?;

            let stats = db.get_stats().map_err(|e| anyhow::anyhow!("{}", e))?;
            let cursors = db.get_all_cursors().map_err(|e| anyhow::anyhow!("{}", e))?;

            if stats.total_posts == 0 && cursors.is_empty() {
                println!("No sync data found. Run 'reddit-dl sync' first.");
                return Ok(());
            }

            println!("reddit-dl status");
            println!("{}", "=".repeat(60));
            println!("Total posts downloaded: {}", stats.total_posts);
            println!();

            if !stats.posts_by_source.is_empty() {
                println!("Posts by source:");
                // Calculate column widths
                let type_width = stats
                    .posts_by_source
                    .iter()
                    .map(|(t, _, _)| t.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);
                let name_width = stats
                    .posts_by_source
                    .iter()
                    .map(|(_, n, _)| n.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);

                println!(
                    "  {:<type_width$}  {:<name_width$}  {:>6}",
                    "Type",
                    "Name",
                    "Posts",
                    type_width = type_width,
                    name_width = name_width,
                );
                println!(
                    "  {}  {}  ------",
                    "-".repeat(type_width),
                    "-".repeat(name_width),
                );
                for (source_type, source_name, count) in &stats.posts_by_source {
                    println!(
                        "  {:<type_width$}  {:<name_width$}  {:>6}",
                        source_type,
                        source_name,
                        count,
                        type_width = type_width,
                        name_width = name_width,
                    );
                }
                println!();
            }

            if !cursors.is_empty() {
                println!("Sync cursors (last known position per source):");
                let type_width = cursors
                    .iter()
                    .map(|(t, _, _, _)| t.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);
                let name_width = cursors
                    .iter()
                    .map(|(_, n, _, _)| n.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);
                let id_width = cursors
                    .iter()
                    .map(|(_, _, c, _)| c.last_post_id.len())
                    .max()
                    .unwrap_or(7)
                    .max(7);

                println!(
                    "  {:<type_width$}  {:<name_width$}  {:<id_width$}  Last Sync",
                    "Type",
                    "Name",
                    "Last ID",
                    type_width = type_width,
                    name_width = name_width,
                    id_width = id_width,
                );
                println!(
                    "  {}  {}  {}  {}",
                    "-".repeat(type_width),
                    "-".repeat(name_width),
                    "-".repeat(id_width),
                    "-".repeat(19),
                );
                for (source_type, source_name, cursor, updated_at) in &cursors {
                    use chrono::TimeZone;
                    let dt = chrono::Utc
                        .timestamp_opt(*updated_at, 0)
                        .single()
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    println!(
                        "  {:<type_width$}  {:<name_width$}  {:<id_width$}  {}",
                        source_type,
                        source_name,
                        cursor.last_post_id,
                        dt,
                        type_width = type_width,
                        name_width = name_width,
                        id_width = id_width,
                    );
                }
            }
        }
        Command::Auth => {
            let client = api::RedditClient::new(&config.auth)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let me = client
                .get_json::<api::MeResponse>("/api/v1/me", &[])
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Authenticated as {}", me.name);
        }
    }

    Ok(())
}

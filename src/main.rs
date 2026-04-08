mod api;
mod config;
mod download;
mod error;
mod post;
mod sources;
mod storage;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "reddit-dl", about = "Bulk Reddit downloader with incremental sync")]
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
        /// Only sync a specific source (e.g., "friends", "saved", "subreddit:pics", "user:spez")
        #[arg(long)]
        source: Option<String>,

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
    if let Some(default) = config::default_config_path() {
        if default.exists() {
            return Ok(default);
        }
    }
    bail!(
        "No config file found. Specify one with --config or create one at {:?}\n\
         See config.example.toml for the expected format.",
        config::default_config_path().unwrap_or_default()
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
            tracing::debug!(
                ?source,
                full,
                ?limit,
                "Sync parameters"
            );

            tracing::info!("Authenticating...");
            let client = api::RedditClient::new(&config.auth).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let me = client
                .get_json::<api::MeResponse>("/api/v1/me", &[])
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            tracing::info!("Authenticated as {}", me.name);

            let active_sources = sources::build_sources(
                &config.sources,
                &me.name,
                source.as_deref(),
            );

            if active_sources.is_empty() {
                println!("No sources configured or matched the filter.");
                return Ok(());
            }

            std::fs::create_dir_all(&output_dir)?;

            let fs = storage::filesystem::OutputManager::new(
                output_dir.clone(),
                config.download.file_naming.clone(),
            );

            let db = storage::db::Database::open(&fs.db_path())
                .map_err(|e| anyhow::anyhow!("{}", e))?;

            // HTTP client for non-API downloads (images, videos)
            let http_client = reqwest::Client::builder()
                .user_agent("reddit-dl/0.1.0")
                .timeout(std::time::Duration::from_secs(60))
                .build()?;

            let mut total_downloaded = 0u32;
            let mut total_skipped = 0u32;
            let mut source_errors = 0u32;

            for src in &active_sources {
                tracing::info!("Syncing {}/{}", src.source_type(), src.source_name());

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
                        if posts.is_empty() {
                            tracing::info!("  No new posts from {}/{}", src.source_type(), src.source_name());
                            continue;
                        }

                        tracing::info!("  Found {} posts from {}/{}", posts.len(), src.source_type(), src.source_name());

                        db.begin_transaction().map_err(|e| anyhow::anyhow!("{}", e))?;
                        let mut batch_downloaded = 0u32;
                        let mut batch_skipped = 0u32;
                        let mut newest_post: Option<(&str, f64)> = None;

                        for post in &posts {
                            // Track newest post for cursor update
                            if newest_post.is_none() {
                                newest_post = Some((&post.name, post.created_utc));
                            }

                            // Dedup check
                            let already = db.is_downloaded(&post.name)
                                .map_err(|e| anyhow::anyhow!("{}", e))?;
                            if already {
                                batch_skipped += 1;
                                continue;
                            }

                            // Download
                            let file_count = download::download_post(
                                &client,
                                &http_client,
                                post,
                                src.source_type(),
                                src.source_name(),
                                &fs,
                                &config.download,
                            ).await.unwrap_or_else(|e| {
                                tracing::warn!("Download failed for {}: {}", post.id, e);
                                0
                            });

                            // Record in DB
                            db.record_post(
                                &post.name,
                                src.source_type(),
                                src.source_name(),
                                &post.title,
                                &post.author,
                                &post.permalink,
                                post.created_utc,
                                file_count,
                            ).map_err(|e| anyhow::anyhow!("{}", e))?;

                            batch_downloaded += 1;
                        }

                        // Update cursor to newest post
                        if let Some((name, utc)) = newest_post {
                            db.update_cursor(
                                src.source_type(),
                                src.source_name(),
                                name,
                                utc as i64,
                            ).map_err(|e| anyhow::anyhow!("{}", e))?;
                        }

                        db.commit().map_err(|e| anyhow::anyhow!("{}", e))?;

                        tracing::info!(
                            "  Downloaded: {}, Skipped (dedup): {}",
                            batch_downloaded,
                            batch_skipped
                        );
                        total_downloaded += batch_downloaded;
                        total_skipped += batch_skipped;
                    }
                    Err(e) => {
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
            // TODO: Phase 7 will implement status display
            println!("Status not yet implemented. Config loaded successfully.");
        }
        Command::Auth => {
            let client = api::RedditClient::new(&config.auth).await
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

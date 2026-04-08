mod api;
mod config;
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
            tracing::info!("Output directory: {}", output_dir.display());

            let _fs = storage::filesystem::OutputManager::new(
                output_dir.clone(),
                config.download.file_naming.clone(),
            );

            for src in &active_sources {
                tracing::info!("Fetching from {}/{}", src.source_type(), src.source_name());

                let cursor = if full { None } else { None }; // TODO: Phase 6 loads from DB
                match src.fetch_posts(&client, cursor, limit).await {
                    Ok(posts) => {
                        tracing::info!(
                            "  Found {} posts from {}/{}",
                            posts.len(),
                            src.source_type(),
                            src.source_name()
                        );
                        for post in &posts {
                            tracing::debug!("  - {} | {}", post.id, post.title);
                            // TODO: Phase 5 will download media/metadata here
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to fetch from {}/{}: {}",
                            src.source_type(),
                            src.source_name(),
                            e
                        );
                    }
                }
            }

            println!("Sync complete.");
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

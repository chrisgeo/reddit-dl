pub mod follows;
pub mod friends;
pub mod saved;
pub mod subreddit;
pub mod user;

use async_trait::async_trait;

use crate::api::RedditClient;
use crate::config::SourcesConfig;
use crate::error::Result;
use crate::post::Post;

/// A source of Reddit posts to download
#[async_trait]
pub trait Source: Send + Sync {
    /// The type of this source (e.g., "friends", "saved", "subreddits")
    fn source_type(&self) -> &str;

    /// The name within that type (e.g., username, subreddit name, "saved")
    fn source_name(&self) -> &str;

    /// Fetch posts from this source.
    /// If `cursor` is Some, stop when reaching that post fullname.
    /// Returns posts newest-first.
    async fn fetch_posts(
        &self,
        client: &RedditClient,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Post>>;
}

/// Build the list of sources to sync.
///
/// When `--source` flags are given, they define exactly what to sync:
///   - `friends`, `follows`, `saved` — built-in category sources
///   - `user:X` — ad-hoc user (no config entry needed)
///   - `subreddit:X` — ad-hoc subreddit (no config entry needed)
///
/// When no `--source` flags are given, falls back to config defaults
/// (friends, follows, saved booleans).
pub fn build_sources(
    config: &SourcesConfig,
    username: &str,
    cli_sources: &[String],
) -> Vec<Box<dyn Source>> {
    // Explicit --source flags: build exactly what was requested.
    if !cli_sources.is_empty() {
        let mut sources: Vec<Box<dyn Source>> = Vec::new();
        for src in cli_sources {
            if let Some(target) = src.strip_prefix("user:") {
                sources.push(Box::new(user::UserSource::new(target.to_string())));
            } else if let Some(target) = src.strip_prefix("subreddit:") {
                sources.push(Box::new(subreddit::SubredditSource::new(
                    target.to_string(),
                )));
            } else {
                match src.as_str() {
                    "friends" => sources.push(Box::new(friends::FriendsSource::new(
                        username.to_string(),
                    ))),
                    "follows" => sources.push(Box::new(follows::FollowsSource::new(
                        username.to_string(),
                    ))),
                    "saved" => {
                        sources.push(Box::new(saved::SavedSource::new(username.to_string())))
                    }
                    other => {
                        tracing::warn!("Unknown source '{}', skipping", other);
                    }
                }
            }
        }
        return sources;
    }

    // No --source flags: use config defaults.
    let mut sources: Vec<Box<dyn Source>> = Vec::new();

    if config.friends {
        sources.push(Box::new(friends::FriendsSource::new(username.to_string())));
    }
    if config.follows {
        sources.push(Box::new(follows::FollowsSource::new(username.to_string())));
    }
    if config.saved {
        sources.push(Box::new(saved::SavedSource::new(username.to_string())));
    }

    sources
}

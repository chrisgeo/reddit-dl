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

/// Build the list of active sources from config, optionally filtered by CLI --source flag.
///
/// When `--source user:X` or `--source subreddit:X` is given and X is not already
/// in the config, the source is created ad-hoc so that CLI one-offs work without
/// editing the config file.
pub fn build_sources(
    config: &SourcesConfig,
    username: &str,
    source_filter: Option<&str>,
) -> Vec<Box<dyn Source>> {
    // If the filter specifies an ad-hoc user or subreddit, handle it directly.
    if let Some(filter) = source_filter {
        if let Some(target_user) = filter.strip_prefix("user:") {
            return vec![Box::new(user::UserSource::new(target_user.to_string()))];
        }
        if let Some(target_sub) = filter.strip_prefix("subreddit:") {
            return vec![Box::new(subreddit::SubredditSource::new(
                target_sub.to_string(),
            ))];
        }
    }

    let mut sources: Vec<Box<dyn Source>> = Vec::new();

    let should_include = |source_type: &str| -> bool {
        match source_filter {
            None => true,
            Some(filter) => source_type == filter,
        }
    };

    if config.friends && should_include("friends") {
        sources.push(Box::new(friends::FriendsSource::new(username.to_string())));
    }

    if config.follows && should_include("follows") {
        sources.push(Box::new(follows::FollowsSource::new(username.to_string())));
    }

    if config.saved && should_include("saved") {
        sources.push(Box::new(saved::SavedSource::new(username.to_string())));
    }

    if source_filter.is_none() || should_include("subreddits") {
        for sub in &config.subreddits {
            sources.push(Box::new(subreddit::SubredditSource::new(sub.clone())));
        }
    }
    if source_filter.is_none() || should_include("users") {
        for user in &config.users {
            sources.push(Box::new(user::UserSource::new(user.clone())));
        }
    }

    sources
}

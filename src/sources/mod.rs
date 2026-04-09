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

/// Build the list of active sources from config, optionally filtered by CLI --source flag
pub fn build_sources(
    config: &SourcesConfig,
    username: &str,
    source_filter: Option<&str>,
) -> Vec<Box<dyn Source>> {
    let mut sources: Vec<Box<dyn Source>> = Vec::new();

    let should_include = |source_type: &str, source_name: Option<&str>| -> bool {
        match source_filter {
            None => true,
            Some(filter) => {
                if let Some(rest) = filter.strip_prefix("subreddit:") {
                    source_type == "subreddits" && source_name == Some(rest)
                } else if let Some(rest) = filter.strip_prefix("user:") {
                    source_type == "users" && source_name == Some(rest)
                } else {
                    source_type == filter
                }
            }
        }
    };

    if config.friends && should_include("friends", None) {
        sources.push(Box::new(friends::FriendsSource::new(username.to_string())));
    }

    if config.follows && should_include("follows", None) {
        sources.push(Box::new(follows::FollowsSource::new(username.to_string())));
    }

    if config.saved && should_include("saved", None) {
        sources.push(Box::new(saved::SavedSource::new(username.to_string())));
    }

    for sub in &config.subreddits {
        if should_include("subreddits", Some(sub)) {
            sources.push(Box::new(subreddit::SubredditSource::new(sub.clone())));
        }
    }

    for user in &config.users {
        if should_include("users", Some(user)) {
            sources.push(Box::new(user::UserSource::new(user.clone())));
        }
    }

    sources
}

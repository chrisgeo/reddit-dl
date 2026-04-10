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
                    "friends" => {
                        sources.push(Box::new(friends::FriendsSource::new(username.to_string())))
                    }
                    "follows" => {
                        sources.push(Box::new(follows::FollowsSource::new(username.to_string())))
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourcesConfig;

    fn config_all_on() -> SourcesConfig {
        SourcesConfig::new(true, true, true)
    }

    fn config_all_off() -> SourcesConfig {
        SourcesConfig::new(false, false, false)
    }

    // ── No --source flags: use config defaults ───────────────────────────────

    #[test]
    fn no_flags_with_all_enabled_returns_three_sources() {
        let sources = build_sources(&config_all_on(), "testuser", &[]);
        assert_eq!(sources.len(), 3);
        let types: Vec<&str> = sources.iter().map(|s| s.source_type()).collect();
        assert!(types.contains(&"friends"));
        assert!(types.contains(&"follows"));
        assert!(types.contains(&"saved"));
    }

    #[test]
    fn no_flags_with_all_disabled_returns_empty() {
        let sources = build_sources(&config_all_off(), "testuser", &[]);
        assert!(sources.is_empty());
    }

    #[test]
    fn no_flags_respects_individual_toggles() {
        let config = SourcesConfig::new(true, false, true);
        let sources = build_sources(&config, "testuser", &[]);
        assert_eq!(sources.len(), 2);
        let types: Vec<&str> = sources.iter().map(|s| s.source_type()).collect();
        assert!(types.contains(&"friends"));
        assert!(types.contains(&"saved"));
        assert!(!types.contains(&"follows"));
    }

    // ── Explicit --source flags override config ──────────────────────────────

    #[test]
    fn source_flag_friends_works() {
        let flags = vec!["friends".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "friends");
    }

    #[test]
    fn source_flag_saved_works() {
        let flags = vec!["saved".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "saved");
    }

    #[test]
    fn source_flag_follows_works() {
        let flags = vec!["follows".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "follows");
    }

    // ── Ad-hoc user and subreddit sources ────────────────────────────────────

    #[test]
    fn adhoc_user_creates_source_without_config() {
        let flags = vec!["user:his_and_hers_".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "users");
        assert_eq!(sources[0].source_name(), "his_and_hers_");
    }

    #[test]
    fn adhoc_subreddit_creates_source_without_config() {
        let flags = vec!["subreddit:pics".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "subreddits");
        assert_eq!(sources[0].source_name(), "pics");
    }

    // ── Multiple --source flags ──────────────────────────────────────────────

    #[test]
    fn multiple_source_flags_compose() {
        let flags = vec![
            "saved".to_string(),
            "user:someone".to_string(),
            "subreddit:rust".to_string(),
            "friends".to_string(),
        ];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources.len(), 4);

        let types: Vec<(&str, &str)> = sources
            .iter()
            .map(|s| (s.source_type(), s.source_name()))
            .collect();
        assert!(types.contains(&("saved", "saved")));
        assert!(types.contains(&("users", "someone")));
        assert!(types.contains(&("subreddits", "rust")));
        assert!(types.contains(&("friends", "all")));
    }

    // ── Explicit flags ignore config booleans ────────────────────────────────

    #[test]
    fn explicit_flags_ignore_config_entirely() {
        // Config has friends=true, but we only ask for saved
        let config = SourcesConfig::new(true, true, true);
        let flags = vec!["saved".to_string()];
        let sources = build_sources(&config, "testuser", &flags);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_type(), "saved");
    }

    // ── Unknown source type ──────────────────────────────────────────────────

    #[test]
    fn unknown_source_type_is_skipped() {
        let flags = vec!["nonexistent".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert!(sources.is_empty());
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn user_with_underscores_in_name() {
        let flags = vec!["user:__double__underscore__".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources[0].source_name(), "__double__underscore__");
    }

    #[test]
    fn subreddit_with_numbers() {
        let flags = vec!["subreddit:3dprinting".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        assert_eq!(sources[0].source_name(), "3dprinting");
    }

    #[test]
    fn duplicate_sources_are_preserved() {
        let flags = vec!["saved".to_string(), "saved".to_string()];
        let sources = build_sources(&config_all_off(), "testuser", &flags);
        // No dedup — callers responsibility if they care
        assert_eq!(sources.len(), 2);
    }
}

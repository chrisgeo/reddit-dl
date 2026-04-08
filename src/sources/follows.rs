use async_trait::async_trait;

use crate::api::endpoints;
use crate::api::RedditClient;
use crate::error::Result;
use crate::post::Post;
use super::Source;

/// Source that discovers followed users by finding subscribed subreddits
/// with the "u_" prefix (Reddit's follow mechanism subscribes you to u_username).
pub struct FollowsSource {
    _username: String,
}

impl FollowsSource {
    pub fn new(username: String) -> Self {
        Self { _username: username }
    }
}

#[async_trait]
impl Source for FollowsSource {
    fn source_type(&self) -> &str {
        "follows"
    }

    fn source_name(&self) -> &str {
        "all"
    }

    async fn fetch_posts(
        &self,
        client: &RedditClient,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Post>> {
        // Discover followed users by fetching subscriptions and filtering for u_ prefix
        let mut followed_users = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let (subs, next_after) =
                endpoints::get_subscriptions(client, after.as_deref()).await?;

            if subs.is_empty() {
                break;
            }

            for sub in &subs {
                if sub.display_name.starts_with("u_") {
                    let username = sub.display_name.strip_prefix("u_").unwrap().to_string();
                    followed_users.push(username);
                }
            }

            match next_after {
                Some(a) => after = Some(a),
                None => break,
            }
        }

        tracing::info!("Found {} followed users", followed_users.len());

        let per_user_limit = limit.unwrap_or(25);
        let mut all_posts = Vec::new();

        for user in &followed_users {
            tracing::debug!("Fetching posts from followed user: {}", user);
            match endpoints::get_user_posts(client, user, None, per_user_limit).await {
                Ok((posts, _)) => {
                    for post in posts {
                        if let Some(cursor_id) = cursor {
                            if post.name == cursor_id {
                                tracing::debug!(
                                    "Hit cursor {} for followed user {}",
                                    cursor_id,
                                    user
                                );
                                break;
                            }
                        }
                        all_posts.push(post);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch posts for followed user {}: {}", user, e);
                }
            }
        }

        Ok(all_posts)
    }
}

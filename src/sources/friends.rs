use async_trait::async_trait;

use super::Source;
use crate::api::endpoints;
use crate::api::RedditClient;
use crate::error::Result;
use crate::post::Post;

/// Meta-source that fetches the friends list, then fetches each friend's posts.
/// When used in the sync loop, this produces separate source entries per friend
/// (source_type="friends", source_name=friend_username).
pub struct FriendsSource {
    _username: String,
}

impl FriendsSource {
    pub fn new(username: String) -> Self {
        Self {
            _username: username,
        }
    }
}

#[async_trait]
impl Source for FriendsSource {
    fn source_type(&self) -> &str {
        "friends"
    }

    fn source_name(&self) -> &str {
        // This is a meta-source; individual friend posts will be tagged
        // with the friend's username as source_name in the sync loop
        "all"
    }

    async fn fetch_posts(
        &self,
        client: &RedditClient,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Post>> {
        let friends = endpoints::get_friends(client).await?;
        tracing::info!("Found {} friends", friends.len());

        let per_friend_limit = limit.unwrap_or(25);
        let mut all_posts = Vec::new();

        for friend in &friends {
            tracing::debug!("Fetching posts from friend: {}", friend.name);
            match endpoints::get_user_posts(client, &friend.name, None, per_friend_limit).await {
                Ok((posts, _)) => {
                    for post in posts {
                        if let Some(cursor_id) = cursor {
                            if post.name == cursor_id {
                                tracing::debug!(
                                    "Hit cursor {} for friend {}",
                                    cursor_id,
                                    friend.name
                                );
                                break;
                            }
                        }
                        all_posts.push(post);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch posts for friend {}: {}", friend.name, e);
                }
            }
        }

        Ok(all_posts)
    }
}

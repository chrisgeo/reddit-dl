use async_trait::async_trait;

use crate::api::endpoints;
use crate::api::RedditClient;
use crate::error::Result;
use crate::post::Post;
use super::Source;

pub struct SavedSource {
    username: String,
}

impl SavedSource {
    pub fn new(username: String) -> Self {
        Self { username }
    }
}

#[async_trait]
impl Source for SavedSource {
    fn source_type(&self) -> &str {
        "saved"
    }

    fn source_name(&self) -> &str {
        "saved"
    }

    async fn fetch_posts(
        &self,
        client: &RedditClient,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<Post>> {
        let per_page = 100u32;
        let max_posts = limit.unwrap_or(u32::MAX) as usize;
        let mut all_posts = Vec::new();
        let mut after: Option<String> = None;

        loop {
            let fetch_limit = per_page.min((max_posts - all_posts.len()) as u32);
            let (posts, next_after) =
                endpoints::get_saved(client, &self.username, after.as_deref(), fetch_limit).await?;

            if posts.is_empty() {
                break;
            }

            for post in posts {
                // Stop if we've reached the cursor (already-seen post)
                if let Some(cursor_id) = cursor {
                    if post.name == cursor_id {
                        tracing::debug!("Hit cursor {} for saved posts", cursor_id);
                        return Ok(all_posts);
                    }
                }
                all_posts.push(post);
                if all_posts.len() >= max_posts {
                    return Ok(all_posts);
                }
            }

            match next_after {
                Some(a) => after = Some(a),
                None => break,
            }
        }

        Ok(all_posts)
    }
}

use async_trait::async_trait;

use crate::api::endpoints;
use crate::api::RedditClient;
use crate::error::Result;
use crate::post::Post;
use super::Source;

pub struct SubredditSource {
    name: String,
}

impl SubredditSource {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait]
impl Source for SubredditSource {
    fn source_type(&self) -> &str {
        "subreddits"
    }

    fn source_name(&self) -> &str {
        &self.name
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
                endpoints::get_subreddit_posts(client, &self.name, after.as_deref(), fetch_limit)
                    .await?;

            if posts.is_empty() {
                break;
            }

            for post in posts {
                if let Some(cursor_id) = cursor {
                    if post.name == cursor_id {
                        tracing::debug!("Hit cursor {} for r/{}", cursor_id, self.name);
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

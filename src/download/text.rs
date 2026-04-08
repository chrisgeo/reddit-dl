use std::path::Path;

use crate::api::RedditClient;
use crate::api::endpoints::get_post_comments;
use crate::error::Result;
use crate::post::Post;

/// Save a self-post's text content as a Markdown file.
pub fn save_self_post(post: &Post, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        "# {title}\n\nBy /u/{author} in /r/{subreddit}\n\n{body}",
        title = post.title,
        author = post.author,
        subreddit = post.subreddit,
        body = post.selftext,
    );

    std::fs::write(path, content)?;
    Ok(())
}

/// Fetch comments for a post and write them as a pretty-printed JSON file.
pub async fn save_comments(
    client: &RedditClient,
    post: &Post,
    path: &Path,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let comments = get_post_comments(client, &post.subreddit, &post.id).await?;
    let json = serde_json::to_string_pretty(&comments)?;
    std::fs::write(path, json)?;
    Ok(())
}

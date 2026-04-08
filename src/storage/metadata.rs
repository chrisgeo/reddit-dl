use serde::Serialize;
use std::path::Path;

/// Metadata for a downloaded post, written as a JSON sidecar
#[derive(Debug, Serialize)]
pub struct PostMetadata {
    pub id: String,
    pub title: String,
    pub author: String,
    pub subreddit: String,
    pub permalink: String,
    pub url: String,
    pub created_utc: f64,
    pub score: i64,
    pub num_comments: i64,
    pub is_self: bool,
    pub is_video: bool,
    pub selftext: Option<String>,
}

pub fn write_metadata(path: &Path, metadata: &PostMetadata) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(metadata)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

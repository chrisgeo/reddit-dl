use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A Reddit post with all fields needed for downloading and archiving
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Post {
    pub id: String,
    /// Fullname like "t3_abc123"
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub subreddit: String,
    pub permalink: String,
    #[serde(default)]
    pub url: Option<String>,
    pub created_utc: f64,
    #[serde(default)]
    pub selftext: String,
    #[serde(default)]
    pub is_self: bool,
    #[serde(default)]
    pub is_video: bool,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub num_comments: i64,
    #[serde(default)]
    pub is_gallery: Option<bool>,
    #[serde(default)]
    pub gallery_data: Option<GalleryData>,
    #[serde(default)]
    pub media_metadata: Option<HashMap<String, MediaMetadataItem>>,
    #[serde(default)]
    pub media: Option<Media>,
    #[serde(default)]
    pub preview: Option<Preview>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GalleryData {
    pub items: Vec<GalleryItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GalleryItem {
    pub media_id: String,
    pub id: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaMetadataItem {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub e: String, // "Image", "AnimatedImage"
    #[serde(default)]
    pub m: String, // MIME type like "image/jpg"
    #[serde(default)]
    pub s: Option<MediaSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaSource {
    #[serde(default)]
    pub u: Option<String>, // URL
    #[serde(default)]
    pub gif: Option<String>,
    #[serde(default)]
    pub mp4: Option<String>,
    #[serde(default)]
    pub x: Option<i32>, // width
    #[serde(default)]
    pub y: Option<i32>, // height
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Media {
    pub reddit_video: Option<RedditVideo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedditVideo {
    pub fallback_url: String,
    #[serde(default)]
    pub dash_url: Option<String>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub duration: Option<i32>,
    #[serde(default)]
    pub is_gif: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Preview {
    #[serde(default)]
    pub images: Vec<PreviewImage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreviewImage {
    pub source: PreviewSource,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreviewSource {
    pub url: String,
    pub width: i32,
    pub height: i32,
}

impl Post {
    /// Convert to metadata for JSON sidecar
    pub fn to_metadata(&self) -> crate::storage::metadata::PostMetadata {
        crate::storage::metadata::PostMetadata {
            id: self.id.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            subreddit: self.subreddit.clone(),
            permalink: self.permalink.clone(),
            url: self.url.clone().unwrap_or_default(),
            created_utc: self.created_utc,
            score: self.score,
            num_comments: self.num_comments,
            is_self: self.is_self,
            is_video: self.is_video,
            selftext: if self.is_self {
                Some(self.selftext.clone())
            } else {
                None
            },
        }
    }
}

use crate::post::Post;

/// The kind of media a post contains, used to dispatch to the right downloader.
#[derive(Debug)]
pub enum MediaType {
    DirectImage { url: String, extension: String },
    RedditVideo { video_url: String, audio_url: Option<String> },
    RedditGallery,
    ImgurSingle { url: String },
    ImgurAlbum { url: String },
    SelfPost,
    ExternalLink { url: String },
    NoMedia,
}

/// Image file extensions we recognise as direct downloads.
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "mp4", "gifv"];

/// Inspect a post and decide what kind of download is needed.
pub fn resolve_media_type(post: &Post) -> MediaType {
    // 1. Self-post (text post)
    if post.is_self {
        return MediaType::SelfPost;
    }

    let url = match &post.url {
        Some(u) => u.clone(),
        None => return MediaType::NoMedia,
    };

    // Guard against malformed URLs (e.g. missing scheme) — don't panic, just
    // treat them as NoMedia so the rest of the pipeline is unaffected.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        tracing::debug!("Post {} has malformed URL (no http scheme): {}", post.id, url);
        return MediaType::NoMedia;
    }

    // Strip URL fragment (#...) for all subsequent matching; the cleaned URL is
    // used for comparisons but we preserve the original for the download so that
    // servers that need query params still receive them.
    let url_for_matching: &str = url.split('#').next().unwrap_or(&url);

    // 2. Reddit gallery
    if post.is_gallery.unwrap_or(false) {
        return MediaType::RedditGallery;
    }

    // 3. Reddit-hosted video
    if post.is_video {
        if let Some(media) = &post.media {
            if let Some(rv) = &media.reddit_video {
                // Audio track lives at the same base URL with DASH_audio.mp4
                let audio_url = derive_audio_url(&rv.fallback_url);
                return MediaType::RedditVideo {
                    video_url: rv.fallback_url.clone(),
                    audio_url,
                };
            }
        }
        // is_video but no media.reddit_video — fall through to URL checks
    }

    // 4a. preview.redd.it — image preview URLs, treat as direct images
    if url_for_matching.contains("preview.redd.it") {
        let ext = extension_from_url(url_for_matching).unwrap_or_else(|| "jpg".to_string());
        return MediaType::DirectImage { url, extension: ext };
    }

    // 4b. i.redd.it direct image/video
    if url_for_matching.contains("i.redd.it") {
        let ext = extension_from_url(url_for_matching).unwrap_or_else(|| "jpg".to_string());
        return MediaType::DirectImage { url, extension: ext };
    }

    // 5. Imgur album / gallery
    if url_for_matching.contains("imgur.com/a/") || url_for_matching.contains("imgur.com/gallery/") {
        return MediaType::ImgurAlbum { url };
    }

    // 6. Imgur single image (i.imgur.com or imgur.com/<hash>)
    if url_for_matching.contains("i.imgur.com") || is_imgur_single(url_for_matching) {
        return MediaType::ImgurSingle { url };
    }

    // 7. Any URL whose path ends with a known image/video extension
    // Use url_for_matching so query params / fragments don't confuse extension detection,
    // but pass the original url to DirectImage so query params are preserved for the download.
    if let Some(ext) = extension_from_url(url_for_matching) {
        if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return MediaType::DirectImage { url, extension: ext };
        }
    }

    // 8. Everything else is an external link we can't directly download
    MediaType::ExternalLink { url }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Extract the file extension from a URL path, lowercased, without the dot.
/// Strips query strings before looking at the path component.
pub fn extension_from_url(url: &str) -> Option<String> {
    // Strip query / fragment
    let path_part = url.split('?').next().unwrap_or(url);
    let path_part = path_part.split('#').next().unwrap_or(path_part);
    let last_segment = path_part.rsplit('/').next().unwrap_or(path_part);
    if let Some(dot_pos) = last_segment.rfind('.') {
        let ext = last_segment[dot_pos + 1..].to_lowercase();
        if !ext.is_empty() {
            return Some(ext);
        }
    }
    None
}

/// Attempt to derive the audio-only URL for a Reddit video.
/// Reddit stores DASH streams; the audio track is at DASH_audio.mp4 alongside
/// the video track (e.g. DASH_720.mp4 → DASH_audio.mp4).
fn derive_audio_url(video_url: &str) -> Option<String> {
    // Strip query params
    let base = video_url.split('?').next()?;
    // Find the last path segment and replace it with DASH_audio.mp4
    let slash_pos = base.rfind('/')?;
    let audio = format!("{}/DASH_audio.mp4", &base[..slash_pos]);
    Some(audio)
}

/// Return true if the URL looks like an imgur single-image page:
/// e.g. https://imgur.com/AbCdEfG  (no /a/ or /gallery/ prefix)
fn is_imgur_single(url: &str) -> bool {
    if !url.contains("imgur.com") {
        return false;
    }
    // Must not be an album or gallery
    if url.contains("/a/") || url.contains("/gallery/") {
        return false;
    }
    // The path should have exactly one segment after the domain
    // e.g. https://imgur.com/AbCdEfG or https://imgur.com/AbCdEfG.jpg
    let after_domain = url
        .find("imgur.com/")
        .map(|i| &url[i + "imgur.com/".len()..]);
    if let Some(path) = after_domain {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        return segments.len() == 1;
    }
    false
}

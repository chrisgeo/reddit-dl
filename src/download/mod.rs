pub mod media;
pub mod resolver;
pub mod text;

use crate::api::RedditClient;
use crate::config::DownloadConfig;
use crate::error::Result;
use crate::post::Post;
use crate::storage::filesystem::OutputManager;
use crate::storage::metadata::write_metadata;
use resolver::MediaType;
use std::path::Path;

/// Download all media associated with `post` and write optional sidecars.
///
/// Returns the number of media files saved (not counting sidecar files).
pub async fn download_post(
    client: &RedditClient,
    http_client: &reqwest::Client,
    post: &Post,
    source_type: &str,
    source_name: &str,
    output: &OutputManager,
    config: &DownloadConfig,
) -> Result<u32> {
    // 1. Ensure the source directory exists
    output.ensure_source_dir(source_type, source_name)?;

    // 2. Resolve media type
    let media_type = resolver::resolve_media_type(post);

    // 3. Download
    let file_count = dispatch(
        http_client,
        client,
        post,
        source_type,
        source_name,
        output,
        config,
        &media_type,
    )
    .await?;

    // 4. Write metadata JSON sidecar if requested
    if config.include_metadata {
        let meta_path = output.post_path(
            source_type,
            source_name,
            &post.id,
            Some(&post.title),
            "json",
        );
        let metadata = post.to_metadata();
        if let Err(e) = write_metadata(&meta_path, &metadata) {
            tracing::warn!("Failed to write metadata for post {}: {}", post.id, e);
        }
    }

    Ok(file_count)
}

// ── internal dispatcher ───────────────────────────────────────────────────────

async fn dispatch(
    http_client: &reqwest::Client,
    client: &RedditClient,
    post: &Post,
    source_type: &str,
    source_name: &str,
    output: &OutputManager,
    config: &DownloadConfig,
    media_type: &MediaType,
) -> Result<u32> {
    match media_type {
        // ── Self-post (text) ──────────────────────────────────────────────────
        MediaType::SelfPost => {
            let path = output.post_path(
                source_type,
                source_name,
                &post.id,
                Some(&post.title),
                "md",
            );
            match text::save_self_post(post, &path) {
                Ok(()) => tracing::debug!("Saved self-post {}", post.id),
                Err(e) => tracing::warn!("Failed to save self-post {}: {}", post.id, e),
            }

            let mut count = 1u32;

            if config.include_comments {
                let comments_path = output.post_path(
                    source_type,
                    source_name,
                    &post.id,
                    Some(&post.title),
                    "comments.json",
                );
                match text::save_comments(client, post, &comments_path).await {
                    Ok(()) => tracing::debug!("Saved comments for {}", post.id),
                    Err(e) => tracing::warn!("Failed to save comments for {}: {}", post.id, e),
                }
                count += 1;
            }

            Ok(count)
        }

        // ── Direct image / video ──────────────────────────────────────────────
        MediaType::DirectImage { url, extension } => {
            let path = output.post_path(
                source_type,
                source_name,
                &post.id,
                Some(&post.title),
                extension,
            );
            match media::download_direct(http_client, url, &path).await {
                Ok(()) => {
                    tracing::debug!("Downloaded direct media {} → {}", url, path.display());
                    Ok(1)
                }
                Err(e) => {
                    tracing::warn!("Failed to download direct media for post {}: {}", post.id, e);
                    Ok(0)
                }
            }
        }

        // ── Reddit-hosted video ───────────────────────────────────────────────
        MediaType::RedditVideo { video_url, audio_url } => {
            let path = output.post_path(
                source_type,
                source_name,
                &post.id,
                Some(&post.title),
                "mp4",
            );
            match media::download_reddit_video(
                http_client,
                video_url,
                audio_url.as_deref(),
                &path,
            )
            .await
            {
                Ok(()) => {
                    tracing::debug!("Downloaded Reddit video for post {}", post.id);
                    Ok(1)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to download Reddit video for post {}: {}",
                        post.id,
                        e
                    );
                    Ok(0)
                }
            }
        }

        // ── Reddit gallery ────────────────────────────────────────────────────
        MediaType::RedditGallery => {
            let gallery_dir = output.gallery_dir(source_type, source_name, &post.id);
            match media::download_gallery(http_client, post, &gallery_dir).await {
                Ok(n) => {
                    tracing::debug!(
                        "Downloaded {} gallery images for post {}",
                        n,
                        post.id
                    );
                    Ok(n)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to download gallery for post {}: {}",
                        post.id,
                        e
                    );
                    Ok(0)
                }
            }
        }

        // ── Imgur single ──────────────────────────────────────────────────────
        MediaType::ImgurSingle { url } => {
            // Resolve to a direct image URL: add .jpg if no extension present
            let (download_url, ext) =
                resolve_imgur_single_url(url);
            let path = output.post_path(
                source_type,
                source_name,
                &post.id,
                Some(&post.title),
                &ext,
            );
            match media::download_direct(http_client, &download_url, &path).await {
                Ok(()) => {
                    tracing::debug!("Downloaded imgur single for post {}", post.id);
                    Ok(1)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to download imgur single for post {}: {}",
                        post.id,
                        e
                    );
                    Ok(0)
                }
            }
        }

        // ── Imgur album ───────────────────────────────────────────────────────
        // We log a warning — full Imgur album support requires the Imgur API
        // which is out of scope for Phase 5.
        MediaType::ImgurAlbum { url } => {
            tracing::warn!(
                "Imgur album downloads are not yet supported (post {}, url: {})",
                post.id,
                url
            );
            Ok(0)
        }

        // ── External link ─────────────────────────────────────────────────────
        MediaType::ExternalLink { url } => {
            tracing::debug!(
                "Post {} is an external link ({}); skipping media download",
                post.id,
                url
            );
            Ok(0)
        }

        // ── No media ─────────────────────────────────────────────────────────
        MediaType::NoMedia => {
            tracing::debug!("Post {} has no media URL; skipping", post.id);
            Ok(0)
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// For an imgur single-image URL, return a (download_url, extension) pair.
/// Ensures the URL points to a direct image (i.imgur.com) and has an extension.
fn resolve_imgur_single_url(url: &str) -> (String, String) {
    // If it's already a direct image URL with extension, use as-is
    if let Some(ext) = resolver::extension_from_url(url) {
        if ["jpg", "jpeg", "png", "gif", "webp", "mp4", "gifv"]
            .contains(&ext.as_str())
        {
            // Convert .gifv to .mp4 (Imgur serves mp4 for .gifv)
            if ext == "gifv" {
                let mp4_url = url.replace(".gifv", ".mp4");
                return (mp4_url, "mp4".to_string());
            }
            return (url.to_string(), ext);
        }
    }

    // For imgur.com/HASH (no extension), try i.imgur.com/HASH.jpg
    let hash = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("unknown");
    let direct = format!("https://i.imgur.com/{}.jpg", hash);
    (direct, "jpg".to_string())
}

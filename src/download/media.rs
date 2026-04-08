use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use futures_util::StreamExt;

use crate::error::{Error, Result};
use crate::post::Post;

// ── Direct download ───────────────────────────────────────────────────────────

/// Download a single URL to `path` using streaming (no full-file buffering).
pub async fn download_direct(
    http_client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<()> {
    let response = http_client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Other(format!(
            "HTTP {} downloading {}",
            status, url
        )));
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut file = fs::File::create(path).await?;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    Ok(())
}

// ── Reddit video ──────────────────────────────────────────────────────────────

/// Download a Reddit video.
///
/// If `audio_url` is provided and ffmpeg is available, the video and audio
/// streams are merged into a single MP4.  Otherwise only the video stream is
/// saved and a warning is logged.
pub async fn download_reddit_video(
    http_client: &reqwest::Client,
    video_url: &str,
    audio_url: Option<&str>,
    path: &Path,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    // Build temp paths next to the final output so rename is cheap.
    let base = path.with_extension("");
    let tmp_video = base.with_extension("_video.mp4.tmp");
    let tmp_audio = base.with_extension("_audio.mp4.tmp");

    // Download video stream
    download_to_file(http_client, video_url, &tmp_video).await?;

    if let Some(aurl) = audio_url {
        // Try downloading the audio track; Reddit doesn't always have one.
        match download_to_file(http_client, aurl, &tmp_audio).await {
            Ok(()) if has_ffmpeg() => {
                // Merge with ffmpeg
                let status = tokio::process::Command::new("ffmpeg")
                    .args([
                        "-y",
                        "-i", &tmp_video.to_string_lossy(),
                        "-i", &tmp_audio.to_string_lossy(),
                        "-c", "copy",
                        &path.to_string_lossy(),
                    ])
                    .status()
                    .await
                    .map_err(|e| Error::Other(format!("ffmpeg exec failed: {}", e)))?;

                // Clean up temps regardless of ffmpeg result
                let _ = fs::remove_file(&tmp_video).await;
                let _ = fs::remove_file(&tmp_audio).await;

                if !status.success() {
                    return Err(Error::Other(format!(
                        "ffmpeg exited with status {} for {}",
                        status,
                        path.display()
                    )));
                }
                return Ok(());
            }
            Ok(()) => {
                // Audio downloaded but no ffmpeg — save video-only
                tracing::warn!(
                    "ffmpeg not found; saving video-only for {}",
                    path.display()
                );
                let _ = fs::remove_file(&tmp_audio).await;
            }
            Err(e) => {
                // Audio track unavailable (404 etc.) — save video-only
                tracing::warn!(
                    "Could not download audio track ({}); saving video-only for {}",
                    e,
                    path.display()
                );
            }
        }
    }

    // No audio or merge skipped: rename video temp to final path
    fs::rename(&tmp_video, path).await?;
    Ok(())
}

// ── Reddit gallery ────────────────────────────────────────────────────────────

/// Download every image in a Reddit gallery.
///
/// Files are written into `gallery_dir` as `1.jpg`, `2.png`, etc., preserving
/// the ordering from `gallery_data.items`.  Returns the number of files saved.
pub async fn download_gallery(
    http_client: &reqwest::Client,
    post: &Post,
    gallery_dir: &Path,
) -> Result<u32> {
    let gallery_data = match &post.gallery_data {
        Some(gd) => gd,
        None => {
            tracing::warn!("Post {} marked as gallery but has no gallery_data", post.id);
            return Ok(0);
        }
    };
    let media_metadata = match &post.media_metadata {
        Some(mm) => mm,
        None => {
            tracing::warn!(
                "Post {} marked as gallery but has no media_metadata",
                post.id
            );
            return Ok(0);
        }
    };

    fs::create_dir_all(gallery_dir).await?;

    let mut count = 0u32;
    for (index, item) in gallery_data.items.iter().enumerate() {
        let number = index + 1; // 1-based
        let media_id = &item.media_id;

        let meta = match media_metadata.get(media_id) {
            Some(m) => m,
            None => {
                tracing::warn!(
                    "Gallery item {} not found in media_metadata for post {}",
                    media_id,
                    post.id
                );
                continue;
            }
        };

        if meta.status != "valid" && !meta.status.is_empty() {
            tracing::warn!(
                "Gallery item {} has status '{}', skipping",
                media_id,
                meta.status
            );
            continue;
        }

        let source = match &meta.s {
            Some(s) => s,
            None => {
                tracing::warn!("Gallery item {} has no source URL", media_id);
                continue;
            }
        };

        // Prefer the mp4 for animated images, then the direct URL
        let raw_url = source.mp4.as_deref()
            .or(source.u.as_deref())
            .or(source.gif.as_deref());

        let raw_url = match raw_url {
            Some(u) => u,
            None => {
                tracing::warn!("Gallery item {} has no usable URL", media_id);
                continue;
            }
        };

        // Gallery URLs have HTML-encoded `&amp;` — decode them
        let url = raw_url.replace("&amp;", "&");

        // Derive extension from MIME type or URL
        let ext = ext_from_mime(&meta.m)
            .or_else(|| crate::download::resolver::extension_from_url(&url))
            .unwrap_or_else(|| "jpg".to_string());

        let file_path = gallery_dir.join(format!("{}.{}", number, ext));

        tracing::debug!("Downloading gallery item {}/{}: {}", number, gallery_data.items.len(), url);

        match download_direct(http_client, &url, &file_path).await {
            Ok(()) => count += 1,
            Err(e) => {
                tracing::warn!(
                    "Failed to download gallery item {} for post {}: {}",
                    number,
                    post.id,
                    e
                );
            }
        }
    }

    Ok(count)
}

// ── ffmpeg probe ──────────────────────────────────────────────────────────────

/// Return `true` if `ffmpeg` is available on `$PATH`.
pub fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Stream-download `url` to `path`.
async fn download_to_file(
    http_client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<()> {
    let response = http_client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Other(format!("HTTP {} for {}", status, url)));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut file = fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

/// Map a MIME type string to a file extension.
fn ext_from_mime(mime: &str) -> Option<String> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("jpg".to_string()),
        "image/png" => Some("png".to_string()),
        "image/gif" => Some("gif".to_string()),
        "image/webp" => Some("webp".to_string()),
        "video/mp4" => Some("mp4".to_string()),
        _ => None,
    }
}

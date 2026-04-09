use futures_util::StreamExt;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::post::Post;

/// Maximum file size we are willing to download (500 MiB).
const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

/// Number of extra attempts after the first failure for transient errors.
const DOWNLOAD_RETRIES: u32 = 2;

// ── Direct download ───────────────────────────────────────────────────────────

/// Download a single URL to `path` using streaming (no full-file buffering).
///
/// Retries up to `DOWNLOAD_RETRIES` additional times on transient network
/// errors or 5xx responses.  Returns an immediate error (no retry) on 403/404.
/// Deletes the output file and returns an error if the downloaded file is empty
/// or exceeds `MAX_FILE_SIZE`.
pub async fn download_direct(http_client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let total_attempts = DOWNLOAD_RETRIES + 1;
    let mut last_err: Option<Error> = None;

    for attempt in 0..total_attempts {
        if attempt > 0 {
            let delay = Duration::from_secs(attempt as u64); // 1s, 2s
            tracing::debug!(
                "Retrying download (attempt {}/{}) for {} in {:?}",
                attempt + 1,
                total_attempts,
                url,
                delay
            );
            tokio::time::sleep(delay).await;
        }

        match attempt_download_direct(http_client, url, path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                // Non-retryable errors: return immediately
                if is_permanent_download_error(&e) {
                    return Err(e);
                }
                tracing::warn!(
                    "Transient error on download attempt {}/{} for {}: {}",
                    attempt + 1,
                    total_attempts,
                    url,
                    e
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        Error::Other(format!(
            "All {} download attempts failed for {}",
            total_attempts, url
        ))
    }))
}

/// Single attempt at downloading a URL.  Returns a permanent error for 403/404
/// and a transient error for network issues / 5xx.
async fn attempt_download_direct(
    http_client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<()> {
    let response = match http_client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            // Classify network errors
            if e.is_timeout() || e.is_connect() {
                return Err(Error::Http(e));
            }
            return Err(Error::Http(e));
        }
    };

    let status = response.status();
    let status_u16 = status.as_u16();

    // Permanent failures — don't retry
    if status_u16 == 403 || status_u16 == 404 {
        return Err(Error::Other(format!(
            "HTTP {} (permanent) downloading {}",
            status, url
        )));
    }

    if !status.is_success() {
        return Err(Error::Other(format!("HTTP {} downloading {}", status, url)));
    }

    // Check Content-Length up front to avoid wasting time on huge files
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_FILE_SIZE {
            tracing::warn!(
                "Skipping {}: Content-Length {} exceeds limit of {} bytes",
                url,
                content_length,
                MAX_FILE_SIZE
            );
            return Err(Error::Other(format!(
                "File too large ({} bytes, limit {} bytes): {}",
                content_length, MAX_FILE_SIZE, url
            )));
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut file = fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    let mut bytes_written: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        bytes_written += bytes.len() as u64;

        // Guard against servers that lie about Content-Length
        if bytes_written > MAX_FILE_SIZE {
            drop(file);
            let _ = fs::remove_file(path).await;
            tracing::warn!(
                "Aborting download of {}: exceeded {} bytes during streaming",
                url,
                MAX_FILE_SIZE
            );
            return Err(Error::Other(format!(
                "File too large (exceeded {} bytes during download): {}",
                MAX_FILE_SIZE, url
            )));
        }

        file.write_all(&bytes).await?;
    }
    file.flush().await?;

    // Empty response body — clean up and report error
    if bytes_written == 0 {
        drop(file);
        let _ = fs::remove_file(path).await;
        return Err(Error::Other(format!(
            "Empty response body (0 bytes) for {}",
            url
        )));
    }

    Ok(())
}

/// Returns true if the error is permanent and should not be retried.
fn is_permanent_download_error(e: &Error) -> bool {
    match e {
        Error::Other(msg) => msg.contains("HTTP 403") || msg.contains("HTTP 404"),
        _ => false,
    }
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
                        "-i",
                        &tmp_video.to_string_lossy(),
                        "-i",
                        &tmp_audio.to_string_lossy(),
                        "-c",
                        "copy",
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
                tracing::warn!("ffmpeg not found; saving video-only for {}", path.display());
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
        let raw_url = source
            .mp4
            .as_deref()
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

        tracing::debug!(
            "Downloading gallery item {}/{}: {}",
            number,
            gallery_data.items.len(),
            url
        );

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

/// Stream-download `url` to `path`.  Retries up to `DOWNLOAD_RETRIES` times on
/// transient errors.  Returns an immediate error (no retry) on 403/404.
/// Also enforces the `MAX_FILE_SIZE` limit and errors on empty responses.
async fn download_to_file(http_client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let total_attempts = DOWNLOAD_RETRIES + 1;
    let mut last_err: Option<Error> = None;

    for attempt in 0..total_attempts {
        if attempt > 0 {
            let delay = Duration::from_secs(attempt as u64);
            tracing::debug!(
                "Retrying download_to_file (attempt {}/{}) for {} in {:?}",
                attempt + 1,
                total_attempts,
                url,
                delay
            );
            tokio::time::sleep(delay).await;
        }

        match attempt_download_to_file(http_client, url, path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if is_permanent_download_error(&e) {
                    return Err(e);
                }
                tracing::warn!(
                    "Transient error on download_to_file attempt {}/{} for {}: {}",
                    attempt + 1,
                    total_attempts,
                    url,
                    e
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        Error::Other(format!(
            "All {} download attempts failed for {}",
            total_attempts, url
        ))
    }))
}

/// Single attempt at streaming a URL to a file.
async fn attempt_download_to_file(
    http_client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<()> {
    let response = http_client.get(url).send().await.map_err(Error::Http)?;
    let status = response.status();
    let status_u16 = status.as_u16();

    if status_u16 == 403 || status_u16 == 404 {
        return Err(Error::Other(format!(
            "HTTP {} (permanent) for {}",
            status, url
        )));
    }

    if !status.is_success() {
        return Err(Error::Other(format!("HTTP {} for {}", status, url)));
    }

    // Reject oversized files before writing anything
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_FILE_SIZE {
            tracing::warn!(
                "Skipping {}: Content-Length {} exceeds limit of {} bytes",
                url,
                content_length,
                MAX_FILE_SIZE
            );
            return Err(Error::Other(format!(
                "File too large ({} bytes, limit {}): {}",
                content_length, MAX_FILE_SIZE, url
            )));
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut file = fs::File::create(path).await?;
    let mut stream = response.bytes_stream();
    let mut bytes_written: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        bytes_written += bytes.len() as u64;

        if bytes_written > MAX_FILE_SIZE {
            drop(file);
            let _ = fs::remove_file(path).await;
            return Err(Error::Other(format!(
                "File too large (exceeded {} bytes during download): {}",
                MAX_FILE_SIZE, url
            )));
        }

        file.write_all(&bytes).await?;
    }
    file.flush().await?;

    if bytes_written == 0 {
        drop(file);
        let _ = fs::remove_file(path).await;
        return Err(Error::Other(format!(
            "Empty response body (0 bytes) for {}",
            url
        )));
    }

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

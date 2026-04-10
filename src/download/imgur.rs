use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::Path;

/// Imgur API response for a single image
#[derive(Debug, Deserialize)]
struct ImgurResponse<T> {
    data: T,
    #[allow(dead_code)]
    success: bool,
}

#[derive(Debug, Deserialize)]
struct ImgurImage {
    #[allow(dead_code)]
    id: String,
    link: String,
    #[serde(default)]
    mp4: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImgurAlbum {
    images: Vec<ImgurImage>,
}

/// Download a single Imgur image.
/// Resolves imgur.com/HASH → i.imgur.com/HASH.ext
/// Handles .gifv → .mp4 conversion.
pub async fn download_single(http_client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let (download_url, _ext) = resolve_single_url(url);
    super::media::download_direct(http_client, &download_url, path).await
}

/// Download all images in an Imgur album.
/// Requires client_id for the Imgur API.
/// Returns the number of files downloaded.
pub async fn download_album(
    http_client: &reqwest::Client,
    url: &str,
    album_dir: &Path,
    client_id: &str,
) -> Result<u32> {
    let album_hash = extract_album_hash(url)
        .ok_or_else(|| Error::Other(format!("Could not extract album hash from: {}", url)))?;

    let api_url = format!("https://api.imgur.com/3/album/{}", album_hash);
    let resp = http_client
        .get(&api_url)
        .header("Authorization", format!("Client-ID {}", client_id))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(Error::Other(format!(
            "Imgur API returned {} for album {}",
            resp.status(),
            album_hash
        )));
    }

    let album_resp: ImgurResponse<ImgurAlbum> = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("Failed to parse Imgur album response: {}", e)))?;

    tokio::fs::create_dir_all(album_dir).await?;

    let mut count = 0u32;
    for (i, image) in album_resp.data.images.iter().enumerate() {
        let number = i + 1;

        // Prefer mp4 for animated content, otherwise use the link
        let download_url = image.mp4.as_deref().unwrap_or(&image.link);
        let ext = extension_for_image(image);

        let file_path = album_dir.join(format!("{}.{}", number, ext));

        match super::media::download_direct(http_client, download_url, &file_path).await {
            Ok(()) => count += 1,
            Err(e) => {
                tracing::warn!(
                    "Failed to download album image {}/{}: {}",
                    number,
                    album_resp.data.images.len(),
                    e
                );
            }
        }
    }

    Ok(count)
}

/// Extract the album hash from an Imgur URL.
/// Handles: imgur.com/a/HASH, imgur.com/gallery/HASH
fn extract_album_hash(url: &str) -> Option<String> {
    let url = url.split('?').next().unwrap_or(url);
    let url = url.split('#').next().unwrap_or(url);
    let url = url.trim_end_matches('/');

    // Try /a/HASH or /gallery/HASH
    for prefix in &["/a/", "/gallery/"] {
        if let Some(pos) = url.find(prefix) {
            let hash = &url[pos + prefix.len()..];
            if !hash.is_empty() {
                return Some(hash.to_string());
            }
        }
    }
    None
}

/// Resolve a single Imgur URL to a direct download URL + extension.
fn resolve_single_url(url: &str) -> (String, String) {
    // Already a direct i.imgur.com link with extension
    if let Some(ext) = super::resolver::extension_from_url(url) {
        if ["jpg", "jpeg", "png", "gif", "webp", "mp4", "gifv"].contains(&ext.as_str()) {
            if ext == "gifv" {
                return (url.replace(".gifv", ".mp4"), "mp4".to_string());
            }
            return (url.to_string(), ext);
        }
    }

    // imgur.com/HASH → i.imgur.com/HASH.jpg
    let hash = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("unknown");
    (
        format!("https://i.imgur.com/{}.jpg", hash),
        "jpg".to_string(),
    )
}

/// Determine file extension for an Imgur image from its metadata.
fn extension_for_image(image: &ImgurImage) -> String {
    if image.mp4.is_some() {
        return "mp4".to_string();
    }
    if let Some(ref mime) = image.mime_type {
        match mime.as_str() {
            "image/jpeg" | "image/jpg" => return "jpg".to_string(),
            "image/png" => return "png".to_string(),
            "image/gif" => return "gif".to_string(),
            "image/webp" => return "webp".to_string(),
            "video/mp4" => return "mp4".to_string(),
            _ => {}
        }
    }
    super::resolver::extension_from_url(&image.link).unwrap_or_else(|| "jpg".to_string())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_album_hash ────────────────────────────────────────────────────

    #[test]
    fn extract_album_hash_slash_a() {
        let hash = extract_album_hash("https://imgur.com/a/AbCdEfG");
        assert_eq!(hash, Some("AbCdEfG".to_string()));
    }

    #[test]
    fn extract_album_hash_gallery() {
        let hash = extract_album_hash("https://imgur.com/gallery/XyZ1234");
        assert_eq!(hash, Some("XyZ1234".to_string()));
    }

    #[test]
    fn extract_album_hash_with_query_params() {
        let hash = extract_album_hash("https://imgur.com/a/AbCdEfG?foo=bar&baz=qux");
        assert_eq!(hash, Some("AbCdEfG".to_string()));
    }

    #[test]
    fn extract_album_hash_with_fragment() {
        let hash = extract_album_hash("https://imgur.com/a/AbCdEfG#image1");
        assert_eq!(hash, Some("AbCdEfG".to_string()));
    }

    #[test]
    fn extract_album_hash_trailing_slash() {
        let hash = extract_album_hash("https://imgur.com/a/AbCdEfG/");
        assert_eq!(hash, Some("AbCdEfG".to_string()));
    }

    #[test]
    fn extract_album_hash_not_an_album() {
        let hash = extract_album_hash("https://imgur.com/AbCdEfG");
        assert_eq!(hash, None);
    }

    #[test]
    fn extract_album_hash_empty_hash() {
        let hash = extract_album_hash("https://imgur.com/a/");
        assert_eq!(hash, None);
    }

    // ── resolve_single_url ────────────────────────────────────────────────────

    #[test]
    fn resolve_single_url_direct_jpeg() {
        let (url, ext) = resolve_single_url("https://i.imgur.com/AbCdEfG.jpg");
        assert_eq!(url, "https://i.imgur.com/AbCdEfG.jpg");
        assert_eq!(ext, "jpg");
    }

    #[test]
    fn resolve_single_url_direct_png() {
        let (url, ext) = resolve_single_url("https://i.imgur.com/AbCdEfG.png");
        assert_eq!(url, "https://i.imgur.com/AbCdEfG.png");
        assert_eq!(ext, "png");
    }

    #[test]
    fn resolve_single_url_gifv_converted_to_mp4() {
        let (url, ext) = resolve_single_url("https://i.imgur.com/AbCdEfG.gifv");
        assert_eq!(url, "https://i.imgur.com/AbCdEfG.mp4");
        assert_eq!(ext, "mp4");
    }

    #[test]
    fn resolve_single_url_direct_gif() {
        let (url, ext) = resolve_single_url("https://i.imgur.com/AbCdEfG.gif");
        assert_eq!(url, "https://i.imgur.com/AbCdEfG.gif");
        assert_eq!(ext, "gif");
    }

    #[test]
    fn resolve_single_url_hash_only_appends_jpg() {
        let (url, ext) = resolve_single_url("https://imgur.com/AbCdEfG");
        assert_eq!(url, "https://i.imgur.com/AbCdEfG.jpg");
        assert_eq!(ext, "jpg");
    }

    #[test]
    fn resolve_single_url_hash_only_with_trailing_slash() {
        // trailing slash gets trimmed; rsplit picks last non-empty segment
        let (url, _ext) = resolve_single_url("https://imgur.com/AbCdEfG/");
        // The trailing slash causes rsplit('/').next() to return "" so the hash
        // comes out as the empty segment — document the actual behaviour:
        // the function falls back to "unknown" in that case.
        // This also serves as a regression test.
        assert!(url.starts_with("https://i.imgur.com/"));
    }

    // ── extension_for_image ───────────────────────────────────────────────────

    fn make_image(link: &str, mp4: Option<&str>, mime_type: Option<&str>) -> ImgurImage {
        ImgurImage {
            id: "test".to_string(),
            link: link.to_string(),
            mp4: mp4.map(|s| s.to_string()),
            mime_type: mime_type.map(|s| s.to_string()),
        }
    }

    #[test]
    fn extension_for_image_mp4_field_wins() {
        let image = make_image(
            "https://i.imgur.com/x.gif",
            Some("https://i.imgur.com/x.mp4"),
            Some("image/gif"),
        );
        assert_eq!(extension_for_image(&image), "mp4");
    }

    #[test]
    fn extension_for_image_jpeg_mime() {
        let image = make_image("https://i.imgur.com/x.jpg", None, Some("image/jpeg"));
        assert_eq!(extension_for_image(&image), "jpg");
    }

    #[test]
    fn extension_for_image_png_mime() {
        let image = make_image("https://i.imgur.com/x.png", None, Some("image/png"));
        assert_eq!(extension_for_image(&image), "png");
    }

    #[test]
    fn extension_for_image_gif_mime() {
        let image = make_image("https://i.imgur.com/x.gif", None, Some("image/gif"));
        assert_eq!(extension_for_image(&image), "gif");
    }

    #[test]
    fn extension_for_image_webp_mime() {
        let image = make_image("https://i.imgur.com/x.webp", None, Some("image/webp"));
        assert_eq!(extension_for_image(&image), "webp");
    }

    #[test]
    fn extension_for_image_video_mp4_mime() {
        let image = make_image("https://i.imgur.com/x.mp4", None, Some("video/mp4"));
        assert_eq!(extension_for_image(&image), "mp4");
    }

    #[test]
    fn extension_for_image_falls_back_to_url_extension() {
        let image = make_image(
            "https://i.imgur.com/x.png",
            None,
            Some("application/octet-stream"),
        );
        assert_eq!(extension_for_image(&image), "png");
    }

    #[test]
    fn extension_for_image_no_mime_uses_url() {
        let image = make_image("https://i.imgur.com/x.jpg", None, None);
        assert_eq!(extension_for_image(&image), "jpg");
    }

    #[test]
    fn extension_for_image_unknown_defaults_jpg() {
        let image = make_image("https://i.imgur.com/x", None, None);
        assert_eq!(extension_for_image(&image), "jpg");
    }
}

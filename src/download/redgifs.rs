use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{Error, Result};

/// Cached RedGifs auth token, shared across downloads in a session.
#[derive(Clone)]
pub struct RedGifsClient {
    http_client: reqwest::Client,
    token: Arc<RwLock<Option<TokenData>>>,
}

struct TokenData {
    access_token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct GifResponse {
    gif: GifData,
}

#[derive(Debug, Deserialize)]
struct GifData {
    #[allow(dead_code)]
    id: String,
    urls: GifUrls,
}

#[derive(Debug, Deserialize)]
struct GifUrls {
    #[serde(default)]
    hd: Option<String>,
    #[serde(default)]
    sd: Option<String>,
    #[serde(default)]
    gif: Option<String>,
}

impl RedGifsClient {
    pub fn new(http_client: reqwest::Client) -> Self {
        Self {
            http_client,
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a valid auth token, fetching a new one if needed.
    async fn get_token(&self) -> Result<String> {
        // Check if we have a valid cached token
        {
            let guard = self.token.read().await;
            if let Some(ref td) = *guard {
                if td.expires_at > chrono::Utc::now() + chrono::Duration::seconds(60) {
                    return Ok(td.access_token.clone());
                }
            }
        }

        // Fetch new token
        let resp = self
            .http_client
            .get("https://api.redgifs.com/v2/auth/temporary")
            .header("User-Agent", "reddit-dl/0.1.0")
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Auth(format!(
                "RedGifs auth failed with status {}",
                resp.status()
            )));
        }

        // Parse — RedGifs returns {"token": "..."}
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Auth(format!("RedGifs auth parse error: {}", e)))?;

        let access_token = body
            .get("token")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("access_token").and_then(|v| v.as_str()))
            .ok_or_else(|| Error::Auth("RedGifs auth response missing token field".to_string()))?
            .to_string();

        let td = TokenData {
            access_token: access_token.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(3600),
        };
        *self.token.write().await = Some(td);

        Ok(access_token)
    }

    /// Download a RedGifs video to the given path.
    /// Extracts the gif ID from the URL, fetches metadata, downloads HD video.
    pub async fn download(&self, url: &str, path: &Path) -> Result<()> {
        let gif_id = extract_gif_id(url)
            .ok_or_else(|| Error::Other(format!("Could not extract RedGifs ID from: {}", url)))?;

        let token = self.get_token().await?;

        let api_url = format!("https://api.redgifs.com/v2/gifs/{}", gif_id.to_lowercase());
        let resp = self
            .http_client
            .get(&api_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Referer", "https://www.redgifs.com/")
            .header("Origin", "https://www.redgifs.com")
            .header("User-Agent", "reddit-dl/0.1.0")
            .send()
            .await?;

        let status = resp.status();

        // On 401, clear cached token and retry once
        if status.as_u16() == 401 {
            *self.token.write().await = None;
            let token = self.get_token().await?;

            let resp = self
                .http_client
                .get(&api_url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Referer", "https://www.redgifs.com/")
                .header("Origin", "https://www.redgifs.com")
                .header("User-Agent", "reddit-dl/0.1.0")
                .send()
                .await?;

            if !resp.status().is_success() {
                return Err(Error::Other(format!(
                    "RedGifs API returned {} for gif {} (after token refresh)",
                    resp.status(),
                    gif_id
                )));
            }

            let gif_resp: GifResponse = resp
                .json()
                .await
                .map_err(|e| Error::Other(format!("Failed to parse RedGifs response: {}", e)))?;

            return self.download_best_quality(&gif_resp.gif.urls, path).await;
        }

        if !status.is_success() {
            return Err(Error::Other(format!(
                "RedGifs API returned {} for gif {}",
                status, gif_id
            )));
        }

        let gif_resp: GifResponse = resp
            .json()
            .await
            .map_err(|e| Error::Other(format!("Failed to parse RedGifs response: {}", e)))?;

        self.download_best_quality(&gif_resp.gif.urls, path).await
    }

    /// Download the best available quality: hd → sd → gif
    async fn download_best_quality(&self, urls: &GifUrls, path: &Path) -> Result<()> {
        let download_url = urls
            .hd
            .as_deref()
            .or(urls.sd.as_deref())
            .or(urls.gif.as_deref())
            .ok_or_else(|| Error::Other("RedGifs response has no download URLs".to_string()))?;

        super::media::download_direct(&self.http_client, download_url, path).await
    }
}

/// Extract the gif ID from a RedGifs URL.
/// Handles:
///   https://www.redgifs.com/watch/someid
///   https://redgifs.com/watch/someid
///   https://www.redgifs.com/watch/someid?query=1
///   https://v3.redgifs.com/watch/someid
///   https://thumbs2.redgifs.com/SomeId.mp4 (direct CDN)
///   https://i.redgifs.com/i/someid.gif
fn extract_gif_id(url: &str) -> Option<String> {
    let url = url.split('?').next().unwrap_or(url);
    let url = url.split('#').next().unwrap_or(url);
    let url = url.trim_end_matches('/');

    // /watch/HASH pattern
    if let Some(pos) = url.find("/watch/") {
        let id = &url[pos + "/watch/".len()..];
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }

    // /i/HASH.ext pattern (direct image)
    if let Some(pos) = url.find("/i/") {
        let rest = &url[pos + "/i/".len()..];
        let id = rest.split('.').next().unwrap_or(rest);
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }

    // CDN pattern: thumbsN.redgifs.com/HASH.mp4 or HASH-mobile.mp4
    if url.contains("redgifs.com/") {
        let last_segment = url.rsplit('/').next()?;
        let id = last_segment
            .split('.')
            .next()
            .unwrap_or(last_segment)
            .split('-')
            .next() // strip "-mobile" suffix
            .unwrap_or(last_segment);
        if !id.is_empty() && !id.contains("redgifs") {
            return Some(id.to_string());
        }
    }

    None
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_watch_url_www() {
        assert_eq!(
            extract_gif_id("https://www.redgifs.com/watch/someId"),
            Some("someId".to_string())
        );
    }

    #[test]
    fn extract_watch_url_no_www() {
        assert_eq!(
            extract_gif_id("https://redgifs.com/watch/someId"),
            Some("someId".to_string())
        );
    }

    #[test]
    fn extract_watch_url_with_query() {
        assert_eq!(
            extract_gif_id("https://www.redgifs.com/watch/someId?query=1"),
            Some("someId".to_string())
        );
    }

    #[test]
    fn extract_watch_url_versioned_subdomain() {
        assert_eq!(
            extract_gif_id("https://v3.redgifs.com/watch/someId"),
            Some("someId".to_string())
        );
    }

    #[test]
    fn extract_cdn_thumbs_url() {
        assert_eq!(
            extract_gif_id("https://thumbs2.redgifs.com/SomeId.mp4"),
            Some("SomeId".to_string())
        );
    }

    #[test]
    fn extract_direct_image_url() {
        assert_eq!(
            extract_gif_id("https://i.redgifs.com/i/someid.gif"),
            Some("someid".to_string())
        );
    }

    #[test]
    fn extract_invalid_url_returns_none() {
        assert_eq!(
            extract_gif_id("https://www.youtube.com/watch?v=abc123"),
            None
        );
    }

    #[test]
    fn extract_empty_string_returns_none() {
        assert_eq!(extract_gif_id(""), None);
    }

    #[test]
    fn extract_watch_url_strips_trailing_slash() {
        assert_eq!(
            extract_gif_id("https://www.redgifs.com/watch/someId/"),
            Some("someId".to_string())
        );
    }

    #[test]
    fn extract_cdn_mobile_url_strips_suffix() {
        assert_eq!(
            extract_gif_id("https://thumbs2.redgifs.com/SomeId-mobile.mp4"),
            Some("SomeId".to_string())
        );
    }
}

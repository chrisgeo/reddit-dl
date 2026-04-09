use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;

use super::auth::{authenticate, AuthToken};
use crate::config::AuthConfig;
use crate::error::{Error, Result};

pub struct RedditClient {
    client: reqwest::Client,
    token: RwLock<AuthToken>,
    auth_config: AuthConfig,
    last_request: Mutex<Option<Instant>>,
}

/// Returns true if this HTTP status code is a transient error that warrants a retry.
fn is_transient_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Returns true if this reqwest error is a transient network condition.
fn is_transient_network_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect()
}

impl RedditClient {
    pub async fn new(config: &AuthConfig) -> Result<Self> {
        let user_agent = format!("linux:reddit-dl:0.1.0 (by /u/{})", config.username);

        let client = reqwest::Client::builder()
            .user_agent(&user_agent)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(Error::Http)?;

        let token = authenticate(
            &config.client_id,
            &config.client_secret,
            &config.username,
            &config.password,
        )
        .await?;

        Ok(Self {
            client,
            token: RwLock::new(token),
            auth_config: config.clone(),
            last_request: Mutex::new(None),
        })
    }

    async fn refresh_token(&self) -> Result<()> {
        let token = authenticate(
            &self.auth_config.client_id,
            &self.auth_config.client_secret,
            &self.auth_config.username,
            &self.auth_config.password,
        )
        .await?;
        *self.token.write().await = token;
        Ok(())
    }

    /// Enforce rate limit: at most 60 req/min (1 per second minimum gap)
    async fn enforce_rate_limit(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(last_time) = *last {
            let elapsed = last_time.elapsed();
            let min_gap = Duration::from_millis(1000);
            if elapsed < min_gap {
                tokio::time::sleep(min_gap - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        const MAX_ATTEMPTS: u32 = 3;
        let url = format!("https://oauth.reddit.com{}", path);
        // Track whether we've already done a 401 token refresh this call so it
        // only happens once per request (not once per attempt).
        let mut token_refreshed = false;

        for attempt in 0..MAX_ATTEMPTS {
            self.enforce_rate_limit().await;

            // Check if token needs refresh before request
            {
                let token = self.token.read().await;
                if token.is_expired() {
                    drop(token);
                    self.refresh_token().await?;
                    token_refreshed = true;
                }
            }

            let access_token = self.token.read().await.access_token.clone();

            let request_start = Instant::now();
            let send_result = self
                .client
                .get(&url)
                .bearer_auth(&access_token)
                .query(params)
                .send()
                .await;

            // Handle network-level errors (timeout, connection refused, etc.)
            let response = match send_result {
                Ok(r) => r,
                Err(e) => {
                    if is_transient_network_error(&e) && attempt < MAX_ATTEMPTS - 1 {
                        let backoff = Duration::from_secs(1 << attempt);
                        tracing::warn!(
                            "Transient network error on attempt {} for {}: {}. Retrying in {:?}",
                            attempt + 1,
                            url,
                            e,
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(Error::Http(e));
                }
            };

            let status = response.status();
            let elapsed = request_start.elapsed();

            if status.is_success() {
                tracing::debug!(
                    "GET {} -> {} in {:.0}ms",
                    url,
                    status.as_u16(),
                    elapsed.as_secs_f64() * 1000.0
                );
                let body = response.json::<T>().await?;
                return Ok(body);
            }

            let status_u16 = status.as_u16();

            // 401: refresh the token exactly once, then retry
            if status_u16 == 401 && !token_refreshed {
                tracing::debug!("Got 401 for {}, refreshing token and retrying", url);
                self.refresh_token().await?;
                token_refreshed = true;
                continue;
            }

            // 429: respect Retry-After header, then retry
            if status_u16 == 429 {
                let retry_after = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60);

                if attempt < MAX_ATTEMPTS - 1 {
                    tracing::warn!(
                        "Rate limited on attempt {} for {}. Waiting {}s before retry",
                        attempt + 1,
                        url,
                        retry_after
                    );
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }

                return Err(Error::RateLimit {
                    retry_after: Some(retry_after),
                });
            }

            // 500/502/503/504: transient server errors, exponential backoff
            if is_transient_status(status_u16) && attempt < MAX_ATTEMPTS - 1 {
                let backoff = Duration::from_secs(1 << attempt);
                tracing::warn!(
                    "Transient server error {} on attempt {} for {}. Retrying in {:?}",
                    status,
                    attempt + 1,
                    url,
                    backoff
                );
                tokio::time::sleep(backoff).await;
                continue;
            }

            return Err(Error::Other(format!(
                "Unexpected HTTP status {} for {}",
                status, url
            )));
        }

        Err(Error::Other(format!(
            "All {} attempts failed for {}",
            MAX_ATTEMPTS, url
        )))
    }
}

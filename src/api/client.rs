use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;

use crate::config::AuthConfig;
use crate::error::{Error, Result};
use super::auth::{authenticate, AuthToken};

pub struct RedditClient {
    client: reqwest::Client,
    token: RwLock<AuthToken>,
    auth_config: AuthConfig,
    last_request: Mutex<Option<Instant>>,
}

impl RedditClient {
    pub async fn new(config: &AuthConfig) -> Result<Self> {
        let user_agent = format!(
            "linux:reddit-dl:0.1.0 (by /u/{})",
            config.username
        );

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

        for attempt in 0..MAX_ATTEMPTS {
            self.enforce_rate_limit().await;

            // Check if token needs refresh before request
            {
                let token = self.token.read().await;
                if token.is_expired() {
                    drop(token);
                    self.refresh_token().await?;
                }
            }

            let access_token = self.token.read().await.access_token.clone();

            let response = self
                .client
                .get(&url)
                .bearer_auth(&access_token)
                .query(params)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let body = response.json::<T>().await?;
                return Ok(body);
            }

            if status.as_u16() == 401 && attempt == 0 {
                // Refresh token once and retry
                tracing::debug!("Got 401, refreshing token and retrying");
                self.refresh_token().await?;
                continue;
            }

            if status.as_u16() == 429 {
                let retry_after = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60);

                if attempt < MAX_ATTEMPTS - 1 {
                    tracing::warn!("Rate limited. Waiting {}s before retry", retry_after);
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }

                return Err(Error::RateLimit {
                    retry_after: Some(retry_after),
                });
            }

            if status.is_server_error() && attempt < MAX_ATTEMPTS - 1 {
                // Exponential backoff: 1s, 2s, 4s
                let backoff = Duration::from_secs(1 << attempt);
                tracing::warn!(
                    "Server error {} on attempt {}. Retrying in {:?}",
                    status,
                    attempt + 1,
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

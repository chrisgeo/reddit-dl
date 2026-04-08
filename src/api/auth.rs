use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct AuthToken {
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
}

impl AuthToken {
    pub fn is_expired(&self) -> bool {
        // Consider expired 60 seconds early to avoid edge cases
        Utc::now() >= self.expires_at - chrono::Duration::seconds(60)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    error: Option<String>,
}

pub async fn authenticate(
    client_id: &str,
    client_secret: &str,
    username: &str,
    password: &str,
) -> Result<AuthToken> {
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "linux:reddit-dl:0.1.0 (by /u/{})",
            username
        ))
        .build()
        .map_err(|e| Error::Http(e))?;

    let response = client
        .post("https://www.reddit.com/api/v1/access_token")
        .basic_auth(client_id, Some(client_secret))
        .form(&[
            ("grant_type", "password"),
            ("username", username),
            ("password", password),
        ])
        .send()
        .await?;

    let status = response.status();
    let body: TokenResponse = response.json().await?;

    if let Some(err) = body.error {
        return Err(Error::Auth(format!("Reddit API error: {}", err)));
    }

    if !status.is_success() {
        return Err(Error::Auth(format!("HTTP {}", status)));
    }

    if body.access_token.is_empty() {
        return Err(Error::Auth("Empty access token received".to_string()));
    }

    let expires_at = Utc::now() + chrono::Duration::seconds(body.expires_in as i64);

    Ok(AuthToken {
        access_token: body.access_token,
        expires_at,
    })
}

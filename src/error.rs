use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Rate limited, retry after {retry_after:?}s")]
    RateLimit { retry_after: Option<u64> },
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Db(String),
    #[allow(dead_code)]
    #[error("Config error: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

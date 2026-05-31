use thiserror::Error;

#[derive(Error, Debug)]
pub enum RustyError {
    #[error("API error: {0}")]
    Api(String),

    #[error("API error {status_code}: {message}")]
    ApiStatus { status_code: u16, message: String },

    #[error("Auth error: {0}")]
    Auth(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Rate limited, retry after {retry_after:?}s")]
    RateLimit { retry_after: Option<u64> },

    #[error("Context window exceeded")]
    ContextWindowExceeded,

    #[error("Max tokens reached")]
    MaxTokensReached,

    #[error("Cancelled")]
    Cancelled,

    #[error("Config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl RustyError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            RustyError::RateLimit { .. }
                | RustyError::ApiStatus {
                    status_code: 529, ..
                }
        )
    }

    pub fn is_context_limit(&self) -> bool {
        matches!(
            self,
            RustyError::ContextWindowExceeded | RustyError::MaxTokensReached
        )
    }
}

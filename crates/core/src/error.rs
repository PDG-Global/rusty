// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

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
                    status_code: 429 | 500 | 502 | 503 | 504 | 529,
                    ..
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_retryable ─────────────────────────────────────────────────

    #[test]
    fn is_retryable_rate_limit() {
        assert!(RustyError::RateLimit { retry_after: None }.is_retryable());
        assert!(RustyError::RateLimit { retry_after: Some(30) }.is_retryable());
    }

    #[test]
    fn is_retryable_server_errors() {
        for code in [429, 500, 502, 503, 504, 529] {
            assert!(
                RustyError::ApiStatus { status_code: code, message: "err".into() }.is_retryable(),
                "status {code} should be retryable"
            );
        }
    }

    #[test]
    fn is_retryable_false_for_non_retryable() {
        assert!(!RustyError::Api("boom".into()).is_retryable());
        assert!(!RustyError::ApiStatus { status_code: 400, message: "bad".into() }.is_retryable());
        assert!(!RustyError::ApiStatus { status_code: 401, message: "unauth".into() }.is_retryable());
        assert!(!RustyError::Auth("no".into()).is_retryable());
        assert!(!RustyError::PermissionDenied("no".into()).is_retryable());
        assert!(!RustyError::Tool("fail".into()).is_retryable());
        assert!(!RustyError::Http("err".into()).is_retryable());
        assert!(!RustyError::ContextWindowExceeded.is_retryable());
        assert!(!RustyError::MaxTokensReached.is_retryable());
        assert!(!RustyError::Cancelled.is_retryable());
        assert!(!RustyError::Config("bad".into()).is_retryable());
        assert!(!RustyError::Other("x".into()).is_retryable());
    }

    // ── is_context_limit ─────────────────────────────────────────────

    #[test]
    fn is_context_limit_true_variants() {
        assert!(RustyError::ContextWindowExceeded.is_context_limit());
        assert!(RustyError::MaxTokensReached.is_context_limit());
    }

    #[test]
    fn is_context_limit_false_variants() {
        assert!(!RustyError::Api("x".into()).is_context_limit());
        assert!(!RustyError::RateLimit { retry_after: None }.is_context_limit());
        assert!(!RustyError::Tool("x".into()).is_context_limit());
        assert!(!RustyError::Cancelled.is_context_limit());
    }
}

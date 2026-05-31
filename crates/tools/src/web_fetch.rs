// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Returns the response body as text. Use for reading documentation, API responses, or web pages."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum characters to return (default: 10000)"
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let url = input["url"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'url' parameter".into()))?;

        let max_length = input["max_length"].as_u64().unwrap_or(10000) as usize;

        debug!("Fetching URL: {url}");

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to fetch URL: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!(
                "HTTP {status}: {url}"
            )));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to read response: {e}")))?;

        let truncated = if body.len() > max_length {
            format!(
                "{}...\n\n[Truncated: showing {} of {} chars]",
                &body[..max_length],
                max_length,
                body.len()
            )
        } else {
            body
        };

        Ok(ToolResult::success(format!(
            "URL: {url}\nStatus: {status}\n\n{truncated}"
        )))
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anthropic Messages API provider implementation.
//!
//! Implements the `LlmProvider` trait for Anthropic-compatible endpoints
//! (e.g. Kimi Coding API at `api.kimi.com/coding/v1`).
//!
//! Key differences from OpenAI Chat Completions:
//! - Endpoint: `/v1/messages` (not `/chat/completions`)
//! - Auth: `x-api-key` header (not `Authorization: Bearer`)
//! - System prompt: separate top-level `system` field (not a system message)
//! - Streaming: Anthropic SSE events (`message_start`, `content_block_delta`, etc.)

use async_trait::async_trait;
use futures::StreamExt;
use rusty_core::{RustyError, UsageInfo};
use std::pin::Pin;
use tracing::debug;

use crate::{LlmProvider, MessageRequest, MessageResponse, ProviderConfig, StreamEvent};

/// Anthropic Messages API provider.
pub struct AnthropicProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Result<Self, RustyError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", config.api_key))
                .map_err(|_| RustyError::Auth("Invalid API key".to_string()))?,
        );
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "x-api-key",
            reqwest::header::HeaderValue::from_str(&config.api_key)
                .map_err(|_| RustyError::Auth("Invalid API key".to_string()))?,
        );
        headers.insert(
            "anthropic-version",
            reqwest::header::HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );

        // Apply extra headers from provider config.
        if let Some(extra) = &config.extra_headers {
            for (name, value) in extra {
                let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| RustyError::Other(format!("Invalid header name: {name}")))?;
                let header_value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| RustyError::Other(format!("Invalid header value for {name}")))?;
                headers.insert(header_name, header_value);
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .map_err(|e| RustyError::Other(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self { client, config })
    }

    fn endpoint(&self) -> String {
        let base = self.config.api_base.trim_end_matches('/');
        format!("{base}/messages")
    }

    async fn send_with_retry<F, Fut, T>(&self, f: F) -> Result<T, RustyError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, RustyError>>,
    {
        let max_retries = 5;
        let mut delay = std::time::Duration::from_secs(3);

        for attempt in 0..max_retries {
            match f().await {
                Ok(val) => return Ok(val),
                Err(e) if e.is_retryable() && attempt < max_retries - 1 => {
                    if let RustyError::RateLimit {
                        retry_after: Some(secs),
                    } = &e
                    {
                        delay = std::time::Duration::from_secs(*secs.max(&1));
                    } else if matches!(e, RustyError::RateLimit { .. }) {
                        delay = std::time::Duration::from_secs(10);
                    }
                    tracing::warn!(
                        "Retryable error (attempt {}): {e}. Retrying in {delay:?}",
                        attempt + 1
                    );
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2).min(std::time::Duration::from_secs(120));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

// ── Wire format types (Anthropic Messages API) ────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

/// SSE event from the Anthropic Messages streaming API.
#[allow(dead_code)] // Fields needed for serde deserialization
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicStreamEvent {
    MessageStart {
        message: AnthropicStreamMessage,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: ContentDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: MessageDelta,
        usage: Option<AnthropicUsage>,
    },
    MessageStop,
    Ping,
    Error {
        error: ErrorDetail,
    },
}

#[allow(dead_code)] // `content` field needed for serde deserialization
#[derive(Debug, serde::Deserialize)]
struct AnthropicStreamMessage {
    #[serde(default)]
    content: Vec<serde_json::Value>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[allow(dead_code)] // `input` field needed for serde deserialization
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Thinking {
        thinking: String,
    },
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    ThinkingDelta {
        thinking: String,
    },
}

#[derive(Debug, serde::Deserialize)]
struct MessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

#[derive(Debug, serde::Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

// ── Message/Tool conversion helpers ────────────────────────────────────────

/// Convert Rusty messages to Anthropic format.
///
/// In the Anthropic API, tool results are sent as user messages with
/// `tool_result` content blocks. Assistant messages with tool use are
/// sent as assistant messages with `tool_use` content blocks.
fn rusty_messages_to_anthropic(messages: &[rusty_core::Message]) -> Vec<AnthropicMessage> {
    use rusty_core::types::ContentBlock as CB;
    use rusty_core::types::MessageContent;
    use rusty_core::Role;

    let mut result = Vec::new();

    for msg in messages {
        match &msg.role {
            Role::User => match &msg.content {
                MessageContent::Text(text) => {
                    result.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: serde_json::Value::String(text.clone()),
                    });
                }
                MessageContent::Blocks(blocks) => {
                    let has_tool_results = blocks
                        .iter()
                        .any(|b| matches!(b, CB::ToolResult { .. }));

                    if has_tool_results {
                        // Tool results go as a user message with tool_result blocks
                        let content: Vec<serde_json::Value> = blocks
                            .iter()
                            .filter_map(|b| {
                                if let CB::ToolResult {
                                    tool_use_id,
                                    content,
                                    is_error,
                                } = b
                                {
                                    let mut obj = serde_json::json!({
                                        "type": "tool_result",
                                        "tool_use_id": tool_use_id,
                                        "content": content,
                                    });
                                    if let Some(err) = is_error {
                                        obj["is_error"] = serde_json::json!(err);
                                    }
                                    Some(obj)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if !content.is_empty() {
                            result.push(AnthropicMessage {
                                role: "user".to_string(),
                                content: serde_json::Value::Array(content),
                            });
                        }
                    } else {
                        // Regular text blocks
                        let content: Vec<serde_json::Value> = blocks
                            .iter()
                            .map(|b| match b {
                                CB::Text { text } => {
                                    serde_json::json!({"type": "text", "text": text})
                                }
                                CB::Thinking { thinking } => {
                                    serde_json::json!({"type": "thinking", "thinking": thinking})
                                }
                                _ => serde_json::json!({"type": "text", "text": ""}),
                            })
                            .collect();
                        result.push(AnthropicMessage {
                            role: "user".to_string(),
                            content: serde_json::Value::Array(content),
                        });
                    }
                }
            },
            Role::Assistant => match &msg.content {
                MessageContent::Text(text) => {
                    result.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: serde_json::Value::String(text.clone()),
                    });
                }
                MessageContent::Blocks(blocks) => {
                    let content: Vec<serde_json::Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            CB::Text { text } => {
                                Some(serde_json::json!({"type": "text", "text": text}))
                            }
                            CB::ToolUse { id, name, input } => Some(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input,
                            })),
                            CB::Thinking { thinking } => {
                                Some(serde_json::json!({"type": "thinking", "thinking": thinking}))
                            }
                            _ => None,
                        })
                        .collect();
                    if !content.is_empty() {
                        result.push(AnthropicMessage {
                            role: "assistant".to_string(),
                            content: serde_json::Value::Array(content),
                        });
                    }
                }
            },
        }
    }

    result
}

/// Convert Rusty tool definitions to Anthropic format.
fn rusty_tools_to_anthropic(tools: &[rusty_core::ToolDefinition]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

// ── LlmProvider implementation ────────────────────────────────────────────

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse, RustyError> {
        // For non-streaming, we use the same endpoint but with stream=false.
        // However, the Anthropic API's non-streaming response format differs.
        // For simplicity, we'll use streaming internally and collect the result.
        let mut stream = self.create_message_stream(request).await?;

        use futures::StreamExt;
        let mut text = String::new();
        let mut tool_calls = Vec::<(usize, String, String, String)>::new(); // (index, id, name, json)
        let mut usage = UsageInfo::default();
        let mut stop_reason_str = "end_turn".to_string();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(t) => text.push_str(&t),
                StreamEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                } => {
                    while tool_calls.len() <= index {
                        tool_calls.push((tool_calls.len(), String::new(), String::new(), String::new()));
                    }
                    let tc = &mut tool_calls[index];
                    if let Some(id) = id {
                        tc.1 = id;
                    }
                    if let Some(name) = name {
                        tc.2 = name;
                    }
                    tc.3.push_str(&arguments_delta);
                }
                StreamEvent::Usage(u) => usage = u,
                StreamEvent::Done { stop_reason } => {
                    if let Some(sr) = stop_reason {
                        stop_reason_str = sr;
                    }
                }
                _ => {}
            }
        }

        let mut blocks = Vec::new();
        if !text.is_empty() {
            blocks.push(rusty_core::ContentBlock::Text { text });
        }
        for (_, id, name, json) in &tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
            blocks.push(rusty_core::ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input,
            });
        }

        Ok(MessageResponse {
            content: blocks,
            usage,
            stop_reason: Some(stop_reason_str),
        })
    }

    async fn create_message_stream(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError>
    {
        let anthropic_msgs = rusty_messages_to_anthropic(&request.messages);
        let tools = rusty_tools_to_anthropic(&request.tools);

        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": anthropic_msgs,
            "stream": true,
        });

        if let Some(sys) = &request.system {
            body["system"] = serde_json::Value::String(sys.clone());
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(&tools)
                .map_err(|e| RustyError::Other(format!("Serialization error: {e}")))?;
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(budget) = request.thinking_budget {
            if budget > 0 {
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget,
                });
                // Thinking requires temperature = 1
                body["temperature"] = serde_json::json!(1.0);
            }
        }

        let endpoint = self.endpoint();
        let body_str = serde_json::to_string(&body)
            .map_err(|e| RustyError::Other(format!("Serialization error: {e}")))?;

        debug!("Sending Anthropic streaming request to {endpoint}");

        let resp = self
            .send_with_retry(|| async {
                let resp = self
                    .client
                    .post(&endpoint)
                    .body(body_str.clone())
                    .send()
                    .await
                    .map_err(|e| RustyError::Http(e.to_string()))?;

                let status = resp.status();
                if status.as_u16() == 429 {
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok());
                    return Err(RustyError::RateLimit { retry_after });
                }
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(status_to_error_from_parts(status.as_u16(), &text));
                }
                Ok(resp)
            })
            .await?;

        let stream = resp.bytes_stream();
        let events = futures::stream::unfold(
            (stream, String::new(), Vec::<ToolCallTracker>::new()),
            |(mut stream, mut line_buf, mut tool_calls)| async move {
                loop {
                    // Try to find a complete line in the buffer
                    if let Some(newline_pos) = line_buf.find('\n') {
                        let line = line_buf[..newline_pos].trim().to_string();
                        line_buf = line_buf[newline_pos + 1..].to_string();

                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<AnthropicStreamEvent>(data) {
                                Ok(event) => {
                                    if let Some(stream_event) =
                                        convert_anthropic_event(event, &mut tool_calls)
                                    {
                                        return Some((
                                            Ok(stream_event),
                                            (stream, line_buf, tool_calls),
                                        ));
                                    }
                                    // Events that don't produce output (ping, block stop, etc.)
                                    continue;
                                }
                                Err(e) => {
                                    debug!("Failed to parse Anthropic SSE: {e}: {data}");
                                    continue;
                                }
                            }
                        }
                    } else {
                        // Need more data
                        match stream.next().await {
                            Some(Ok(bytes)) => {
                                line_buf.push_str(&String::from_utf8_lossy(&bytes));
                            }
                            Some(Err(e)) => {
                                return Some((
                                    Err(RustyError::Http(e.to_string())),
                                    (stream, line_buf, tool_calls),
                                ));
                            }
                            None => {
                                if !line_buf.trim().is_empty() {
                                    debug!("Stream ended with remaining buffer: {line_buf}");
                                }
                                return None;
                            }
                        }
                    }
                }
            },
        );

        Ok(Box::pin(events))
    }
}

/// Internal tracker for accumulating tool call JSON across streaming events.
#[derive(Default)]
struct ToolCallTracker {
    json_buf: String,
}

/// Convert an Anthropic SSE event into a Rusty `StreamEvent`.
fn convert_anthropic_event(
    event: AnthropicStreamEvent,
    tool_calls: &mut Vec<ToolCallTracker>,
) -> Option<StreamEvent> {
    match event {
        AnthropicStreamEvent::MessageStart { message } => {
            // Emit usage if present (input tokens reported at start)
            if let Some(usage) = message.usage {
                return Some(StreamEvent::Usage(UsageInfo {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_tokens: usage.cache_read_input_tokens
                        + usage.cache_creation_input_tokens,
                }));
            }
            None
        }
        AnthropicStreamEvent::ContentBlockStart {
            index,
            content_block,
        } => {
            match content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    // Start tracking this tool call
                    while tool_calls.len() <= index {
                        tool_calls.push(ToolCallTracker::default());
                    }
                    tool_calls[index] = ToolCallTracker {
                        json_buf: String::new(),
                    };
                    Some(StreamEvent::ToolCallDelta {
                        index,
                        id: Some(id),
                        name: Some(name),
                        arguments_delta: String::new(),
                    })
                }
                ContentBlock::Text { text } => {
                    if text.is_empty() {
                        None
                    } else {
                        Some(StreamEvent::TextDelta(text))
                    }
                }
                ContentBlock::Thinking { thinking } => {
                    if thinking.is_empty() {
                        None
                    } else {
                        Some(StreamEvent::ThinkingDelta(thinking))
                    }
                }
            }
        }
        AnthropicStreamEvent::ContentBlockDelta { index, delta } => match delta {
            ContentDelta::TextDelta { text } => Some(StreamEvent::TextDelta(text)),
            ContentDelta::InputJsonDelta { partial_json } => {
                if tool_calls.len() > index {
                    tool_calls[index].json_buf.push_str(&partial_json);
                }
                Some(StreamEvent::ToolCallDelta {
                    index,
                    id: None,
                    name: None,
                    arguments_delta: partial_json,
                })
            }
            ContentDelta::ThinkingDelta { thinking } => Some(StreamEvent::ThinkingDelta(thinking)),
        },
        AnthropicStreamEvent::ContentBlockStop { .. } => None,
        AnthropicStreamEvent::MessageDelta { delta, usage } => {
            let stop_reason = delta.stop_reason;
            if let Some(usage) = usage {
                // Emit final usage update
                return Some(StreamEvent::Usage(UsageInfo {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_tokens: usage.cache_read_input_tokens
                        + usage.cache_creation_input_tokens,
                }));
            }
            Some(StreamEvent::Done { stop_reason })
        }
        AnthropicStreamEvent::MessageStop => Some(StreamEvent::Done { stop_reason: None }),
        AnthropicStreamEvent::Ping => None,
        AnthropicStreamEvent::Error { error } => Some(StreamEvent::Error(format!(
            "{}: {}",
            error.error_type, error.message
        ))),
    }
}

// ── Error helpers ─────────────────────────────────────────────────────────

fn status_to_error_from_parts(status: u16, body: &str) -> RustyError {
    match status {
        401 | 403 => RustyError::Auth(format!("HTTP {status}: {body}")),
        429 => RustyError::RateLimit { retry_after: None },
        529 => RustyError::RateLimit { retry_after: None },
        _ => RustyError::ApiStatus {
            status_code: status,
            message: body.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_messages() {
        let provider = make_provider("https://api.kimi.com/coding/v1");
        assert_eq!(
            provider.endpoint(),
            "https://api.kimi.com/coding/v1/messages"
        );
    }

    #[test]
    fn endpoint_strips_trailing_slash() {
        let provider = make_provider("https://api.kimi.com/coding/v1/");
        assert_eq!(
            provider.endpoint(),
            "https://api.kimi.com/coding/v1/messages"
        );
    }

    #[test]
    fn convert_tools_basic() {
        let tools = vec![rusty_core::ToolDefinition {
            name: "bash".to_string(),
            description: "Run a command".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"],
            }),
        }];
        let converted = rusty_tools_to_anthropic(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].name, "bash");
        assert_eq!(converted[0].description, "Run a command");
        assert_eq!(converted[0].input_schema["type"], "object");
    }

    #[test]
    fn convert_messages_user_text() {
        let msgs = vec![rusty_core::Message::user("Hello")];
        let converted = rusty_messages_to_anthropic(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
    }

    #[test]
    fn convert_messages_assistant_text() {
        let msgs = vec![rusty_core::Message::assistant("Hi there")];
        let converted = rusty_messages_to_anthropic(&msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
    }

    #[test]
    fn convert_messages_tool_use_and_result() {
        use rusty_core::ContentBlock;
        let msgs = vec![
            rusty_core::Message::assistant_blocks(vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
            }]),
            rusty_core::Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "file.txt".to_string(),
                is_error: None,
            }]),
        ];
        let converted = rusty_messages_to_anthropic(&msgs);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[1].role, "user");
    }

    #[test]
    fn parse_sse_text_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        assert!(matches!(result, Some(StreamEvent::TextDelta(ref s)) if s == "Hello"));
    }

    #[test]
    fn parse_sse_message_delta_with_stop() {
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        // Should emit Usage first
        assert!(matches!(result, Some(StreamEvent::Usage(_))));
    }

    #[test]
    fn parse_sse_tool_use() {
        let data = r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_1","name":"bash","input":{}}}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        assert!(matches!(
            result,
            Some(StreamEvent::ToolCallDelta {
                id: Some(ref id),
                name: Some(ref name),
                ..
            }) if id == "call_1" && name == "bash"
        ));
    }

    #[test]
    fn parse_sse_thinking_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        assert!(
            matches!(result, Some(StreamEvent::ThinkingDelta(ref s)) if s == "Let me think...")
        );
    }

    #[test]
    fn parse_sse_ping_returns_none() {
        let data = r#"{"type":"ping"}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        assert!(result.is_none());
    }

    #[test]
    fn parse_sse_error() {
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let event: AnthropicStreamEvent = serde_json::from_str(data).unwrap();
        let mut tool_calls = vec![];
        let result = convert_anthropic_event(event, &mut tool_calls);
        assert!(matches!(result, Some(StreamEvent::Error(ref s)) if s.contains("overloaded")));
    }

    #[test]
    fn status_401_is_auth_error() {
        let err = status_to_error_from_parts(401, "Unauthorized");
        assert!(matches!(err, RustyError::Auth(_)));
    }

    #[test]
    fn status_429_is_rate_limit() {
        let err = status_to_error_from_parts(429, "Too Many Requests");
        assert!(matches!(err, RustyError::RateLimit { .. }));
    }

    #[test]
    fn status_529_is_rate_limit() {
        let err = status_to_error_from_parts(529, "Overloaded");
        assert!(matches!(err, RustyError::RateLimit { .. }));
    }

    #[test]
    fn status_500_is_api_status() {
        let err = status_to_error_from_parts(500, "Internal Server Error");
        match err {
            RustyError::ApiStatus { status_code, message } => {
                assert_eq!(status_code, 500);
                assert_eq!(message, "Internal Server Error");
            }
            _ => panic!("expected ApiStatus"),
        }
    }

    fn make_provider(api_base: &str) -> AnthropicProvider {
        AnthropicProvider {
            client: reqwest::Client::new(),
            config: ProviderConfig {
                api_key: "test-key".to_string(),
                api_base: api_base.to_string(),
                model: "kimi-k2.6".to_string(),
                max_tokens: 4096,
                temperature: None,
                thinking_budget: None,
                extra_headers: None,
            },
        }
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use rusty_core::{RustyError, UsageInfo};
use std::pin::Pin;
use std::time::Duration;
use tracing::{debug, warn};

use crate::types::*;
use crate::{LlmProvider, MessageRequest, MessageResponse, ProviderConfig, StreamEvent};

pub struct OpenAiProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Result<Self, RustyError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", config.api_key))
                .map_err(|_| RustyError::Auth("Invalid API key".to_string()))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Apply extra headers from provider config (e.g. Kimi routing headers).
        if let Some(extra) = &config.extra_headers {
            for (name, value) in extra {
                let header_name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| RustyError::Other(format!("Invalid header name: {name}")))?;
                let header_value = HeaderValue::from_str(value)
                    .map_err(|_| RustyError::Other(format!("Invalid header value for {name}")))?;
                headers.insert(header_name, header_value);
            }
        }

        let client = reqwest::Client::builder()
            .user_agent(rusty_core::rusty_user_agent())
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| RustyError::Other(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self { client, config })
    }

    fn endpoint(&self) -> String {
        let base = self.config.api_base.trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    async fn send_with_retry<F, Fut, T>(&self, f: F) -> Result<T, RustyError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, RustyError>>,
    {
        let max_retries = 5;
        let mut delay = Duration::from_secs(3);

        for attempt in 0..max_retries {
            match f().await {
                Ok(val) => return Ok(val),
                Err(e) if e.is_retryable() && attempt < max_retries - 1 => {
                    if let RustyError::RateLimit {
                        retry_after: Some(secs),
                    } = &e
                    {
                        delay = Duration::from_secs(*secs.max(&1));
                    } else if matches!(e, RustyError::RateLimit { .. }) {
                        // No retry-after header — use longer backoff for rate limits
                        delay = Duration::from_secs(10);
                    }
                    warn!("Retryable error (attempt {}): {e}. Retrying in {delay:?}", attempt + 1);
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2).min(Duration::from_secs(120));
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai-compat"
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse, RustyError> {
        let mut oai_messages = rusty_messages_to_oai(&request.messages);
        if let Some(system) = &request.system {
            oai_messages.insert(
                0,
                OaiMessage {
                    role: "system".to_string(),
                    content: Some(OaiMessageContent::Text(system.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        let oai_tools = rusty_tools_to_oai(&request.tools);

        let oai_req = OaiRequest {
            model: request.model,
            messages: oai_messages,
            max_tokens: request.max_tokens,
            stream: false,
            temperature: request.temperature,
            tools: oai_tools,
            stream_options: None,
            reasoning_budget: request.thinking_budget,
        };

        let endpoint = self.endpoint();
        let body = serde_json::to_string(&oai_req)
            .map_err(|e| RustyError::Other(format!("Serialization error: {e}")))?;

        debug!("Sending request to {endpoint}");

        let resp = self
            .send_with_retry(|| async {
                self.client
                    .post(&endpoint)
                    .body(body.clone())
                    .send()
                    .await
                    .map_err(|e| RustyError::Http(e.to_string()))?
                    .error_for_status()
                    .map_err(status_to_error)?
                    .json::<OaiResponse>()
                    .await
                    .map_err(|e| RustyError::Http(e.to_string()))
            })
            .await?;

        let usage = UsageInfo {
            input_tokens: resp.usage.as_ref().map_or(0, |u| u.prompt_tokens),
            output_tokens: resp.usage.as_ref().map_or(0, |u| u.completion_tokens),
            cached_tokens: resp.usage.as_ref().map_or(0, |u| u.cached_tokens()),
        };

        let (content, stop_reason) = oai_response_to_rusty(&resp);

        Ok(MessageResponse {
            content,
            usage,
            stop_reason: Some(stop_reason),
        })
    }

    async fn create_message_stream(
        &self,
        request: MessageRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError>
    {
        let mut oai_messages = rusty_messages_to_oai(&request.messages);
        if let Some(system) = &request.system {
            oai_messages.insert(
                0,
                OaiMessage {
                    role: "system".to_string(),
                    content: Some(OaiMessageContent::Text(system.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        let oai_tools = rusty_tools_to_oai(&request.tools);

        let oai_req = OaiRequest {
            model: request.model,
            messages: oai_messages,
            max_tokens: request.max_tokens,
            stream: true,
            temperature: request.temperature,
            tools: oai_tools,
            stream_options: None,
            reasoning_budget: request.thinking_budget,
        };

        let endpoint = self.endpoint();
        let body = serde_json::to_string(&oai_req)
            .map_err(|e| RustyError::Other(format!("Serialization error: {e}")))?;

        debug!("Sending streaming request to {endpoint}");

        let resp = self
            .send_with_retry(|| async {
                let resp = self
                    .client
                    .post(&endpoint)
                    .body(body.clone())
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
            (
                stream,
                String::new(),
                OaiStreamState::default(),
                std::collections::VecDeque::<Result<StreamEvent, RustyError>>::new(),
            ),
            |(mut stream, mut line_buf, mut state, mut pending)| async move {
                loop {
                    if let Some(ev) = pending.pop_front() {
                        return Some((ev, (stream, line_buf, state, pending)));
                    }
                    if state.done {
                        return None;
                    }
                    if let Some(newline_pos) = line_buf.find('\n') {
                        let line = line_buf[..newline_pos].trim().to_string();
                        line_buf = line_buf[newline_pos + 1..].to_string();
                        pending.extend(state.process_line(&line));
                    } else {
                        match stream.next().await {
                            Some(Ok(bytes)) => {
                                line_buf.push_str(&String::from_utf8_lossy(&bytes));
                            }
                            Some(Err(e)) => {
                                return Some((
                                    Err(RustyError::Http(e.to_string())),
                                    (stream, line_buf, state, pending),
                                ));
                            }
                            None => {
                                if !line_buf.trim().is_empty() {
                                    debug!("Stream ended with remaining buffer: {line_buf}");
                                    let rest = std::mem::take(&mut line_buf);
                                    pending.extend(state.process_line(rest.trim()));
                                }
                                // End of stream: emit the single Done.
                                pending.extend(state.done_event());
                                if pending.is_empty() {
                                    return None;
                                }
                            }
                        }
                    }
                }
            },
        );

        Ok(Box::pin(events))
    }
}

/// Per-stream state for OpenAI SSE parsing.
#[derive(Default)]
struct OaiStreamState {
    tool_calls: Vec<(usize, OaiStreamToolCall)>,
    finish: Option<String>,
    done: bool,
}

fn map_finish_reason(finish: &str) -> &'static str {
    match finish {
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        _ => "end_turn",
    }
}

impl OaiStreamState {
    /// Emit the single terminal `Done` (once).
    fn done_event(&mut self) -> Option<Result<StreamEvent, RustyError>> {
        if self.done {
            return None;
        }
        self.done = true;
        Some(Ok(StreamEvent::Done {
            stop_reason: Some(
                self.finish
                    .take()
                    .unwrap_or_else(|| "end_turn".to_string()),
            ),
        }))
    }

    /// Process one trimmed SSE line, returning the events it produces.
    fn process_line(&mut self, line: &str) -> Vec<Result<StreamEvent, RustyError>> {
        if line.is_empty() || self.done {
            return Vec::new();
        }
        if line == "data: [DONE]" || line == "data:[DONE]" {
            return self.done_event().into_iter().collect();
        }
        if let Some(data) = line.strip_prefix("data:").map(|s| s.trim_start()) {
            match serde_json::from_str::<OaiStreamChunk>(data) {
                Ok(chunk) => return self.handle_chunk(chunk),
                Err(e) => debug!("Failed to parse SSE chunk: {e}: {data}"),
            }
        }
        Vec::new()
    }

    fn handle_chunk(&mut self, chunk: OaiStreamChunk) -> Vec<Result<StreamEvent, RustyError>> {
        let tool_calls = &mut self.tool_calls;
                                    let choice = chunk.choices.first();
                                    let mut events = Vec::new();

                                    // Text content
                                    if let Some(c) = choice {
                                        if let Some(content) = &c.delta.content {
                                            if !content.is_empty() {
                                                events.push(Ok(StreamEvent::TextDelta(
                                                    content.clone(),
                                                )));
                                            }
                                        }
                                    }

                                    // Thinking/reasoning content — emit as ThinkingDelta for UI,
                                    // and also as TextDelta so the agent accumulates it.
                                    // Some providers (Kimi) put all output in reasoning_content
                                    // with empty content, so TextDelta is needed to capture it.
                                    if let Some(c) = choice {
                                        if let Some(thinking) = &c.delta.reasoning_content {
                                            if !thinking.is_empty() {
                                                events.push(Ok(StreamEvent::ThinkingDelta(
                                                    thinking.clone(),
                                                )));
                                                // Only emit as text if no regular content
                                                // was present in this chunk
                                                if !events.iter().any(|e| matches!(e, Ok(StreamEvent::TextDelta(_)))) {
                                                    events.push(Ok(StreamEvent::TextDelta(
                                                        thinking.clone(),
                                                    )));
                                                }
                                            }
                                        }
                                    }

                                    // Tool calls
                                    if let Some(c) = choice {
                                    if let Some(tc_deltas) = &c.delta.tool_calls {
                                        for tc in tc_deltas {
                                            // Ensure we have enough slots
                                            while tool_calls.len() <= tc.index {
                                                tool_calls.push((
                                                    tool_calls.len(),
                                                    OaiStreamToolCall {
                                                        index: tool_calls.len(),
                                                        id: None,
                                                        function: None,
                                                    },
                                                ));
                                            }

                                            let existing = &mut tool_calls[tc.index].1;

                                            if let Some(id) = &tc.id {
                                                if existing.id.is_some() && existing.id.as_deref() != Some(id.as_str()) {
                                                    debug!(
                                                        "Tool call index {}: ID changed from {:?} to {:?}",
                                                        tc.index, existing.id, id
                                                    );
                                                }
                                                existing.id = Some(id.clone());
                                            }
                                            let mut args_delta = String::new();
                                            if let Some(func) = &tc.function {
                                                if existing.function.is_none() {
                                                    existing.function =
                                                        Some(OaiStreamFunction {
                                                            name: func.name.clone(),
                                                            arguments: Some(String::new()),
                                                        });
                                                }
                                                if let Some(name) = &func.name {
                                                    if let Some(ref mut f) = existing.function {
                                                        f.name = Some(name.clone());
                                                    }
                                                }
                                                if let Some(args) = &func.arguments {
                                                    args_delta = args.clone();
                                                    if let Some(ref mut f) = existing.function {
                                                        f.arguments = Some(
                                                            f.arguments
                                                                .clone()
                                                                .unwrap_or_default()
                                                                + args,
                                                        );
                                                    }
                                                }
                                            }

                                            // If no ID was provided (some providers omit it),
                                            // generate a synthetic one based on index.
                                            // Use existing.id (accumulated) rather than tc.id
                                            // (this delta) so we don't overwrite a real ID
                                            // from an earlier delta with a synthetic one.
                                            let effective_id = match &existing.id {
                                                Some(id) if !id.is_empty() => Some(id.clone()),
                                                _ => {
                                                    debug!(
                                                        "Tool call index {}: no ID from provider, using synthetic call_{}",
                                                        tc.index, tc.index
                                                    );
                                                    Some(format!("call_{}", tc.index))
                                                }
                                            };

                                            events.push(Ok(StreamEvent::ToolCallDelta {
                                                index: tc.index,
                                                id: effective_id,
                                                name: tc.function.as_ref().and_then(|f| {
                                                    f.name.clone()
                                                }),
                                                arguments_delta: args_delta,
                                            }));
                                        }
                                    }
                                    }

                                    // Finish reason: remember it; Done is emitted at
                                    // [DONE] / end of stream so a trailing usage chunk is read.
                                    if let Some(c) = choice {
                                        if let Some(finish) = &c.finish_reason {
                                            self.finish = Some(map_finish_reason(finish).to_string());
                                        }
                                    }

                                    // Usage data (sent on final chunk when stream_options is used)
                                    if let Some(usage) = &chunk.usage {
                                        events.push(Ok(StreamEvent::Usage(UsageInfo {
                                            input_tokens: usage.prompt_tokens,
                                            output_tokens: usage.completion_tokens,
                                            cached_tokens: usage.cached_tokens(),
                                        })));
                                    }

        events
    }
}

fn status_to_error(e: reqwest::Error) -> RustyError {
    let status = e.status().map(|s| s.as_u16()).unwrap_or(0);
    status_to_error_from_parts(status, &e.to_string())
}

fn status_to_error_from_parts(status: u16, body: &str) -> RustyError {
    match status {
        401 | 403 => RustyError::Auth(format!("HTTP {status}: {body}")),
        429 => {
            let retry_after = None; // Could parse Retry-After header if available
            RustyError::RateLimit { retry_after }
        }
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

    // ── status_to_error_from_parts ───────────────────────────────────

    #[test]
    fn status_401_is_auth_error() {
        let err = status_to_error_from_parts(401, "Unauthorized");
        assert!(matches!(err, RustyError::Auth(_)));
        assert!(err.to_string().contains("401"));
    }

    #[test]
    fn status_403_is_auth_error() {
        let err = status_to_error_from_parts(403, "Forbidden");
        assert!(matches!(err, RustyError::Auth(_)));
        assert!(err.to_string().contains("403"));
    }

    #[test]
    fn status_429_is_rate_limit() {
        let err = status_to_error_from_parts(429, "Too Many Requests");
        assert!(matches!(err, RustyError::RateLimit { retry_after: None }));
    }

    #[test]
    fn status_529_is_rate_limit() {
        let err = status_to_error_from_parts(529, "Overloaded");
        assert!(matches!(err, RustyError::RateLimit { retry_after: None }));
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

    #[test]
    fn status_0_is_api_status() {
        let err = status_to_error_from_parts(0, "unknown");
        assert!(matches!(err, RustyError::ApiStatus { status_code: 0, .. }));
    }

    #[test]
    fn body_preserved_in_error_message() {
        let err = status_to_error_from_parts(500, "detailed error body");
        assert!(err.to_string().contains("detailed error body"));
    }

    // ── endpoint ─────────────────────────────────────────────────────

    #[test]
    fn endpoint_appends_chat_completions() {
        let provider = make_provider("https://api.example.com/v1");
        assert_eq!(provider.endpoint(), "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn endpoint_strips_trailing_slash() {
        let provider = make_provider("https://api.example.com/v1/");
        assert_eq!(provider.endpoint(), "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn endpoint_strips_multiple_trailing_slashes() {
        let provider = make_provider("https://api.example.com/v1///");
        assert_eq!(provider.endpoint(), "https://api.example.com/v1/chat/completions");
    }

    // ── stream state ─────────────────────────────────────────────────

    fn run_lines(lines: &[&str]) -> Vec<StreamEvent> {
        let mut st = OaiStreamState::default();
        let mut out = Vec::new();
        for l in lines {
            out.extend(st.process_line(l).into_iter().map(|e| e.unwrap()));
        }
        out.extend(st.done_event().into_iter().map(|e| e.unwrap()));
        out
    }

    fn done_reasons(evs: &[StreamEvent]) -> Vec<Option<String>> {
        evs.iter()
            .filter_map(|e| match e {
                StreamEvent::Done { stop_reason } => Some(stop_reason.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn finish_then_usage_then_done_marker() {
        let evs = run_lines(&[
            r#"data: {"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":7}}"#,
            "data: [DONE]",
        ]);
        assert!(matches!(evs[0], StreamEvent::TextDelta(ref s) if s == "hi"));
        assert!(matches!(evs[1], StreamEvent::Usage(_)));
        assert_eq!(evs.len(), 3);
        assert_eq!(done_reasons(&evs), vec![Some("max_tokens".to_string())]);
    }

    #[test]
    fn end_of_stream_without_done_marker_emits_single_done() {
        let evs = run_lines(&[
            r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(done_reasons(&evs), vec![Some("tool_use".to_string())]);
    }

    #[test]
    fn no_finish_reason_defaults_to_end_turn() {
        let evs = run_lines(&["data: [DONE]"]);
        assert_eq!(done_reasons(&evs), vec![Some("end_turn".to_string())]);
    }

    #[test]
    fn done_marker_after_done_is_not_duplicated() {
        let evs = run_lines(&["data: [DONE]", "data: [DONE]"]);
        assert_eq!(done_reasons(&evs).len(), 1);
    }

    fn make_provider(api_base: &str) -> OpenAiProvider {
        OpenAiProvider {
            client: reqwest::Client::new(),
            config: ProviderConfig {
                api_key: "test-key".to_string(),
                api_base: api_base.to_string(),
                model: "test-model".to_string(),
                max_tokens: 4096,
                temperature: None,
                thinking_budget: None,
                extra_headers: None,
            },
        }
    }
}

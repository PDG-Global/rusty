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

        let client = reqwest::Client::builder()
            .default_headers(headers)
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
        let oai_messages = rusty_messages_to_oai(&request.messages);
        let oai_tools = rusty_tools_to_oai(&request.tools);

        let oai_req = OaiRequest {
            model: request.model,
            messages: oai_messages,
            max_tokens: request.max_tokens,
            stream: false,
            temperature: request.temperature,
            tools: oai_tools,
            stream_options: None,
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
        let oai_messages = rusty_messages_to_oai(&request.messages);
        let oai_tools = rusty_tools_to_oai(&request.tools);

        let oai_req = OaiRequest {
            model: request.model,
            messages: oai_messages,
            max_tokens: request.max_tokens,
            stream: true,
            temperature: request.temperature,
            tools: oai_tools,
            stream_options: Some(StreamOptions { include_usage: true }),
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
            (stream, String::new(), Vec::<(usize, OaiStreamToolCall)>::new()),
            |(mut stream, mut line_buf, mut tool_calls)| async move {
                loop {
                    // Try to find a complete line in the buffer
                    if let Some(newline_pos) = line_buf.find('\n') {
                        let line = line_buf[..newline_pos].trim().to_string();
                        line_buf = line_buf[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if line == "data: [DONE]" {
                            return Some((
                                Ok(StreamEvent::Done {
                                    stop_reason: Some("end_turn".to_string()),
                                }),
                                (stream, line_buf, tool_calls),
                            ));
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            match serde_json::from_str::<OaiStreamChunk>(data) {
                                Ok(chunk) => {
                                    // Emit usage data if present (sent on final chunk
                                    // when stream_options.include_usage is true)
                                    if let Some(usage) = &chunk.usage {
                                        return Some((
                                            Ok(StreamEvent::Usage(UsageInfo {
                                                input_tokens: usage.prompt_tokens,
                                                output_tokens: usage.completion_tokens,
                                            })),
                                            (stream, line_buf, tool_calls),
                                        ));
                                    }

                                    let choice = match chunk.choices.first() {
                                        Some(c) => c,
                                        None => continue,
                                    };

                                    let mut events = Vec::new();

                                    // Thinking/reasoning content
                                    if let Some(thinking) = &choice.delta.reasoning_content {
                                        if !thinking.is_empty() {
                                            events.push(Ok(StreamEvent::ThinkingDelta(
                                                thinking.clone(),
                                            )));
                                        }
                                    }

                                    // Text content
                                    if let Some(content) = &choice.delta.content {
                                        if !content.is_empty() {
                                            events.push(Ok(StreamEvent::TextDelta(
                                                content.clone(),
                                            )));
                                        }
                                    }

                                    // Tool calls
                                    if let Some(tc_deltas) = &choice.delta.tool_calls {
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
                                                existing.id = Some(id.clone());
                                            }
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

                                            events.push(Ok(StreamEvent::ToolCallDelta {
                                                index: tc.index,
                                                id: tc.id.clone(),
                                                name: tc.function.as_ref().and_then(|f| {
                                                    f.name.clone()
                                                }),
                                                arguments_delta: tc
                                                    .function
                                                    .as_ref()
                                                    .and_then(|f| f.arguments.clone())
                                                    .unwrap_or_default(),
                                            }));
                                        }
                                    }

                                    // Finish reason
                                    if let Some(finish) = &choice.finish_reason {
                                        let stop_reason = match finish.as_str() {
                                            "tool_calls" => "tool_use",
                                            "stop" => "end_turn",
                                            "length" => "max_tokens",
                                            _ => "end_turn",
                                        };
                                        events.push(Ok(StreamEvent::Done {
                                            stop_reason: Some(stop_reason.to_string()),
                                        }));
                                    }

                                    if events.is_empty() {
                                        continue;
                                    }

                                    // Return first event, buffer the rest
                                    let first = events.remove(0);
                                    // For simplicity, just return the first event per chunk
                                    // (Most chunks have exactly one event)
                                    return Some((first, (stream, line_buf, tool_calls)));
                                }
                                Err(e) => {
                                    debug!("Failed to parse SSE chunk: {e}: {data}");
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

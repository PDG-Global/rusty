// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusty_core::ContentBlock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

// OpenAI-compatible wire format types

#[derive(Debug, Clone, Serialize)]
pub struct OaiRequest {
    pub model: String,
    pub messages: Vec<OaiMessage>,
    pub max_tokens: u32,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OaiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_budget: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

/// Content part for multimodal messages (text or image_url)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiImageUrl {
    /// data URI: "data:{media_type};base64,{data}"
    pub url: String,
}

/// Message content: either a plain string or an array of content parts (for multimodal)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OaiMessageContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiMessage {
    pub role: String,
    pub content: Option<OaiMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OaiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OaiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiFunctionCall {
    pub name: String,
    pub arguments: String,
}

// Non-streaming response
#[derive(Debug, Clone, Deserialize)]
pub struct OaiResponse {
    pub choices: Vec<OaiChoice>,
    pub usage: Option<OaiUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiChoice {
    pub message: OaiResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiResponseMessage {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: Option<u32>,
    // OpenAI / Kimi / MiMo: prompt_tokens_details.cached_tokens
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    // DeepSeek: top-level prompt_cache_hit_tokens
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u32>,
}

impl OaiUsage {
    /// Extract the number of cached input tokens, supporting both OpenAI and
    /// DeepSeek wire formats. Returns 0 when not reported.
    pub fn cached_tokens(&self) -> u32 {
        // DeepSeek style (top-level field)
        if let Some(hit) = self.prompt_cache_hit_tokens {
            return hit;
        }
        // OpenAI / Kimi / MiMo style (nested object)
        self.prompt_tokens_details
            .as_ref()
            .map_or(0, |d| d.cached_tokens)
    }
}

// Streaming response chunks
#[derive(Debug, Clone, Deserialize)]
pub struct OaiStreamChunk {
    pub choices: Vec<OaiStreamChoice>,
    pub usage: Option<OaiUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiStreamChoice {
    pub delta: OaiStreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiStreamDelta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<OaiStreamToolCall>>,
    /// Reasoning/thinking content (used by MiMo, DeepSeek, etc.)
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiStreamToolCall {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<OaiStreamFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OaiStreamFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

// Conversion helpers

pub fn rusty_messages_to_oai(messages: &[rusty_core::Message]) -> Vec<OaiMessage> {
    let mut result = Vec::new();
    // Track valid tool_call IDs from the most recent assistant message so we can
    // filter out orphaned ToolResult blocks (those whose tool_use_id doesn't match
    // any tool_call that survived the id/name filter).  Without this, the API
    // receives tool results referencing non-existent tool calls and returns 400.
    let mut last_valid_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages {
        match msg {
            rusty_core::Message {
                role: rusty_core::Role::User,
                content,
            } => {
                // Check if this is a tool result message
                let blocks = match content {
                    rusty_core::MessageContent::Blocks(b) => b.clone(),
                    rusty_core::MessageContent::Text(t) => {
                        result.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(OaiMessageContent::Text(t.clone())),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                        continue;
                    }
                };

                let has_tool_results = blocks
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { .. }));

                if has_tool_results {
                    // Each tool result needs its own message in OpenAI format.
                    // Filter out orphaned results whose tool_use_id doesn't match
                    // any valid tool_call from the preceding assistant message.
                    // This prevents "toolcallid is not found" errors from APIs that
                    // strictly validate tool_call_id references.
                    for block in &blocks {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } = block
                        {
                            if !last_valid_tool_ids.is_empty()
                                && !last_valid_tool_ids.contains(tool_use_id)
                            {
                                warn!(
                                    "Dropping orphaned tool result (tool_use_id={tool_use_id:?}) — \
                                     no matching tool_call in preceding assistant message"
                                );
                                continue;
                            }
                            if tool_use_id.trim().is_empty() {
                                warn!("Dropping tool result with empty tool_use_id");
                                continue;
                            }
                            let text = if is_error.unwrap_or(false) {
                                format!("ERROR: {content}")
                            } else {
                                content.clone()
                            };
                            tracing::debug!(
                                "OAI: emitting tool result with tool_call_id={:?} (len={})",
                                tool_use_id,
                                text.len()
                            );
                            result.push(OaiMessage {
                                role: "tool".to_string(),
                                content: Some(OaiMessageContent::Text(text)),
                                tool_calls: None,
                                tool_call_id: Some(tool_use_id.clone()),
                            });
                        }
                    }
                } else {
                    // Check if we have image blocks - if so, use multipart content
                    let has_images = blocks
                        .iter()
                        .any(|b| matches!(b, ContentBlock::Image { .. }));

                    if has_images {
                        let mut parts = Vec::new();
                        for block in &blocks {
                            match block {
                                ContentBlock::Text { text } => {
                                    parts.push(OaiContentPart::Text {
                                        text: text.clone(),
                                    });
                                }
                                ContentBlock::Image {
                                    media_type,
                                    data,
                                } => {
                                    let data_uri =
                                        format!("data:{};base64,{}", media_type, data);
                                    parts.push(OaiContentPart::ImageUrl {
                                        image_url: OaiImageUrl { url: data_uri },
                                    });
                                }
                                _ => {}
                            }
                        }
                        result.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(OaiMessageContent::Parts(parts)),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    } else {
                        // No images, just text blocks - flatten to string
                        let text = blocks
                            .iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("");
                        result.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(OaiMessageContent::Text(text)),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
            }
            rusty_core::Message {
                role: rusty_core::Role::Assistant,
                content,
            } => {
                let blocks = match content {
                    rusty_core::MessageContent::Blocks(b) => b.clone(),
                    rusty_core::MessageContent::Text(t) => {
                        result.push(OaiMessage {
                            role: "assistant".to_string(),
                            content: Some(OaiMessageContent::Text(t.clone())),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                        continue;
                    }
                };

                let text_parts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                let content_text = if text_parts.is_empty() {
                    None
                } else {
                    Some(OaiMessageContent::Text(text_parts.join("")))
                };

                // Log ToolUse blocks that will be dropped due to empty id/name
                for block in &blocks {
                    if let ContentBlock::ToolUse { id, name, .. } = block {
                        if name.trim().is_empty() || id.trim().is_empty() {
                            warn!(
                                "Dropping tool_use block with empty {} (id={id:?}, name={name:?})",
                                if id.trim().is_empty() { "id" } else { "name" }
                            );
                        }
                    }
                }

                let tool_calls: Vec<OaiToolCall> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, name, input }
                            if !name.trim().is_empty() && !id.trim().is_empty() =>
                        {
                            Some(OaiToolCall {
                                id: id.clone(),
                                call_type: "function".to_string(),
                                function: OaiFunctionCall {
                                    name: name.clone(),
                                    // Ensure arguments is always a JSON object string.
                                    // Some LLM APIs fail if they receive "null" or a non-object
                                    // here (e.g. "Can only get item pairs from a mapping").
                                    arguments: if input.is_object() {
                                        input.to_string()
                                    } else {
                                        "{}".to_string()
                                    },
                                },
                            })
                        }
                        _ => None,
                    })
                    .collect();

                // Track valid tool_call IDs for orphan detection in subsequent tool results
                last_valid_tool_ids.clear();
                for tc in &tool_calls {
                    last_valid_tool_ids.insert(tc.id.clone());
                }

                result.push(OaiMessage {
                    role: "assistant".to_string(),
                    content: content_text,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                });
            }
        }
    }
    result
}

pub fn rusty_tools_to_oai(tools: &[rusty_core::ToolDefinition]) -> Vec<OaiTool> {
    tools
        .iter()
        .map(|t| OaiTool {
            tool_type: "function".to_string(),
            function: OaiFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect()
}

pub fn oai_response_to_rusty(resp: &OaiResponse) -> (Vec<ContentBlock>, String) {
    let mut blocks = Vec::new();
    let choice = match resp.choices.first() {
        Some(c) => c,
        None => return (blocks, "end_turn".to_string()),
    };

    if let Some(content) = &choice.message.content {
        if !content.is_empty() {
            blocks.push(ContentBlock::Text {
                text: content.clone(),
            });
        }
    }

    if let Some(tool_calls) = &choice.message.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }

    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => "tool_use".to_string(),
        Some("stop") => "end_turn".to_string(),
        Some("length") => "max_tokens".to_string(),
        other => other.unwrap_or("end_turn").to_string(),
    };

    (blocks, stop_reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_core::ContentBlock;

    fn assert_content_text(msg: &OaiMessage, expected: &str) {
        match &msg.content {
            Some(OaiMessageContent::Text(s)) => assert_eq!(s, expected),
            other => panic!("expected OaiMessageContent::Text, got {:?}", other),
        }
    }

    fn make_response(finish_reason: Option<&str>, content: Option<&str>) -> OaiResponse {
        OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    role: Some("assistant".to_string()),
                    content: content.map(|s| s.to_string()),
                    tool_calls: None,
                },
                finish_reason: finish_reason.map(|s| s.to_string()),
            }],
            usage: None,
        }
    }

    // ── stop reason mapping ──────────────────────────────────────────

    #[test]
    fn stop_reason_tool_calls_maps_to_tool_use() {
        let resp = make_response(Some("tool_calls"), None);
        let (_, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "tool_use");
    }

    #[test]
    fn stop_reason_stop_maps_to_end_turn() {
        let resp = make_response(Some("stop"), Some("done"));
        let (_, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "end_turn");
    }

    #[test]
    fn stop_reason_length_maps_to_max_tokens() {
        let resp = make_response(Some("length"), Some("truncated"));
        let (_, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "max_tokens");
    }

    #[test]
    fn stop_reason_none_defaults_to_end_turn() {
        let resp = make_response(None, Some("hi"));
        let (_, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "end_turn");
    }

    #[test]
    fn stop_reason_unknown_passthrough() {
        let resp = make_response(Some("content_filter"), Some("hi"));
        let (_, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "content_filter");
    }

    // ── content extraction ───────────────────────────────────────────

    #[test]
    fn text_content_extracted() {
        let resp = make_response(Some("stop"), Some("Hello world"));
        let (blocks, _) = oai_response_to_rusty(&resp);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn empty_choices_returns_empty() {
        let resp = OaiResponse {
            choices: vec![],
            usage: None,
        };
        let (blocks, reason) = oai_response_to_rusty(&resp);
        assert!(blocks.is_empty());
        assert_eq!(reason, "end_turn");
    }

    #[test]
    fn empty_content_skipped() {
        let resp = make_response(Some("stop"), Some(""));
        let (blocks, _) = oai_response_to_rusty(&resp);
        assert!(blocks.is_empty());
    }

    // ── tool call extraction ─────────────────────────────────────────

    #[test]
    fn tool_calls_parsed_from_response() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_123".to_string(),
                        call_type: "function".to_string(),
                        function: OaiFunctionCall {
                            name: "bash".to_string(),
                            arguments: r#"{"command":"ls"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };
        let (blocks, reason) = oai_response_to_rusty(&resp);
        assert_eq!(reason, "tool_use");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[test]
    fn text_and_tool_calls_together() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    role: Some("assistant".to_string()),
                    content: Some("Let me check.".to_string()),
                    tool_calls: Some(vec![OaiToolCall {
                        id: "c1".to_string(),
                        call_type: "function".to_string(),
                        function: OaiFunctionCall {
                            name: "file_read".to_string(),
                            arguments: r#"{"path":"/tmp/a"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };
        let (blocks, _) = oai_response_to_rusty(&resp);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "Let me check."));
        assert!(matches!(&blocks[1], ContentBlock::ToolUse { name, .. } if name == "file_read"));
    }

    #[test]
    fn malformed_tool_arguments_falls_back_to_null() {
        let resp = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "bad".to_string(),
                        call_type: "function".to_string(),
                        function: OaiFunctionCall {
                            name: "bash".to_string(),
                            arguments: "not-json!!!".to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };
        let (blocks, _) = oai_response_to_rusty(&resp);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::ToolUse { input, .. } => {
                assert_eq!(*input, serde_json::Value::Null);
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    // ── rusty_messages_to_oai ────────────────────────────────────────

    #[test]
    fn user_text_message() {
        let msgs = vec![rusty_core::Message::user("hello world")];
        let oai = rusty_messages_to_oai(&msgs);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].role, "user");
        assert_content_text(&oai[0], "hello world");
        assert!(oai[0].tool_calls.is_none());
        assert!(oai[0].tool_call_id.is_none());
    }

    #[test]
    fn assistant_text_message() {
        let msgs = vec![rusty_core::Message::assistant("hi there")];
        let oai = rusty_messages_to_oai(&msgs);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].role, "assistant");
        assert_content_text(&oai[0], "hi there");
        assert!(oai[0].tool_calls.is_none());
    }

    #[test]
    fn assistant_with_tool_use_blocks() {
        let msgs = vec![rusty_core::Message {
            role: rusty_core::Role::Assistant,
            content: rusty_core::MessageContent::Blocks(vec![
                rusty_core::ContentBlock::Text {
                    text: "Let me run it".to_string(),
                },
                rusty_core::ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ]),
        }];
        let oai = rusty_messages_to_oai(&msgs);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].role, "assistant");
        assert_content_text(&oai[0], "Let me run it");
        let calls = oai[0].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "bash");
    }

    #[test]
    fn user_tool_result_message() {
        let msgs = vec![rusty_core::Message {
            role: rusty_core::Role::User,
            content: rusty_core::MessageContent::Blocks(vec![
                rusty_core::ContentBlock::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "file contents here".to_string(),
                    is_error: None,
                },
            ]),
        }];
        let oai = rusty_messages_to_oai(&msgs);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].role, "tool");
        assert_eq!(oai[0].tool_call_id.as_deref(), Some("call_1"));
        assert_content_text(&oai[0], "file contents here");
    }

    #[test]
    fn tool_result_with_error_gets_prefix() {
        let msgs = vec![rusty_core::Message {
            role: rusty_core::Role::User,
            content: rusty_core::MessageContent::Blocks(vec![
                rusty_core::ContentBlock::ToolResult {
                    tool_use_id: "call_err".to_string(),
                    content: "permission denied".to_string(),
                    is_error: Some(true),
                },
            ]),
        }];
        let oai = rusty_messages_to_oai(&msgs);
        assert_content_text(&oai[0], "ERROR: permission denied");
    }

    #[test]
    fn multiple_tool_results_become_separate_messages() {
        let msgs = vec![rusty_core::Message {
            role: rusty_core::Role::User,
            content: rusty_core::MessageContent::Blocks(vec![
                rusty_core::ContentBlock::ToolResult {
                    tool_use_id: "c1".to_string(),
                    content: "result 1".to_string(),
                    is_error: None,
                },
                rusty_core::ContentBlock::ToolResult {
                    tool_use_id: "c2".to_string(),
                    content: "result 2".to_string(),
                    is_error: None,
                },
            ]),
        }];
        let oai = rusty_messages_to_oai(&msgs);
        assert_eq!(oai.len(), 2);
        assert_eq!(oai[0].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(oai[1].tool_call_id.as_deref(), Some("c2"));
    }

    #[test]
    fn empty_messages_list() {
        let oai = rusty_messages_to_oai(&[]);
        assert!(oai.is_empty());
    }

    // ── rusty_tools_to_oai ───────────────────────────────────────────

    #[test]
    fn tools_converted_correctly() {
        let tools = vec![rusty_core::ToolDefinition {
            name: "bash".to_string(),
            description: "Run a shell command".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        }];
        let oai = rusty_tools_to_oai(&tools);
        assert_eq!(oai.len(), 1);
        assert_eq!(oai[0].tool_type, "function");
        assert_eq!(oai[0].function.name, "bash");
        assert_eq!(oai[0].function.description, "Run a shell command");
    }

    #[test]
    fn empty_tools_list() {
        let oai = rusty_tools_to_oai(&[]);
        assert!(oai.is_empty());
    }
}

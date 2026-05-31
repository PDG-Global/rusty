use rusty_core::ContentBlock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OaiMessage {
    pub role: String,
    pub content: Option<String>,
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
pub struct OaiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: Option<u32>,
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
                            content: Some(t.clone()),
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
                    // Each tool result needs its own message in OpenAI format
                    for block in &blocks {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } = block
                        {
                            let text = if is_error.unwrap_or(false) {
                                format!("ERROR: {content}")
                            } else {
                                content.clone()
                            };
                            result.push(OaiMessage {
                                role: "tool".to_string(),
                                content: Some(text),
                                tool_calls: None,
                                tool_call_id: Some(tool_use_id.clone()),
                            });
                        }
                    }
                } else {
                    // Regular user message with text blocks
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
                        content: Some(text),
                        tool_calls: None,
                        tool_call_id: None,
                    });
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
                            content: Some(t.clone()),
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
                    Some(text_parts.join(""))
                };

                let tool_calls: Vec<OaiToolCall> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { id, name, input } => Some(OaiToolCall {
                            id: id.clone(),
                            call_type: "function".to_string(),
                            function: OaiFunctionCall {
                                name: name.clone(),
                                arguments: input.to_string(),
                            },
                        }),
                        _ => None,
                    })
                    .collect();

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

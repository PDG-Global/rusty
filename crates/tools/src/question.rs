// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

pub struct QuestionTool;

#[async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their answer. Use this instead of stopping \
         with a natural-language question. The tool blocks until the user responds. \
         Always prefer this over ending your turn with a question — it keeps your task \
         context intact and prevents the auto-continuation system from answering on the \
         user's behalf."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "header": {
                    "type": "string",
                    "description": "Short label (max 30 chars) shown in the UI"
                },
                "options": {
                    "type": "array",
                    "description": "Optional list of choices",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "Display text (1-5 words)"
                            },
                            "description": {
                                "type": "string",
                                "description": "Explanation of this choice"
                            }
                        },
                        "required": ["label", "description"],
                        "additionalProperties": false
                    }
                },
                "multiple": {
                    "type": "boolean",
                    "description": "Allow selecting multiple choices (default: false)"
                }
            },
            "required": ["question"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let question = input["question"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'question' parameter".into()))?;

        let header = input["header"].as_str().unwrap_or("");

        // Format the question with options for display
        let mut prompt = question.to_string();
        if let Some(options) = input["options"].as_array() {
            if !options.is_empty() {
                prompt.push_str("\n\n");
                for (i, opt) in options.iter().enumerate() {
                    let label = opt["label"].as_str().unwrap_or("?");
                    let desc = opt["description"].as_str().unwrap_or("");
                    if desc.is_empty() {
                        prompt.push_str(&format!("  {}. {}\n", i + 1, label));
                    } else {
                        prompt.push_str(&format!("  {}. {} — {}\n", i + 1, label, desc));
                    }
                }
            }
        }

        // Use the question callback if available, otherwise return an error
        let answer = if let Some(cb) = &ctx.on_question {
            let header_str = if header.is_empty() {
                // Truncate question to 30 chars for header
                let end = question.char_indices().nth(30).map(|(i, _)| i).unwrap_or(question.len());
                &question[..end]
            } else {
                header
            };
            cb(header_str, &prompt).await
        } else {
            // In headless/sub-agent mode with no callback — return a marker
            // so the model knows it can't get an answer this way
            return Err(RustyError::Tool(
                "No user available to answer questions. Make a reasonable decision and proceed.".into(),
            ));
        };

        Ok(ToolResult {
            content: answer,
            is_error: false,
        })
    }
}

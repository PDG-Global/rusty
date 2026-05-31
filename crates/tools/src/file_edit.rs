// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct FileEditTool;

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing exact text. The old_string must match exactly once in the file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to find and replace (must be unique in the file)"
                },
                "new_string": {
                    "type": "string",
                    "description": "Text to replace old_string with"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let path_str = input["path"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'path' parameter".into()))?;
        let old_string = input["old_string"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'old_string' parameter".into()))?;
        let new_string = input["new_string"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'new_string' parameter".into()))?;

        let path = crate::resolve_path(path_str, &ctx.working_dir)?;
        debug!("Editing file: {}", path.display());

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to read {}: {e}", path.display())))?;

        let count = content.matches(old_string).count();
        if count == 0 {
            return Ok(ToolResult::error(format!(
                "old_string not found in {}. Make sure the text matches exactly.",
                path.display()
            )));
        }
        if count > 1 {
            return Ok(ToolResult::error(format!(
                "old_string found {count} times in {}. It must be unique. Provide more context.",
                path.display()
            )));
        }

        let new_content = content.replacen(old_string, new_string, 1);

        tokio::fs::write(&path, &new_content)
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to write {}: {e}", path.display())))?;

        // Show a brief diff
        let diff = similar::TextDiff::from_lines(&content, &new_content);
        let summary: String = diff
            .iter_all_changes()
            .filter_map(|change| match change.tag() {
                similar::ChangeTag::Delete => Some(format!("- {}", change.value())),
                similar::ChangeTag::Insert => Some(format!("+ {}", change.value())),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(ToolResult::success(format!(
            "Edited {}\n{summary}",
            path.display()
        )))
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns the file content with line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative to working directory or absolute)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based, optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)"
                }
            },
            "required": ["path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let path_str = input["path"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'path' parameter".into()))?;

        let path = crate::resolve_path(path_str, &ctx.working_dir)?;
        debug!("Reading file: {}", path.display());

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to read {}: {e}", path.display())))?;

        let lines: Vec<&str> = content.lines().collect();
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(lines.len() as u64) as usize;

        let end = (offset + limit).min(lines.len());
        if offset >= lines.len() {
            return Ok(ToolResult::success("(empty)"));
        }

        let numbered: String = lines[offset..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4}\t{}", offset + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolResult::success(numbered))
    }
}

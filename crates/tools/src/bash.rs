// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use for running tests, building, git commands, and system operations. Note: commands run in the working directory but are NOT sandboxed — avoid commands that read/write outside the working directory."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'command' parameter".into()))?;

        let timeout_secs = input["timeout"].as_u64().unwrap_or(120);

        debug!("Executing bash command: {command}");

        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            output,
        )
        .await
        .map_err(|_| RustyError::Tool(format!("Command timed out after {timeout_secs}s")))?
        .map_err(|e| RustyError::Tool(format!("Failed to execute command: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&format!("STDERR:\n{stderr}"));
        }
        if result.is_empty() {
            result = format!("(exit code: {exit_code})");
        }

        if exit_code != 0 {
            result.push_str(&format!("\n(exit code: {exit_code})"));
        }

        Ok(ToolResult {
            content: result,
            is_error: !output.status.success(),
        })
    }
}

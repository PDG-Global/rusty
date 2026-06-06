// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{CancelToken, PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use crate::{Tool, ToolContext, ToolResult};

/// Function type for spawning sub-agents
pub type SubAgentFn = Arc<
    dyn Fn(String, PathBuf, Option<CancelToken>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, RustyError>> + Send>>
        + Send
        + Sync,
>;

/// Spawn a sub-agent to handle a complex subtask.
pub struct AgentTool {
    pub spawn_fn: SubAgentFn,
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a complex subtask. The sub-agent runs independently with its own context. Use for delegating research, multi-step tasks, or parallel exploration."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task description for the sub-agent"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context or constraints for the sub-agent"
                }
            },
            "required": ["task"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let task = input["task"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'task' parameter".into()))?;

        let context = input["context"].as_str().unwrap_or("");

        let full_task = if context.is_empty() {
            task.to_string()
        } else {
            format!("{task}\n\nAdditional context: {context}")
        };

        match (self.spawn_fn)(full_task, ctx.working_dir.clone(), ctx.cancel.clone()).await {
            Ok(result) => Ok(ToolResult::success(result)),
            Err(e) => Ok(ToolResult::error(format!("Sub-agent error: {e}"))),
        }
    }
}

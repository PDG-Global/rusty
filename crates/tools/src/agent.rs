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
    dyn Fn(String, String, PathBuf, Option<CancelToken>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, RustyError>> + Send>>
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
        "Spawn a sub-agent to handle a complex subtask. The sub-agent runs independently with its own context. Use for delegating research, multi-step tasks, or parallel exploration.\n\n\
         Available agent types (pass via subagent_type):\n\
         - explore: Read-only exploration. Tools: Read, Glob, Grep, WebFetch. Use for searches, code analysis, and fact-finding.\n\
         - coder: General coding. Tools: Read, Write, Edit, Bash, Glob, Grep, WebFetch, ApplyPatch. Use for implementation tasks.\n\n\
         Guidelines:\n\
         - Prefer 'explore' for research and investigation tasks.\n\
         - Prefer 'coder' for tasks that require file modifications.\n\
         - Provide a concise description (3-5 words) for UI display.\n\
         - Resume is not yet supported; always start a new subagent."
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
                },
                "subagent_type": {
                    "type": "string",
                    "enum": ["explore", "coder"],
                    "description": "Type of subagent to spawn. 'explore' is read-only (safer, cheaper). 'coder' can write files. Defaults to 'explore'.",
                    "default": "explore"
                },
                "description": {
                    "type": "string",
                    "description": "Short task description (3-5 words) for UI display"
                },
                "resume": {
                    "type": "string",
                    "description": "Optional agent ID to resume instead of creating a new instance. Not yet supported."
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
        let subagent_type = input["subagent_type"].as_str().unwrap_or("explore");
        let resume = input["resume"].as_str().unwrap_or("");

        // Validate subagent_type
        let subagent_type = match subagent_type {
            "coder" => "coder",
            "explore" | "" => "explore",
            _ => "explore",
        };

        // Resume is not yet supported
        if !resume.is_empty() {
            return Ok(ToolResult::error(
                "Resume is not yet supported. Start a new subagent instead.".to_string(),
            ));
        }

        let full_task = if context.is_empty() {
            task.to_string()
        } else {
            format!("{task}\n\nAdditional context: {context}")
        };

        let agent_id = format!("subagent-{}", uuid::Uuid::new_v4());

        match (self.spawn_fn)(
            full_task,
            subagent_type.to_string(),
            ctx.working_dir.clone(),
            ctx.cancel.clone(),
        )
        .await
        {
            Ok(result) => {
                let output = format!(
                    "agent_id: {agent_id}\n\
                     actual_subagent_type: {subagent_type}\n\
                     status: completed\n\n\
                     [summary]\n\
                     {result}"
                );
                Ok(ToolResult::success(output))
            }
            Err(e) => {
                let output = format!(
                    "agent_id: {agent_id}\n\
                     actual_subagent_type: {subagent_type}\n\
                     status: failed\n\n\
                     subagent error: {e}"
                );
                Ok(ToolResult::error(output))
            }
        }
    }
}

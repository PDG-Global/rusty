// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::background::BackgroundManager;
use crate::{Tool, ToolContext, ToolResult};

/// Query the status and result of a background subagent task.
pub struct TaskOutputTool {
    pub manager: Arc<BackgroundManager>,
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "task_output"
    }

    fn description(&self) -> &str {
        "Check the status and retrieve the result of a background subagent task. \
         Use the task_id returned when the subagent was spawned with run_in_background=true."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID returned by the agent tool when run_in_background was true"
                }
            },
            "required": ["task_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let task_id = input["task_id"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'task_id' parameter".into()))?;

        match self.manager.get(task_id).await {
            Some(task) => {
                let status = task.status.as_str();
                let elapsed = task.start_time.elapsed().as_secs();
                let mut output = format!(
                    "task_id: {task_id}\n\
                     description: {}\n\
                     status: {status}\n\
                     elapsed: {elapsed}s",
                    task.description
                );
                if let Some(result) = &task.result {
                    output.push_str(&format!("\n\n[result]\n{result}"));
                }
                if let Some(error) = &task.error {
                    output.push_str(&format!("\n\n[error]\n{error}"));
                }
                Ok(ToolResult::success(output))
            }
            None => {
                let output = format!(
                    "task_id: {task_id}\n\
                     status: not_found\n\n\
                     No background task found with this ID. It may have completed and been removed, or the ID may be incorrect."
                );
                Ok(ToolResult::error(output))
            }
        }
    }
}

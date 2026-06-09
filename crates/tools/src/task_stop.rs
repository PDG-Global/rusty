// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::background::BackgroundManager;
use crate::{Tool, ToolContext, ToolResult};

/// Stop (cancel) a running background subagent task.
pub struct TaskStopTool {
    pub manager: Arc<BackgroundManager>,
}

#[async_trait]
impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "task_stop"
    }

    fn description(&self) -> &str {
        "Stop a background subagent task that is still running. \
         Use the task_id returned when the subagent was spawned with run_in_background=true. \
         Note: this marks the task as stopped but does not force-kill the underlying process."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID of the background subagent to stop"
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
                if task.status == crate::background::BackgroundTaskStatus::Running {
                    self.manager.stop(task_id).await;
                    let output = format!(
                        "task_id: {task_id}\n\
                         status: stopped\n\n\
                         The background task has been marked as stopped."
                    );
                    Ok(ToolResult::success(output))
                } else {
                    let output = format!(
                        "task_id: {task_id}\n\
                         status: {}\n\n\
                         The task is not running and cannot be stopped.",
                        task.status.as_str()
                    );
                    Ok(ToolResult::error(output))
                }
            }
            None => {
                let output = format!(
                    "task_id: {task_id}\n\
                     status: not_found\n\n\
                     No background task found with this ID."
                );
                Ok(ToolResult::error(output))
            }
        }
    }
}

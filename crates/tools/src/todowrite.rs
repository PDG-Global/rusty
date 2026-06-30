// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::task::TaskRegistry;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use std::sync::Arc;

use crate::{Tool, ToolContext, ToolResult};

pub struct TodoWriteTool {
    registry: Arc<TaskRegistry>,
    session_id: String,
}

impl TodoWriteTool {
    pub fn new(registry: Arc<TaskRegistry>, session_id: String) -> Self {
        Self {
            registry,
            session_id,
        }
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Manage a structured task list for tracking progress through multi-step work. \
         Tasks have stable IDs (T1, T1.1, T2, ...) that do not change when other tasks \
         are added or completed. The list is persisted to disk and injected into your \
         system prompt each turn.\n\n\
         Operations:\n\
         - create: register a new task. summary required. optional: parent_id.\n\
         - list: enumerate tasks (defaults to active only).\n\
         - start: mark a task in_progress. id required.\n\
         - block: mark a task blocked. id required. optional: reason.\n\
         - unblock: mark blocked task as open. id required.\n\
         - done: mark a task complete. id required. optional: summary.\n\
         - abandon: drop a task without completing. id required. optional: reason.\n\
         - rename: change a task's summary. id + summary required.\n\n\
         Status lifecycle: open ⇄ in_progress → blocked → done | abandoned.\n\
         Keep one task in_progress when work is underway. Only mark done when fully complete.\n\n\
         Backward compatible: passing {\"todos\": [...]} auto-creates tasks from the list."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "object",
                    "description": "Task operation to perform.",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "list", "start", "block", "unblock", "done", "abandon", "rename"],
                            "description": "The operation to perform"
                        },
                        "id": {
                            "type": "string",
                            "description": "Task id, e.g. T1 or T1.1. Required for start/block/unblock/done/abandon/rename."
                        },
                        "summary": {
                            "type": "string",
                            "description": "Task summary. Required for create and rename."
                        },
                        "parent_id": {
                            "type": "string",
                            "description": "Parent task id for sub-tasks (create only)."
                        },
                        "reason": {
                            "type": "string",
                            "description": "Reason for block/unblock/abandon."
                        }
                    },
                    "required": ["action"]
                },
                "todos": {
                    "type": "array",
                    "description": "(Legacy) Full list of TODO items. Auto-creates tasks from the list.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"]
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"]
                            }
                        },
                        "required": ["content"]
                    }
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        // Check for new-style operation first
        if let Some(op) = input.get("operation") {
            return self.execute_operation(op).await;
        }

        // Backward-compatible: legacy {todos: [...]} format
        if let Some(todos) = input.get("todos").and_then(|v| v.as_array()) {
            return self.execute_legacy_todos(todos).await;
        }

        // No operation or todos — just list current tasks
        let rendered = self.registry.render_for_tool_output(&self.session_id);
        Ok(ToolResult::success(rendered))
    }
}

impl TodoWriteTool {
    async fn execute_operation(&self, op: &Value) -> Result<ToolResult, RustyError> {
        let action = op
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RustyError::Tool("Missing 'action' in operation".into()))?;

        let sid = &self.session_id;

        match action {
            "create" => {
                let summary = op
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RustyError::Tool("create requires 'summary'".into()))?;
                let parent_id = op.get("parent_id").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .create(sid, summary, parent_id, None)
                    .map_err(|e| RustyError::Tool(format!("Failed to create task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "Created {} — {}\n\n{}",
                    task.id, task.summary, rendered
                )))
            }
            "list" => {
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(rendered))
            }
            "start" => {
                let id = require_id(op)?;
                let task = self
                    .registry
                    .start(sid, id, None)
                    .map_err(|e| RustyError::Tool(format!("Failed to start task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} → {}\n\n{}",
                    task.id,
                    task.status.as_str(),
                    rendered
                )))
            }
            "block" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .block(sid, id, reason)
                    .map_err(|e| RustyError::Tool(format!("Failed to block task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} → {}\n\n{}",
                    task.id,
                    task.status.as_str(),
                    rendered
                )))
            }
            "unblock" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .unblock(sid, id, reason)
                    .map_err(|e| RustyError::Tool(format!("Failed to unblock task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} → {}\n\n{}",
                    task.id,
                    task.status.as_str(),
                    rendered
                )))
            }
            "done" => {
                let id = require_id(op)?;
                let summary = op.get("summary").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .done(sid, id, summary)
                    .map_err(|e| RustyError::Tool(format!("Failed to complete task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} → done\n\n{}",
                    task.id, rendered
                )))
            }
            "abandon" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .abandon(sid, id, reason)
                    .map_err(|e| RustyError::Tool(format!("Failed to abandon task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} → abandoned\n\n{}",
                    task.id, rendered
                )))
            }
            "rename" => {
                let id = require_id(op)?;
                let summary = op
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RustyError::Tool("rename requires 'summary'".into()))?;
                let task = self
                    .registry
                    .rename(sid, id, summary)
                    .map_err(|e| RustyError::Tool(format!("Failed to rename task: {e}")))?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} renamed → \"{}\"\n\n{}",
                    task.id, task.summary, rendered
                )))
            }
            other => Err(RustyError::Tool(format!(
                "Unknown operation: '{other}'. Use: create, list, start, block, unblock, done, abandon, rename"
            ))),
        }
    }

    /// Backward-compatible: create tasks from a legacy {todos: [...]} array.
    /// Maps old status values to new ones and creates tasks in the registry.
    async fn execute_legacy_todos(&self, todos: &[Value]) -> Result<ToolResult, RustyError> {
        let sid = &self.session_id;

        // Get existing tasks to avoid re-creating duplicates
        let existing = self
            .registry
            .list(sid, None, true)
            .unwrap_or_default();

        // If there are no existing tasks, create from the list
        if existing.is_empty() {
            for item in todos {
                let summary = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if summary.is_empty() {
                    continue;
                }
                let status_str = item
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");

                let task = self
                    .registry
                    .create(sid, &summary, None, None)
                    .map_err(|e| RustyError::Tool(format!("Failed to create task: {e}")))?;

                // Apply status if not default "pending"
                match status_str {
                    "in_progress" => {
                        let _ = self.registry.start(sid, &task.id, None);
                    }
                    "completed" | "done" => {
                        let _ = self.registry.done(sid, &task.id, None);
                    }
                    "cancelled" | "canceled" => {
                        let _ = self.registry.abandon(sid, &task.id, None);
                    }
                    _ => {} // "pending" is the default
                }
            }
        } else {
            // Tasks already exist — update statuses based on the todos list
            // Match by summary text (best effort)
            for item in todos {
                let summary = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status_str = item
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pending");

                if let Some(task) = existing.iter().find(|t| t.summary == summary) {
                    match status_str {
                        "in_progress" if task.status != rusty_core::task::TaskStatus::InProgress => {
                            let _ = self.registry.start(sid, &task.id, None);
                        }
                        "completed" | "done" if !task.status.is_terminal() => {
                            let _ = self.registry.done(sid, &task.id, None);
                        }
                        "cancelled" | "canceled" if !task.status.is_terminal() => {
                            let _ = self.registry.abandon(sid, &task.id, None);
                        }
                        _ => {}
                    }
                } else {
                    // New task not in registry — create it
                    let task = self
                        .registry
                        .create(sid, &summary, None, None)
                        .map_err(|e| RustyError::Tool(format!("Failed to create task: {e}")))?;
                    match status_str {
                        "in_progress" => {
                            let _ = self.registry.start(sid, &task.id, None);
                        }
                        "completed" | "done" => {
                            let _ = self.registry.done(sid, &task.id, None);
                        }
                        "cancelled" | "canceled" => {
                            let _ = self.registry.abandon(sid, &task.id, None);
                        }
                        _ => {}
                    }
                }
            }
        }

        let rendered = self.registry.render_for_tool_output(sid);
        Ok(ToolResult::success(format!("Todo list updated.\n{}", rendered)))
    }
}

fn require_id(op: &Value) -> Result<&str, RustyError> {
    op.get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RustyError::Tool("This operation requires an 'id' field (e.g. T1, T1.1)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_core::task::TaskRegistry;
    use tempfile::TempDir;

    fn make_tool() -> (TodoWriteTool, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let registry = Arc::new(TaskRegistry::open(&db_path).unwrap());
        let tool = TodoWriteTool::new(registry, "test-session".to_string());
        (tool, dir)
    }

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            permission_mode: rusty_core::PermissionMode::BypassPermissions,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(
                json!({"operation": {"action": "create", "summary": "Implement auth"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("T1"));
        assert!(result.content.contains("Implement auth"));
    }

    #[tokio::test]
    async fn test_lifecycle() {
        let (tool, _dir) = make_tool();

        // Create
        tool.execute(
            json!({"operation": {"action": "create", "summary": "Task A"}}),
            &ctx(),
        )
        .await
        .unwrap();

        // Start
        let result = tool
            .execute(
                json!({"operation": {"action": "start", "id": "T1"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("in_progress"));

        // Done
        let result = tool
            .execute(
                json!({"operation": {"action": "done", "id": "T1", "summary": "all tests pass"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("done"));
    }

    #[tokio::test]
    async fn test_subtasks() {
        let (tool, _dir) = make_tool();

        tool.execute(
            json!({"operation": {"action": "create", "summary": "Parent"}}),
            &ctx(),
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"operation": {"action": "create", "summary": "Child", "parent_id": "T1"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("T1.1"));
    }

    #[tokio::test]
    async fn test_legacy_todos_format() {
        let (tool, _dir) = make_tool();

        let result = tool
            .execute(
                json!({
                    "todos": [
                        {"content": "Task A", "status": "pending"},
                        {"content": "Task B", "status": "in_progress"},
                        {"content": "Task C", "status": "completed"}
                    ]
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("Todo list updated"));
        assert!(result.content.contains("T1"));
        assert!(result.content.contains("Task A"));
    }

    #[tokio::test]
    async fn test_no_operation_lists_tasks() {
        let (tool, _dir) = make_tool();
        let result = tool.execute(json!({}), &ctx()).await.unwrap();
        assert!(result.content.contains("No tasks"));
    }

    #[tokio::test]
    async fn test_block_and_unblock() {
        let (tool, _dir) = make_tool();

        tool.execute(
            json!({"operation": {"action": "create", "summary": "Task"}}),
            &ctx(),
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"operation": {"action": "block", "id": "T1", "reason": "waiting on dep"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("blocked"));

        let result = tool
            .execute(
                json!({"operation": {"action": "unblock", "id": "T1"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("open"));
    }

    #[tokio::test]
    async fn test_rename() {
        let (tool, _dir) = make_tool();

        tool.execute(
            json!({"operation": {"action": "create", "summary": "Old name"}}),
            &ctx(),
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"operation": {"action": "rename", "id": "T1", "summary": "New name"}}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.content.contains("New name"));
    }
}

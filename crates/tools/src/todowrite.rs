// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::task::{TaskRegistry, TaskStatus};
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use std::sync::{Arc, RwLock};

use crate::{Tool, ToolContext, ToolResult};

pub struct TodoWriteTool {
    registry: Arc<TaskRegistry>,
    session_id: Arc<RwLock<String>>,
}

impl TodoWriteTool {
    pub fn new(registry: Arc<TaskRegistry>, session_id: String) -> Self {
        Self {
            registry,
            session_id: Arc::new(RwLock::new(session_id)),
        }
    }

    /// Shared handle to the active session id; swap its value to change sessions (e.g. on /resume).
    pub fn session_handle(&self) -> Arc<RwLock<String>> {
        self.session_id.clone()
    }

    fn current_session(&self) -> String {
        self.session_id
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }
}

const FORMAT_HELP: &str = "Expected {\"operation\": {\"action\": ..., \"id\": ..., \"summary\": ...}}, \
flat args like {\"action\": \"done\", \"id\": \"T2\"}, or legacy {\"todos\": [...]}. \
Actions: create, list, start, block, unblock, done, abandon, rename, reopen.";

fn terr<E: std::fmt::Display>(e: E) -> RustyError {
    RustyError::Tool(e.to_string())
}

/// Normalise action synonyms to the canonical action names.
fn canonical_action(a: &str) -> String {
    match a.trim().to_lowercase().as_str() {
        "add" | "new" => "create".into(),
        "complete" | "completed" | "finish" => "done".into(),
        "cancel" | "cancelled" | "canceled" => "abandon".into(),
        "update" | "status" | "set_status" | "set" => "set_status".into(),
        other => other.to_string(),
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
         are added or completed. The current task list is shown to you in the system prompt.\n\n\
         Operations (pass as {\"action\": ..., \"id\": ..., \"summary\": ...}):\n\
         - create: register a new task. summary required. optional: parent_id.\n\
         - list: enumerate tasks.\n\
         - start: mark a task in_progress (also reopens a done/abandoned task). id required.\n\
         - block: mark a task blocked. id required. optional: reason.\n\
         - unblock: mark blocked task as open. id required.\n\
         - done: mark a task complete. id required. optional: summary.\n\
         - abandon: drop a task without completing. id required. optional: reason.\n\
         - rename: change a task's summary. id + summary required.\n\
         - reopen: reopen a done/abandoned task. id required.\n\n\
         Keep one task in_progress when work is underway. Only mark done when fully complete.\n\
         Legacy: {\"todos\": [...]} replaces the whole list (tasks omitted from it are abandoned)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "object",
                    "description": "Task operation to perform. The same fields may also be given at the top level.",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "list", "start", "block", "unblock", "done", "abandon", "rename", "reopen"],
                            "description": "The operation to perform"
                        },
                        "id": {
                            "type": "string",
                            "description": "Task id, e.g. T1 or T1.1. Required for all but create/list."
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
                "action": {"type": "string", "description": "Flat alternative to operation.action."},
                "id": {"type": "string", "description": "Flat alternative to operation.id."},
                "summary": {"type": "string", "description": "Flat alternative to operation.summary."},
                "status": {"type": "string", "description": "With id: set status (open, in_progress, blocked, done, abandoned)."},
                "todos": {
                    "type": "array",
                    "description": "(Legacy) Full list of TODO items; replaces the task list.",
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
        let sid = self.current_session();

        // Nested operation object
        if let Some(op) = input.get("operation").filter(|v| v.is_object()) {
            return self.execute_operation(&sid, op);
        }

        // Flat args: top-level action (or id+status)
        let has_action = input.get("action").and_then(|v| v.as_str()).is_some();
        let has_id_status = input.get("id").is_some() && input.get("status").is_some();
        if has_action || has_id_status {
            return self.execute_operation(&sid, &input);
        }

        // Legacy {todos: [...]}
        if let Some(todos) = input.get("todos").and_then(|v| v.as_array()) {
            return self.execute_legacy_todos(&sid, todos);
        }

        // Empty call: list current tasks
        let empty = match &input {
            Value::Null => true,
            Value::Object(m) => m.is_empty(),
            _ => false,
        };
        if empty {
            return Ok(ToolResult::success(self.registry.render_for_tool_output(&sid)));
        }

        Err(RustyError::Tool(format!(
            "Unrecognised todowrite input. {FORMAT_HELP}"
        )))
    }
}

impl TodoWriteTool {
    fn execute_operation(&self, sid: &str, op: &Value) -> Result<ToolResult, RustyError> {
        let raw_action = op.get("action").and_then(|v| v.as_str());
        let action = match raw_action {
            Some(a) => canonical_action(a),
            None if op.get("id").is_some() && op.get("status").is_some() => "set_status".into(),
            None => {
                return Err(RustyError::Tool(format!(
                    "Missing 'action' in operation. {FORMAT_HELP}"
                )))
            }
        };
        let action = action.as_str();

        let transition_output = |task_id: &str, label: &str| {
            let rendered = self.registry.render_for_tool_output(sid);
            Ok(ToolResult::success(format!("{task_id} → {label}\n\n{rendered}")))
        };

        match action {
            "create" => {
                let summary = op
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| RustyError::Tool("create requires 'summary'".into()))?;
                let parent_id = op.get("parent_id").and_then(|v| v.as_str());
                let task = self
                    .registry
                    .create(sid, summary, parent_id, None)
                    .map_err(terr)?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "Created {} — {}\n\n{}",
                    task.id, task.summary, rendered
                )))
            }
            "list" => Ok(ToolResult::success(self.registry.render_for_tool_output(sid))),
            "start" => {
                let id = require_id(op)?;
                let task = self.registry.start(sid, id, None).map_err(terr)?;
                transition_output(&task.id, task.status.as_str())
            }
            "reopen" => {
                let id = require_id(op)?;
                let task = self.registry.reopen(sid, id).map_err(terr)?;
                transition_output(&task.id, task.status.as_str())
            }
            "block" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self.registry.block(sid, id, reason).map_err(terr)?;
                transition_output(&task.id, task.status.as_str())
            }
            "unblock" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self.registry.unblock(sid, id, reason).map_err(terr)?;
                transition_output(&task.id, task.status.as_str())
            }
            "done" => {
                let id = require_id(op)?;
                let summary = op.get("summary").and_then(|v| v.as_str());
                let task = self.registry.done(sid, id, summary).map_err(terr)?;
                transition_output(&task.id, "done")
            }
            "abandon" => {
                let id = require_id(op)?;
                let reason = op.get("reason").and_then(|v| v.as_str());
                let task = self.registry.abandon(sid, id, reason).map_err(terr)?;
                transition_output(&task.id, "abandoned")
            }
            "set_status" => {
                let id = require_id(op)?;
                let raw = op
                    .get("status")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RustyError::Tool("status update requires 'status'".into()))?;
                let status = TaskStatus::parse(raw).ok_or_else(|| {
                    RustyError::Tool(format!(
                        "Unknown status '{raw}'. Use: open, in_progress, blocked, done, abandoned"
                    ))
                })?;
                let task = self.registry.set_status(sid, id, status).map_err(terr)?;
                transition_output(&task.id, task.status.as_str())
            }
            "rename" => {
                let id = require_id(op)?;
                let summary = op
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RustyError::Tool("rename requires 'summary'".into()))?;
                let task = self.registry.rename(sid, id, summary).map_err(terr)?;
                let rendered = self.registry.render_for_tool_output(sid);
                Ok(ToolResult::success(format!(
                    "{} renamed → \"{}\"\n\n{}",
                    task.id, task.summary, rendered
                )))
            }
            other => Err(RustyError::Tool(format!(
                "Unknown operation: '{other}'. Use: create, list, start, block, unblock, done, abandon, rename, reopen"
            ))),
        }
    }

    /// Legacy {todos: [...]}: a full replacement of the session's task list.
    /// Matches existing tasks by normalised summary, creates new ones, applies
    /// statuses, and abandons non-terminal tasks missing from the list.
    fn execute_legacy_todos(&self, sid: &str, todos: &[Value]) -> Result<ToolResult, RustyError> {
        // Validate everything first so a bad status changes nothing.
        let mut items: Vec<(String, TaskStatus)> = Vec::new();
        for item in todos {
            let summary = item
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if summary.is_empty() {
                continue;
            }
            let status = match item.get("status").and_then(|v| v.as_str()) {
                None => TaskStatus::Open,
                Some(raw) => TaskStatus::parse(raw).ok_or_else(|| {
                    RustyError::Tool(format!(
                        "Unknown status '{raw}' for todo '{summary}'. Use: pending, in_progress, completed, cancelled, blocked"
                    ))
                })?,
            };
            items.push((summary, status));
        }

        let existing = self.registry.list(sid, None, true).map_err(terr)?;
        let mut matched: Vec<String> = Vec::new();

        for (summary, status) in &items {
            let key = normalize(summary);
            let found = existing
                .iter()
                .find(|t| !matched.contains(&t.id) && normalize(&t.summary) == key);
            match found {
                Some(task) => {
                    matched.push(task.id.clone());
                    if task.status != *status {
                        self.registry
                            .set_status(sid, &task.id, status.clone())
                            .map_err(terr)?;
                    }
                }
                None => {
                    let task = self.registry.create(sid, summary, None, None).map_err(terr)?;
                    matched.push(task.id.clone());
                    if *status != TaskStatus::Open {
                        self.registry
                            .set_status(sid, &task.id, status.clone())
                            .map_err(terr)?;
                    }
                }
            }
        }

        // Full replacement: abandon live tasks that were left out
        for task in &existing {
            if !task.status.is_terminal() && !matched.contains(&task.id) {
                self.registry
                    .abandon(sid, &task.id, Some("removed from todo list"))
                    .map_err(terr)?;
            }
        }

        let rendered = self.registry.render_for_tool_output(sid);
        Ok(ToolResult::success(format!("Todo list updated.\n{}", rendered)))
    }
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
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
            on_question: None,
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
    async fn test_empty_input_lists_tasks() {
        let (tool, _dir) = make_tool();
        let result = tool.execute(json!({}), &ctx()).await.unwrap();
        assert!(result.content.contains("No tasks"));
    }

    #[tokio::test]
    async fn test_unrecognised_input_is_error() {
        let (tool, _dir) = make_tool();
        let err = tool
            .execute(json!({"foo": "bar"}), &ctx())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Expected"));
    }

    #[tokio::test]
    async fn test_flat_args() {
        let (tool, _dir) = make_tool();
        let r = tool
            .execute(json!({"action": "add", "summary": "Flat task"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("T1"));
        let r = tool
            .execute(json!({"action": "done", "id": "T1"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("T1 (done)"));
        let r = tool
            .execute(json!({"id": "T1", "status": "in_progress"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("T1 (in_progress)"));
        // Errors surface clearly
        let err = tool
            .execute(json!({"action": "start", "id": "T9"}), &ctx())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("No task T9 in this session"));
    }

    #[tokio::test]
    async fn test_done_twice_errors_and_reopen() {
        let (tool, _dir) = make_tool();
        tool.execute(json!({"action": "create", "summary": "A"}), &ctx())
            .await
            .unwrap();
        tool.execute(json!({"action": "done", "id": "T1"}), &ctx())
            .await
            .unwrap();
        let err = tool
            .execute(json!({"action": "done", "id": "T1"}), &ctx())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("T1 is already done"));
        let r = tool
            .execute(json!({"action": "reopen", "id": "T1"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("T1 (open)"));
    }

    #[tokio::test]
    async fn test_session_handle_swap() {
        let (tool, _dir) = make_tool();
        tool.execute(json!({"action": "create", "summary": "A"}), &ctx())
            .await
            .unwrap();
        *tool.session_handle().write().unwrap() = "other".to_string();
        let r = tool.execute(json!({}), &ctx()).await.unwrap();
        assert!(r.content.contains("No tasks"));
        let r = tool
            .execute(json!({"action": "create", "summary": "B"}), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("Created T1"));
    }

    #[tokio::test]
    async fn test_legacy_status_synonyms_and_unknown() {
        let (tool, _dir) = make_tool();
        let r = tool
            .execute(
                json!({"todos": [
                    {"content": "A", "status": "todo"},
                    {"content": "B", "status": "active"},
                    {"content": "C", "status": "done"},
                    {"content": "D", "status": "canceled"},
                    {"content": "E", "status": "blocked"}
                ]}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(r.content.contains("T1 (open)"));
        assert!(r.content.contains("T2 (in_progress)"));
        assert!(r.content.contains("T3 (done)"));
        assert!(r.content.contains("T4 (abandoned)"));
        assert!(r.content.contains("T5 (blocked)"));

        let err = tool
            .execute(json!({"todos": [{"content": "A", "status": "wat"}]}), &ctx())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown status 'wat'"));
    }

    #[tokio::test]
    async fn test_legacy_matching_and_replacement() {
        let (tool, _dir) = make_tool();
        tool.execute(
            json!({"todos": [
                {"content": "Write code", "status": "pending"},
                {"content": "Old item", "status": "pending"}
            ]}),
            &ctx(),
        )
        .await
        .unwrap();
        let r = tool
            .execute(
                json!({"todos": [
                    {"content": "  write   CODE ", "status": "completed"},
                    {"content": "New item", "status": "pending"}
                ]}),
                &ctx(),
            )
            .await
            .unwrap();
        // Matched case/whitespace-insensitively, not duplicated
        assert!(r.content.contains("T1 (done)"));
        // Omitted task abandoned, new one created
        assert!(r.content.contains("T2 (abandoned)"));
        assert!(r.content.contains("T3 (open)"));
        assert!(!r.content.contains("T4"));
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

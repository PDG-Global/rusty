// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::plan::{Plan, PlanItem, PlanItemPriority, PlanItemStatus};
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use std::sync::Arc;

use crate::{Tool, ToolContext, ToolResult};

pub struct TodoWriteTool {
    plan: Arc<tokio::sync::Mutex<Plan>>,
}

impl TodoWriteTool {
    pub fn new(plan: Arc<tokio::sync::Mutex<Plan>>) -> Self {
        Self { plan }
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Manage a structured task list for tracking progress through multi-step work. \
         Use this tool to create and update a TODO list. The list is persisted to disk \
         and injected into your system prompt each turn, so you do not need to remember it. \
         Always pass the full current state when updating. Keep titles short and actionable. \
         When work is underway, keep exactly one task in_progress. Only mark a task completed \
         when it is fully done — not when you have merely planned or started it."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The full list of TODO items. Pass the complete current list each time.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "The task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "The current status of the task"
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"],
                                "description": "The priority level of the task"
                            }
                        },
                        "required": ["content"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let todos = input
            .get("todos")
            .and_then(|v| v.as_array())
            .ok_or_else(|| RustyError::Tool("Missing or invalid 'todos' array".into()))?;

        // Replace the entire plan with the new items.
        {
            let mut plan = self.plan.lock().await;
            let items: Vec<PlanItem> = todos
                .iter()
                .map(|item| {
                    let content = item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let status = PlanItemStatus::from_str(
                        item.get("status").and_then(|v| v.as_str()).unwrap_or("pending"),
                    );
                    let priority = PlanItemPriority::from_str(
                        item.get("priority")
                            .and_then(|v| v.as_str())
                            .unwrap_or("medium"),
                    );
                    PlanItem {
                        content,
                        status,
                        priority,
                    }
                })
                .collect();
            plan.set_items(items);
            if let Err(e) = plan.save() {
                return Err(RustyError::Tool(format!("Failed to save plan: {e}")));
            }
        }

        // Return the full formatted list so the model can see the current state
        // in the conversation history.
        let plan = self.plan.lock().await;
        let rendered = plan.render_for_tool_output();
        let output = format!(
            "Todo list updated.\n{}\n\n\
             Continue using the todo list to track progress. Mark tasks done immediately \
             after finishing them, and keep exactly one task in_progress when work is underway.",
            rendered,
        );

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_core::plan::Plan;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    fn make_plan() -> (Arc<Mutex<Plan>>, TempDir) {
        let dir = TempDir::new().unwrap();
        let plan = Plan::new(dir.path().to_string_lossy().to_string());
        (Arc::new(Mutex::new(plan)), dir)
    }

    #[tokio::test]
    async fn test_todowrite_create_list() {
        let (plan, _dir) = make_plan();
        let tool = TodoWriteTool::new(plan.clone());
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            permission_mode: rusty_core::PermissionMode::BypassPermissions,
            cancel: None,
        };

        let args = json!({
            "todos": [
                {"content": "Task A", "status": "pending", "priority": "high"},
                {"content": "Task B", "status": "in_progress", "priority": "medium"},
                {"content": "Task C", "status": "completed", "priority": "low"}
            ]
        });

        let result = tool.execute(args, &ctx).await.unwrap();
        let text = result.content.as_str();
        assert!(text.contains("Todo list updated"));
        assert!(text.contains("1. [pending] Task A"));
        assert!(text.contains("2. [in_progress] Task B"));
        assert!(text.contains("3. [completed] Task C"));
        assert!(text.contains("Continue using the todo list"));
    }

    #[tokio::test]
    async fn test_todowrite_full_replacement() {
        let (plan, _dir) = make_plan();
        let tool = TodoWriteTool::new(plan.clone());
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            permission_mode: rusty_core::PermissionMode::BypassPermissions,
            cancel: None,
        };

        // First update
        tool.execute(
            json!({"todos": [{"content": "Old task", "status": "pending"}]}),
            &ctx,
        )
        .await
        .unwrap();

        // Second update should replace, not append
        tool.execute(
            json!({"todos": [{"content": "New task", "status": "pending"}]}),
            &ctx,
        )
        .await
        .unwrap();

        let plan = plan.lock().await;
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].content, "New task");
    }

    #[tokio::test]
    async fn test_todowrite_persists_to_disk() {
        let dir = TempDir::new().unwrap();
        let mut plan = Plan::new(dir.path().to_string_lossy().to_string());
        plan.add_item("Seed".into(), PlanItemPriority::Low);
        plan.save().unwrap();

        let loaded = Plan::load_for_project(dir.path()).await.unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].content, "Seed");
    }
}

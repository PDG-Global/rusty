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
         Use this tool to create and update a TODO list that helps track what needs to be done, \
         what's currently in progress, and what's been completed. \
         The list persists across the conversation and will be injected into your context each turn, \
         so you cannot forget your commitments. \
         Always pass the full current state when updating."
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

        // Build output grouped by priority with status indicators.
        let plan = self.plan.lock().await;
        let high: Vec<_> = plan
            .items
            .iter()
            .filter(|i| i.priority == PlanItemPriority::High)
            .collect();
        let medium: Vec<_> = plan
            .items
            .iter()
            .filter(|i| i.priority == PlanItemPriority::Medium)
            .collect();
        let low: Vec<_> = plan
            .items
            .iter()
            .filter(|i| i.priority == PlanItemPriority::Low)
            .collect();

        let mut output = String::new();

        let format_items = |items: &[&PlanItem]| -> String {
            items
                .iter()
                .map(|i| format!("  {} {}", i.status.indicator(), i.content))
                .collect::<Vec<_>>()
                .join("\n")
        };

        if !high.is_empty() {
            output.push_str(&format!("[HIGH]\n{}\n", format_items(&high)));
        }
        if !medium.is_empty() {
            output.push_str(&format!("[MEDIUM]\n{}\n", format_items(&medium)));
        }
        if !low.is_empty() {
            output.push_str(&format!("[LOW]\n{}\n", format_items(&low)));
        }

        Ok(ToolResult::success(output.trim().to_string()))
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
        assert!(text.contains("[HIGH]"));
        assert!(text.contains("[ ] Task A"));
        assert!(text.contains("[~] Task B"));
        assert!(text.contains("[x] Task C"));
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

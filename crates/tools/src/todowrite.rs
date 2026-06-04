// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

pub struct TodoWriteTool;

#[derive(Debug, Clone)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub priority: String,
}

/// Valid status values and their display indicators.
const STATUS_MAP: &[(&str, &str)] = &[
    ("pending", "[ ]"),
    ("in_progress", "[~]"),
    ("completed", "[x]"),
    ("cancelled", "[-]"),
];

/// Normalize a status string, falling back to "pending" for unknown values.
fn normalize_status(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    if STATUS_MAP.iter().any(|(s, _)| *s == lower) {
        lower
    } else {
        "pending".to_string()
    }
}

/// Normalize a priority string, falling back to "medium" for unknown values.
fn normalize_priority(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "high" | "low" => raw.trim().to_lowercase(),
        _ => "medium".to_string(),
    }
}

/// Return the display indicator for a given status.
fn status_indicator(status: &str) -> &'static str {
    STATUS_MAP
        .iter()
        .find(|(s, _)| *s == status)
        .map(|(_, indicator)| *indicator)
        .unwrap_or("[ ]")
}

fn parse_todos(input: &Value) -> Result<Vec<TodoItem>, RustyError> {
    let todos_value = input
        .get("todos")
        .ok_or_else(|| RustyError::Tool("Missing required 'todos' parameter".into()))?;

    let todos_array = todos_value
        .as_array()
        .ok_or_else(|| RustyError::Tool("'todos' must be an array".into()))?;

    let mut todos = Vec::with_capacity(todos_array.len());

    for (i, item) in todos_array.iter().enumerate() {
        if !item.is_object() {
            return Err(RustyError::Tool(format!(
                "todos[{i}]: expected an object, got {}",
                item_type_name(item)
            )));
        }

        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        if content.is_empty() {
            return Err(RustyError::Tool(format!(
                "todos[{i}]: 'content' is required and must be a non-empty string"
            )));
        }

        let status = item
            .get("status")
            .and_then(|v| v.as_str())
            .map(normalize_status)
            .unwrap_or_else(|| "pending".to_string());

        let priority = item
            .get("priority")
            .and_then(|v| v.as_str())
            .map(normalize_priority)
            .unwrap_or_else(|| "medium".to_string());

        todos.push(TodoItem {
            content,
            status,
            priority,
        });
    }

    Ok(todos)
}

fn format_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "No tasks.".to_string();
    }

    let counts = count_by_status(todos);
    let total = todos.len();
    let completed = counts.get("completed").copied().unwrap_or(0);

    let mut output = format!(
        "Todo List ({completed}/{total} completed)\n\
         {}\n",
        "─".repeat(40)
    );

    // Group by priority
    let priority_order = ["high", "medium", "low"];

    for priority in &priority_order {
        let group: Vec<&TodoItem> = todos
            .iter()
            .filter(|t| t.priority == *priority)
            .collect();

        if group.is_empty() {
            continue;
        }

        let label = format!("[{}]", priority.to_uppercase());
        output.push_str(&format!("\n{label}\n"));

        for item in &group {
            let indicator = status_indicator(&item.status);
            output.push_str(&format!("  {indicator} {}\n", item.content));
        }
    }

    output
}

fn count_by_status(todos: &[TodoItem]) -> std::collections::HashMap<String, usize> {
    let mut map = std::collections::HashMap::new();
    for item in todos {
        *map.entry(item.status.clone()).or_insert(0) += 1;
    }
    map
}

/// Extract incomplete tasks from a parsed JSON todos array.
/// Returns (status, content) pairs for items not marked completed or cancelled.
pub fn get_incomplete_tasks(todos: &[TodoItem]) -> Vec<(&str, &str)> {
    todos
        .iter()
        .filter(|t| t.status != "completed" && t.status != "cancelled")
        .map(|t| (t.status.as_str(), t.content.as_str()))
        .collect()
}

fn item_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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
         The list persists across the conversation — always pass the full current state when updating."
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
                        "required": ["content"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["todos"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let todos = parse_todos(&input)?;
        let output = format_todos(&todos);
        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dummy_ctx() -> crate::ToolContext {
        crate::ToolContext {
            working_dir: std::path::PathBuf::from("."),
            permission_mode: rusty_core::PermissionMode::Default,
        }
    }

    #[tokio::test]
    async fn test_basic_todo_list() {
        let input = json!({
            "todos": [
                { "content": "Write tests", "status": "in_progress", "priority": "high" },
                { "content": "Fix bug #42", "status": "pending", "priority": "high" },
                { "content": "Update docs", "status": "completed", "priority": "medium" },
                { "content": "Refactor utils", "status": "pending", "priority": "low" }
            ]
        });

        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[HIGH]"));
        assert!(result.content.contains("[MEDIUM]"));
        assert!(result.content.contains("[LOW]"));
        assert!(result.content.contains("[~] Write tests"));
        assert!(result.content.contains("[ ] Fix bug #42"));
        assert!(result.content.contains("[x] Update docs"));
        assert!(result.content.contains("[ ] Refactor utils"));
        assert!(result.content.contains("1/4 completed"));
    }

    #[tokio::test]
    async fn test_empty_todos() {
        let input = json!({ "todos": [] });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No tasks"));
    }

    #[tokio::test]
    async fn test_defaults_applied() {
        let input = json!({
            "todos": [
                { "content": "A task with no status or priority" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(!result.is_error);
        // Should default to pending (no indicator yet) and medium priority
        assert!(result.content.contains("[ ] A task with no status or priority"));
        assert!(result.content.contains("[MEDIUM]"));
    }

    #[tokio::test]
    async fn test_invalid_status_defaults_to_pending() {
        let input = json!({
            "todos": [
                { "content": "Oops", "status": "bananas" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[ ] Oops"));
    }

    #[tokio::test]
    async fn test_invalid_priority_defaults_to_medium() {
        let input = json!({
            "todos": [
                { "content": "Task", "priority": "urgent" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[MEDIUM]"));
    }

    #[tokio::test]
    async fn test_missing_todos_field() {
        let input = json!({});
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_todos_not_array() {
        let input = json!({ "todos": "not an array" });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_content_rejected() {
        let input = json!({
            "todos": [
                { "content": "" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_priority_grouping_order() {
        let input = json!({
            "todos": [
                { "content": "Low task", "priority": "low" },
                { "content": "High task", "priority": "high" },
                { "content": "Medium task", "priority": "medium" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        let high_pos = result.content.find("[HIGH]").unwrap();
        let med_pos = result.content.find("[MEDIUM]").unwrap();
        let low_pos = result.content.find("[LOW]").unwrap();
        assert!(high_pos < med_pos, "HIGH should come before MEDIUM");
        assert!(med_pos < low_pos, "MEDIUM should come before LOW");
    }

    #[tokio::test]
    async fn test_cancelled_status() {
        let input = json!({
            "todos": [
                { "content": "Dropped idea", "status": "cancelled" }
            ]
        });
        let result = TodoWriteTool.execute(input, &dummy_ctx()).await.unwrap();
        assert!(result.content.contains("[-] Dropped idea"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = TodoWriteTool;
        assert_eq!(tool.name(), "todowrite");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.permission_level(), PermissionLevel::None);

        let schema = tool.input_schema();
        assert!(schema["properties"]["todos"].is_object());
        assert!(schema["required"].as_array().unwrap().contains(&json!("todos")));
    }
}

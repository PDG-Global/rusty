// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusty_core::memory::ProjectMemory;
use rusty_core::{PermissionLevel, RustyError, ToolDefinition};

use crate::{Tool, ToolContext, ToolResult};

pub struct MemoryTool {
    pub memory: Arc<Mutex<ProjectMemory>>,
}

impl MemoryTool {
    /// Create a new MemoryTool from a ProjectMemory instance.
    /// Wraps it in Arc<Mutex<...>> automatically.
    pub fn new(memory: ProjectMemory) -> Self {
        Self {
            memory: Arc::new(Mutex::new(memory)),
        }
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Manage project memories: save, search, list, and delete persistent notes \
            for the current project. Memories are stored per-project and persist across \
            sessions. Use this to remember important facts about the codebase, user \
            preferences, conventions, or anything you want to recall later.\n\n\
            Actions:\n\
            - save: Save a new memory. Provide `content`.\n\
            - search: Find memories matching a `query` (case-insensitive substring).\n\
            - list: List all saved memories.\n\
            - delete: Remove a memory by `id`."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "search", "list", "delete"],
                    "description": "The action to perform."
                },
                "content": {
                    "type": "string",
                    "description": "The memory content to save (required for 'save')."
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for 'search')."
                },
                "id": {
                    "type": "string",
                    "description": "Memory ID to delete (required for 'delete')."
                }
            },
            "required": ["action"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, RustyError> {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RustyError::Tool("missing required field: action".into()))?;

        match action {
            "save" => {
                let content = input
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        RustyError::Tool("'save' requires 'content' field".into())
                    })?
                    .to_string();

                if content.trim().is_empty() {
                    return Err(RustyError::Tool(
                        "memory content cannot be empty".into(),
                    ));
                }

                let entry = {
                    let mut memory = self.memory.lock();
                    let entry = memory.add(content).ok_or_else(|| {
                        RustyError::Tool(
                            "memory content is empty or contained only unsafe content".into(),
                        )
                    })?;
                    memory
                        .save()
                        .map_err(|e| RustyError::Tool(format!("failed to save memory: {e}")))?;
                    entry
                };
                Ok(ToolResult::success(format!(
                    "Saved memory (id: {}): {}",
                    entry.id, entry.content
                )))
            }
            "search" => {
                let query = input
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        RustyError::Tool("'search' requires 'query' field".into())
                    })?;

                let memory = self.memory.lock();
                let results = memory.search(query);

                if results.is_empty() {
                    return Ok(ToolResult::success(format!(
                        "No memories match '{}'.",
                        query
                    )));
                }

                let mut out = format!(
                    "{} memor{} found for '{}':\n",
                    results.len(),
                    if results.len() == 1 { "y" } else { "ies" },
                    query
                );
                for m in &results {
                    out.push_str(&format!("[{}] {}\n", m.id, m.content));
                }
                Ok(ToolResult::success(out))
            }
            "list" => {
                let memory = self.memory.lock();
                if memory.memories.is_empty() {
                    return Ok(ToolResult::success(
                        "No memories saved for this project.".to_string(),
                    ));
                }

                let mut out = format!(
                    "{} memor{} for this project:\n",
                    memory.memories.len(),
                    if memory.memories.len() == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                );
                for m in &memory.memories {
                    out.push_str(&format!("[{}] {}\n", m.id, m.content));
                }
                Ok(ToolResult::success(out))
            }
            "delete" => {
                let id = input
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        RustyError::Tool("'delete' requires 'id' field".into())
                    })?;

                let removed = {
                    let mut memory = self.memory.lock();
                    let removed = memory.remove(id);
                    if removed {
                        memory.save().map_err(|e| {
                            RustyError::Tool(format!("failed to save memory: {e}"))
                        })?;
                    }
                    removed
                };

                if removed {
                    Ok(ToolResult::success(format!("Deleted memory {}.", id)))
                } else {
                    Err(RustyError::Tool(format!(
                        "No memory found with id '{}'.",
                        id
                    )))
                }
            }
            _ => Err(RustyError::Tool(format!(
                "Unknown action '{}'. Use: save, search, list, delete",
                action
            ))),
        }
    }
}

/// Create a boxed MemoryTool with the given shared memory state.
pub fn make_memory_tool(memory: Arc<Mutex<ProjectMemory>>) -> Box<dyn Tool> {
    Box::new(MemoryTool { memory })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tool() -> (MemoryTool, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut pm = ProjectMemory::new(dir.path().to_string_lossy().to_string());
        pm.add("Use cargo check for validation".into());
        pm.add("Deploy to production on Fridays".into());
        pm.add("CARGO_BUILD_FLAGS set in env".into());
        let tool = MemoryTool {
            memory: Arc::new(Mutex::new(pm)),
        };
        (tool, dir)
    }

    fn ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            working_dir: dir.to_path_buf(),
            permission_mode: rusty_core::PermissionMode::BypassPermissions,
        }
    }

    #[tokio::test]
    async fn save_action() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "save",
                    "content": "remember this fact"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Saved memory"));
        assert!(result.content.contains("remember this fact"));

        let mem = tool.memory.lock();
        assert_eq!(mem.memories.len(), 4);
    }

    #[tokio::test]
    async fn save_empty_content_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "save",
                    "content": "   "
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn save_missing_content_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "save"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_action() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "search",
                    "query": "cargo"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("2 memor"));
        assert!(result.content.contains("cargo check"));
        assert!(result.content.contains("CARGO_BUILD"));
    }

    #[tokio::test]
    async fn search_no_match() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "search",
                    "query": "nonexistent"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No memories match"));
    }

    #[tokio::test]
    async fn search_missing_query_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "search"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_action() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(serde_json::json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("3 memor"));
        assert!(result.content.contains("cargo check"));
        assert!(result.content.contains("Deploy to production"));
        assert!(result.content.contains("CARGO_BUILD"));
    }

    #[tokio::test]
    async fn list_empty() {
        let dir = TempDir::new().unwrap();
        let pm = ProjectMemory::new(dir.path().to_string_lossy().to_string());
        let tool = MemoryTool {
            memory: Arc::new(Mutex::new(pm)),
        };
        let result = tool
            .execute(serde_json::json!({"action": "list"}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No memories"));
    }

    #[tokio::test]
    async fn delete_action() {
        let (tool, dir) = make_tool();
        let id = tool.memory.lock().memories[0].id.clone();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "delete",
                    "id": id
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Deleted memory"));
        assert_eq!(tool.memory.lock().memories.len(), 2);
    }

    #[tokio::test]
    async fn delete_nonexistent_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "delete",
                    "id": "no-such-id"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_missing_id_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "delete"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "invalid"
                }),
                &ctx(dir.path()),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_action_errors() {
        let (tool, dir) = make_tool();
        let result = tool
            .execute(serde_json::json!({}), &ctx(dir.path()))
            .await;
        assert!(result.is_err());
    }
}

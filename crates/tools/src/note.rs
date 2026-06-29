// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::{Tool, ToolContext, ToolResult};

/// Append-only scratchpad tool for recording observations during a session.
/// Content is written to a session-scoped notes.md file and processed
/// during checkpoint extraction.
pub struct NoteTool {
    notes_path: PathBuf,
}

impl NoteTool {
    pub fn new(notes_path: PathBuf) -> Self {
        Self { notes_path }
    }
}

#[async_trait]
impl Tool for NoteTool {
    fn name(&self) -> &str {
        "note"
    }

    fn description(&self) -> &str {
        "Record an observation or decision to a session scratchpad. \
         Use this to capture context that would otherwise be lost — \
         discovered patterns, important file locations, technical decisions, \
         gotchas, or anything you want to remember across compaction. \
         Notes are processed during context management and preserved in summaries. \
         Keep each note concise (1-3 sentences)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The observation or decision to record"
                }
            },
            "required": ["content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RustyError::Tool("Missing 'content' string".into()))?;

        if content.trim().is_empty() {
            return Ok(ToolResult::error("Note content cannot be empty"));
        }

        let entry = format!("{content}\n");

        // Append to notes file, creating it with template if it doesn't exist.
        if let Some(parent) = self.notes_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                RustyError::Tool(format!("Failed to create notes directory: {e}"))
            })?;
        }

        // If file doesn't exist, create with template header
        if !self.notes_path.exists() {
            let header = "# Session notes\n_Free-form scratchpad. Append entries as you go; the checkpoint writer reconciles them at checkpoint events._\n\n";
            tokio::fs::write(&self.notes_path, header).await.map_err(|e| {
                RustyError::Tool(format!("Failed to create notes file: {e}"))
            })?;
        }

        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.notes_path)
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to open notes file: {e}")))?
            .write_all(entry.as_bytes())
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to write note: {e}")))?;

        Ok(ToolResult::success(format!("Noted: {content}")))
    }
}

use tokio::io::AsyncWriteExt;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            permission_mode: rusty_core::PermissionMode::BypassPermissions,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn test_note_appends_to_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.md");
        let tool = NoteTool::new(path.clone());
        let ctx = make_ctx();

        tool.execute(json!({"content": "First observation"}), &ctx)
            .await
            .unwrap();
        tool.execute(json!({"content": "Second observation"}), &ctx)
            .await
            .unwrap();

        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(body.contains("First observation"));
        assert!(body.contains("Second observation"));
        // File starts with template header
        assert!(body.starts_with("# Session notes"));
        // Observations appear after the header
        assert!(body.find("First observation").unwrap() > body.find("# Session notes").unwrap());
    }

    #[tokio::test]
    async fn test_note_empty_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("notes.md");
        let tool = NoteTool::new(path);
        let ctx = make_ctx();

        let result = tool.execute(json!({"content": "  "}), &ctx).await.unwrap();
        assert!(result.is_error);
    }
}

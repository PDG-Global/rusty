use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating it and any parent directories as needed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let path_str = input["path"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'path' parameter".into()))?;
        let content = input["content"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'content' parameter".into()))?;

        let path = crate::resolve_path(path_str, &ctx.working_dir)?;
        debug!("Writing file: {}", path.display());

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| RustyError::Tool(format!("Failed to create directory: {e}")))?;
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| RustyError::Tool(format!("Failed to write {}: {e}", path.display())))?;

        let line_count = content.lines().count();
        Ok(ToolResult::success(format!(
            "Wrote {} ({} lines)",
            path.display(),
            line_count
        )))
    }
}

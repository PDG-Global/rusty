// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod agent;
pub mod bash;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod web_fetch;

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError, ToolDefinition};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub permission_mode: rusty_core::PermissionMode,
}

/// Resolve a path against the working directory and validate it stays within the sandbox.
/// Returns the canonicalized absolute path, or an error if it escapes the working directory.
pub fn resolve_path(path_str: &str, working_dir: &Path) -> Result<PathBuf, RustyError> {
    let raw = PathBuf::from(path_str);
    let joined = if raw.is_absolute() {
        raw
    } else {
        working_dir.join(raw)
    };

    // Canonicalize to resolve .., symlinks, etc.
    // If the file doesn't exist yet, canonicalize the parent and append the filename.
    let canonical = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| RustyError::Tool(format!("Cannot resolve path '{}': {e}", path_str)))?
    } else {
        // For new files, canonicalize the parent directory
        let parent = joined.parent().unwrap_or(Path::new("."));
        let file_name = joined
            .file_name()
            .ok_or_else(|| RustyError::Tool(format!("Invalid path: '{}'", path_str)))?;
        let canon_parent = parent.canonicalize().map_err(|e| {
            RustyError::Tool(format!(
                "Cannot resolve parent directory for '{}': {e}",
                path_str
            ))
        })?;
        canon_parent.join(file_name)
    };

    let canon_working = working_dir.canonicalize().map_err(|e| {
        RustyError::Tool(format!("Cannot resolve working directory: {e}"))
    })?;

    if !canonical.starts_with(&canon_working) {
        return Err(RustyError::Tool(format!(
            "Access denied: '{}' is outside the working directory ({})",
            canonical.display(),
            canon_working.display()
        )));
    }

    Ok(canonical)
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn permission_level(&self) -> PermissionLevel;

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError>;
}

/// Returns all built-in tools (excluding AgentTool which requires special construction)
pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(bash::BashTool),
        Box::new(file_read::FileReadTool),
        Box::new(file_edit::FileEditTool),
        Box::new(file_write::FileWriteTool),
        Box::new(glob::GlobTool),
        Box::new(grep::GrepTool),
        Box::new(web_fetch::WebFetchTool::new()),
    ]
}

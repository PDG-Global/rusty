// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g. '**/*.rs', 'src/**/*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to working directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let pattern = input["pattern"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'pattern' parameter".into()))?;

        let search_dir = match input["path"].as_str() {
            Some(p) => crate::resolve_path(p, &ctx.working_dir)?,
            None => ctx.working_dir.clone(),
        };

        debug!("Glob search: {pattern} in {}", search_dir.display());

        // Use walkdir for recursive matching
        let mut matches = Vec::new();
        let max_results = 500;
        let matcher = glob::Pattern::new(pattern)
            .map_err(|e| RustyError::Tool(format!("Invalid glob pattern: {e}")))?;

        // Directories to skip (build artifacts, dependencies, etc.)
        const SKIP_DIRS: &[&str] = &[
            "node_modules", "target", "__pycache__", ".git", ".svn", ".hg",
            "dist", "build", ".next", ".nuxt", ".cache", "vendor", "venv",
            ".venv", "env", ".tox", ".mypy_cache", ".pytest_cache",
            "coverage", ".turbo", ".parcel-cache",
        ];

        let walker = walkdir::WalkDir::new(&search_dir)
            .follow_links(false)
            .into_iter();

        for entry in walker.filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            // Skip hidden directories
            if name.starts_with('.') {
                return false;
            }
            // Skip known large directories
            if e.file_type().is_dir() && SKIP_DIRS.contains(&name) {
                return false;
            }
            true
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let relative = path
                .strip_prefix(&search_dir)
                .unwrap_or(path)
                .to_string_lossy();

            if matcher.matches(&relative) {
                matches.push(path.display().to_string());
                if matches.len() >= max_results {
                    break;
                }
            }
        }

        matches.sort();

        if matches.is_empty() {
            Ok(ToolResult::success("No files found matching pattern."))
        } else {
            let count = matches.len();
            let truncated = if count >= max_results {
                format!(" (truncated to {max_results})")
            } else {
                String::new()
            };
            let list = matches.join("\n");
            Ok(ToolResult::success(format!(
                "Found {count} files{truncated}:\n{list}"
            )))
        }
    }
}

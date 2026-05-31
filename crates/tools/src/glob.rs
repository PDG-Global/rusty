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
        let matcher = glob::Pattern::new(pattern)
            .map_err(|e| RustyError::Tool(format!("Invalid glob pattern: {e}")))?;

        let walker = walkdir::WalkDir::new(&search_dir)
            .follow_links(false)
            .into_iter();

        for entry in walker.filter_entry(|e| {
            // Skip hidden directories
            !e.file_name()
                .to_str()
                .map(|s| s.starts_with('.'))
                .unwrap_or(false)
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
            }
        }

        matches.sort();

        if matches.is_empty() {
            Ok(ToolResult::success("No files found matching pattern."))
        } else {
            let count = matches.len();
            let list = matches.join("\n");
            Ok(ToolResult::success(format!(
                "Found {count} files:\n{list}"
            )))
        }
    }
}

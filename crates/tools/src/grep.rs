use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a regex pattern in files. Returns matching lines with file paths and line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (defaults to working directory)"
                },
                "include": {
                    "type": "string",
                    "description": "File glob to filter (e.g. '*.rs')"
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let pattern_str = input["pattern"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'pattern' parameter".into()))?;

        let search_path = match input["path"].as_str() {
            Some(p) => crate::resolve_path(p, &ctx.working_dir)?,
            None => ctx.working_dir.clone(),
        };

        let include = input["include"].as_str();

        debug!("Grep search: {pattern_str} in {}", search_path.display());

        let re = regex::Regex::new(pattern_str)
            .map_err(|e| RustyError::Tool(format!("Invalid regex: {e}")))?;

        let mut results = Vec::new();

        if search_path.is_file() {
            search_file(&search_path, &re, &ctx.working_dir, &mut results);
        } else {
            let walker = walkdir::WalkDir::new(&search_path)
                .follow_links(false)
                .into_iter();

            for entry in walker.filter_entry(|e| {
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

                // Skip binary-like files by extension
                if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                    if matches!(
                        ext,
                        "exe" | "dll" | "so" | "dylib" | "bin" | "o" | "a" | "png" | "jpg" | "gif" | "pdf" | "zip" | "tar" | "gz"
                    ) {
                        continue;
                    }
                }

                // Filter by include pattern
                if let Some(inc) = include {
                    let file_name = entry.file_name().to_string_lossy();
                    if let Ok(pat) = glob::Pattern::new(inc) {
                        if !pat.matches(&file_name) {
                            continue;
                        }
                    }
                }

                search_file(entry.path(), &re, &ctx.working_dir, &mut results);

                // Cap results
                if results.len() >= 200 {
                    results.push("... (results truncated)".to_string());
                    break;
                }
            }
        }

        if results.is_empty() {
            Ok(ToolResult::success("No matches found."))
        } else {
            let count = results.len();
            let output = results.join("\n");
            Ok(ToolResult::success(format!(
                "{count} matches:\n{output}"
            )))
        }
    }
}

fn search_file(path: &std::path::Path, re: &regex::Regex, base_dir: &std::path::Path, results: &mut Vec<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // Skip unreadable files
    };

    let relative = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .display()
        .to_string();

    for (line_num, line) in content.lines().enumerate() {
        if re.find(line).is_some() {
            results.push(format!("{}:{}: {}", relative, line_num + 1, line.trim()));
        }
    }
}

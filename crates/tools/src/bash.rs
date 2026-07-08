// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

/// Maximum characters for bash output before truncation.
/// Truncation happens at line boundaries to keep output readable.
const MAX_BASH_OUTPUT: usize = 10_000;

/// Truncate text at a line boundary, keeping the first `max_chars` characters
/// worth of complete lines. Appends a truncation notice with total size.
fn smart_truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    // Find a safe byte boundary at or before max_chars to avoid panicking
    // when max_chars lands inside a multi-byte UTF-8 character.
    let safe = text.floor_char_boundary(max_chars);
    let slice = &text[..safe];
    let cut_at = slice.rfind('\n').unwrap_or(safe);

    format!(
        "{}\n\n... (output truncated, showing {} of {} chars — {} lines omitted)",
        &text[..cut_at],
        cut_at,
        text.len(),
        text[cut_at..].lines().count(),
    )
}

/// Extract path-like tokens from a shell command string.
///
/// Returns tokens that look like file paths: absolute paths, paths starting with
/// `./` or `../`, or paths starting with `~`. Handles single and double quotes.
/// Does NOT resolve shell variables, globs, or subshell expansions — those are
/// deliberately excluded since we can't resolve them statically.
fn extract_path_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                i += 1;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                i += 1;
            }
            ' ' | '\t' | '\n' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                i += 1;
            }
            _ => {
                current.push(chars[i]);
                i += 1;
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    // Filter to path-like tokens
    tokens
        .into_iter()
        .filter(|t| {
            t.starts_with('/')
                || t.starts_with("./")
                || t.starts_with("../")
                || t.starts_with('~')
        })
        .collect()
}

/// Resolve a path token against the working directory, handling `~` expansion.
fn resolve_path_token(token: &str, working_dir: &Path) -> PathBuf {
    if let Some(rest) = token.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if token == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }

    let raw = PathBuf::from(token);
    if raw.is_absolute() {
        raw
    } else {
        working_dir.join(raw)
    }
}

/// Normalize a path by resolving `.` and `..` components without requiring
/// the path to exist on disk. Unlike `canonicalize()`, this does not resolve
/// symlinks — it only collapses redundant components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                // Pop the last component if it's a normal dir (not root or parent)
                if let Some(last) = components.last() {
                    if !matches!(last, std::path::Component::RootDir | std::path::Component::ParentDir) {
                        components.pop();
                        continue;
                    }
                }
                components.push(comp);
            }
            std::path::Component::CurDir => {
                // Skip `.`
                continue;
            }
            _ => components.push(comp),
        }
    }
    components.iter().collect()
}

/// Check if a bash command references paths outside the working directory.
///
/// This is a lightweight check — it catches the most common cases:
/// - Absolute paths outside the working dir (e.g., `/etc/passwd`)
/// - `../` paths that escape the working dir
/// - Redirect targets (`>`, `>>`) pointing outside the working dir
/// - `~` paths that expand outside the working dir
///
/// It does NOT catch:
/// - Paths constructed via shell variables or subshells
/// - Relative paths that happen to exist outside the working dir
/// - Commands that change directory internally (`cd /other && cat file`)
///
/// Returns `Ok(())` if paths look safe, or `Err` with the offending path.
fn check_bash_paths(command: &str, working_dir: &Path) -> Result<(), RustyError> {
    let canon_working = working_dir.canonicalize().map_err(|e| {
        RustyError::Tool(format!("Cannot resolve working directory: {e}"))
    })?;

    // Check redirect targets first (>, >>, 2>, etc.)
    if let Some(target) = extract_redirect_target(command) {
        let resolved = resolve_path_token(&target, working_dir);
        let normalized = normalize_path(&resolved);
        if is_path_outside(&normalized, &canon_working) {
            return Err(RustyError::Tool(format!(
                "Access denied: redirect target '{}' is outside the working directory ({})",
                resolved.display(),
                canon_working.display()
            )));
        }
    }

    // Extract path-like tokens from the command
    let tokens = extract_path_tokens(command);

    for token in &tokens {
        let resolved = resolve_path_token(token, working_dir);
        let normalized = normalize_path(&resolved);
        if is_path_outside(&normalized, &canon_working) {
            return Err(RustyError::Tool(format!(
                "Access denied: '{}' is outside the working directory ({})",
                token,
                canon_working.display()
            )));
        }
    }

    Ok(())
}

/// Check if a normalized path resolves outside the canonical working directory.
/// Tries canonicalize first (resolves symlinks), then falls back to canonicalizing
/// the nearest existing ancestor and checking the prefix.
fn is_path_outside(normalized: &Path, canon_working: &Path) -> bool {
    // Try canonicalize first — handles symlinks and existing paths
    if let Ok(canon) = normalized.canonicalize() {
        return !canon.starts_with(canon_working);
    }

    // Path doesn't exist — walk up to find the nearest existing ancestor,
    // canonicalize that, then check if it's inside the working dir.
    let mut cursor: &Path = normalized;
    while let Some(parent) = cursor.parent() {
        if parent.as_os_str().is_empty() {
            break;
        }
        if let Ok(canon_parent) = parent.canonicalize() {
            // The parent exists and we can canonicalize it.
            // The full path would be canon_parent + remaining filename components.
            // If the canonicalized parent is outside the working dir, the path is outside.
            return !canon_parent.starts_with(canon_working);
        }
        cursor = parent;
    }

    // Couldn't resolve any ancestor — allow it (defensive fallback)
    false
}

/// Extract the target path from a shell redirect operator (>, >>, 2>, 2>>).
/// Returns None if no redirect is found.
fn extract_redirect_target(command: &str) -> Option<String> {
    let bytes = command.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'>' if !in_single_quote && !in_double_quote => {
                // Skip the > (and optional > for >>)
                let mut j = i + 1;
                if j < len && bytes[j] == b'>' {
                    j += 1;
                }
                // Skip whitespace
                while j < len && bytes[j] == b' ' {
                    j += 1;
                }
                // Extract the target path
                let mut target = String::new();
                while j < len && !matches!(bytes[j], b' ' | b'\t' | b'\n' | b'|' | b';' | b'&') {
                    if bytes[j] == b'\'' {
                        j += 1;
                        while j < len && bytes[j] != b'\'' {
                            target.push(bytes[j] as char);
                            j += 1;
                        }
                    } else if bytes[j] == b'"' {
                        j += 1;
                        while j < len && bytes[j] != b'"' {
                            target.push(bytes[j] as char);
                            j += 1;
                        }
                    } else {
                        target.push(bytes[j] as char);
                    }
                    j += 1;
                }
                if !target.is_empty() {
                    return Some(target);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use for running tests, building, git commands, and system operations. Path arguments and redirect targets are checked — commands referencing paths outside the working directory will be rejected."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'command' parameter".into()))?;

        let timeout_secs = input["timeout"].as_u64().unwrap_or(120);

        // Check for paths outside the working directory before executing.
        // Bypass mode skips this restriction — the user explicitly opted in.
        if ctx.permission_mode != rusty_core::PermissionMode::BypassPermissions {
            check_bash_paths(command, &ctx.working_dir)?;
        }

        debug!("Executing bash command: {command}");

        let output = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            output,
        )
        .await
        .map_err(|_| RustyError::Tool(format!("Command timed out after {timeout_secs}s")))?
        .map_err(|e| RustyError::Tool(format!("Failed to execute command: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);

        let mut result = String::new();
        if !stdout.is_empty() {
            result.push_str(&smart_truncate(&stdout, MAX_BASH_OUTPUT));
        }
        if !stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&format!("STDERR:\n{}", smart_truncate(&stderr, MAX_BASH_OUTPUT)));
        }
        if result.is_empty() {
            result = format!("(exit code: {exit_code})");
        }

        if exit_code != 0 {
            result.push_str(&format!("\n(exit code: {exit_code})"));
        }

        Ok(ToolResult {
            content: result,
            is_error: !output.status.success(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_path_tokens_absolute() {
        let tokens = extract_path_tokens("cat /etc/passwd");
        assert_eq!(tokens, vec!["/etc/passwd"]);
    }

    #[test]
    fn extract_path_tokens_relative() {
        let tokens = extract_path_tokens("cat ./foo/bar.txt");
        assert_eq!(tokens, vec!["./foo/bar.txt"]);
    }

    #[test]
    fn extract_path_tokens_parent() {
        let tokens = extract_path_tokens("cat ../other/file.txt");
        assert_eq!(tokens, vec!["../other/file.txt"]);
    }

    #[test]
    fn extract_path_tokens_tilde() {
        let tokens = extract_path_tokens("cat ~/Documents/file.txt");
        assert_eq!(tokens, vec!["~/Documents/file.txt"]);
    }

    #[test]
    fn extract_path_tokens_quoted() {
        let tokens = extract_path_tokens("cat '/etc/passwd'");
        assert_eq!(tokens, vec!["/etc/passwd"]);
    }

    #[test]
    fn extract_path_tokens_no_paths() {
        let tokens = extract_path_tokens("echo hello world");
        assert!(tokens.is_empty());
    }

    #[test]
    fn extract_path_tokens_multiple() {
        let tokens = extract_path_tokens("cp /tmp/a.txt /tmp/b.txt");
        assert_eq!(tokens, vec!["/tmp/a.txt", "/tmp/b.txt"]);
    }

    #[test]
    fn extract_redirect_target_simple() {
        assert_eq!(extract_redirect_target("echo hi > /tmp/out.txt"), Some("/tmp/out.txt".into()));
    }

    #[test]
    fn extract_redirect_target_append() {
        assert_eq!(extract_redirect_target("echo hi >> /tmp/out.txt"), Some("/tmp/out.txt".into()));
    }

    #[test]
    fn extract_redirect_target_stderr() {
        assert_eq!(extract_redirect_target("cmd 2> /tmp/err.txt"), Some("/tmp/err.txt".into()));
    }

    #[test]
    fn extract_redirect_target_none() {
        assert_eq!(extract_redirect_target("echo hi"), None);
    }

    #[test]
    fn check_bash_paths_allows_relative() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox");
        std::fs::create_dir_all(&wd).unwrap();
        assert!(check_bash_paths("cat ./file.txt", &wd).is_ok());
        assert!(check_bash_paths("ls src/", &wd).is_ok());
        assert!(check_bash_paths("echo hello", &wd).is_ok());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_blocks_absolute_outside() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox2");
        std::fs::create_dir_all(&wd).unwrap();
        // /etc is not inside the working dir
        assert!(check_bash_paths("cat /etc/passwd", &wd).is_err());
        assert!(check_bash_paths("rm -rf /tmp/something", &wd).is_err());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_blocks_redirect_outside() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox3");
        std::fs::create_dir_all(&wd).unwrap();
        assert!(check_bash_paths("echo hi > /etc/test.txt", &wd).is_err());
        assert!(check_bash_paths("echo hi >> /tmp/out.txt", &wd).is_err());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_allows_redirect_inside() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox4");
        std::fs::create_dir_all(&wd).unwrap();
        assert!(check_bash_paths("echo hi > ./output.txt", &wd).is_ok());
        assert!(check_bash_paths("echo hi > output.txt", &wd).is_ok());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_blocks_cd_outside() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox5");
        std::fs::create_dir_all(&wd).unwrap();
        // cd to an absolute path outside working dir
        assert!(check_bash_paths("cd /tmp && ls", &wd).is_err());
        assert!(check_bash_paths("cd /etc", &wd).is_err());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_blocks_parent_traversal() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox6");
        std::fs::create_dir_all(&wd).unwrap();
        // ../ that could escape
        assert!(check_bash_paths("cat ../../../etc/passwd", &wd).is_err());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_allows_commands_without_paths() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox7");
        std::fs::create_dir_all(&wd).unwrap();
        assert!(check_bash_paths("echo hello", &wd).is_ok());
        assert!(check_bash_paths("git status", &wd).is_ok());
        assert!(check_bash_paths("cargo build", &wd).is_ok());
        assert!(check_bash_paths("ls -la", &wd).is_ok());
        assert!(check_bash_paths("env | grep PATH", &wd).is_ok());
        std::fs::remove_dir_all(&wd).ok();
    }

    #[test]
    fn check_bash_paths_allows_subdir_access() {
        let wd = std::env::temp_dir().join("rusty_test_bash_sandbox8");
        std::fs::create_dir_all(&wd).unwrap();
        // Accessing subdirectories of the working dir is fine
        assert!(check_bash_paths("cat ./src/main.rs", &wd).is_ok());
        assert!(check_bash_paths("ls ./crates/", &wd).is_ok());
        std::fs::remove_dir_all(&wd).ok();
    }
}

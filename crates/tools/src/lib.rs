// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod apply_patch;
pub mod agent;
pub mod background;
pub mod bash;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod memory;
pub mod note;
pub mod plan_mode;
pub mod question;
pub mod task_output;
pub mod task_stop;
pub mod todowrite;
pub mod web_fetch;

use async_trait::async_trait;
use rusty_core::{CancelToken, PermissionLevel, RustyError, ToolDefinition};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Callback for the question tool: receives (header, full_prompt), returns user's answer.
pub type QuestionCallback = std::sync::Arc<
    dyn Fn(&str, &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub permission_mode: rusty_core::PermissionMode,
    pub cancel: Option<CancelToken>,
    pub on_question: Option<QuestionCallback>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("working_dir", &self.working_dir)
            .field("permission_mode", &self.permission_mode)
            .field("cancel", &self.cancel)
            .field("on_question", &self.on_question.is_some())
            .finish()
    }
}

/// Resolve a path against the working directory and validate it stays within the sandbox.
/// Returns the canonicalized absolute path, or an error if it escapes the working directory.
///
/// **Security note:** This function avoids `path.exists()` before `canonicalize()` to
/// eliminate a TOCTOU (time-of-check-time-of-use) race window. Instead, it attempts
/// `canonicalize()` directly and falls back to parent-only canonicalization if the
/// file doesn't exist yet.
pub fn resolve_path(path_str: &str, working_dir: &Path) -> Result<PathBuf, RustyError> {
    let raw = PathBuf::from(path_str);
    let joined = if raw.is_absolute() {
        raw
    } else {
        working_dir.join(raw)
    };

    // Attempt canonicalization directly — no preceding exists() check.
    // This eliminates the TOCTOU window where a symlink could be created
    // between the check and the canonicalize call.
    let canonical = match joined.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // File doesn't exist yet — canonicalize the parent and append filename.
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
        }
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

/// Pre-operation validation: verify that `path` is not a symlink pointing outside
/// the sandbox. Uses `symlink_metadata` (does not follow symlinks) to detect symlinks,
/// then resolves and checks the target if one is found.
///
/// Call this **before** any write or delete operation to prevent TOCTOU attacks where
/// a symlink is created between `resolve_path` and the actual I/O.
pub fn verify_not_escaping_symlink(path: &Path, working_dir: &Path) -> Result<(), RustyError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()), // Path doesn't exist yet, nothing to check
    };

    if !meta.is_symlink() {
        return Ok(());
    }

    // Path is a symlink — resolve it and check the target
    let resolved = path.canonicalize().map_err(|e| {
        RustyError::Tool(format!(
            "Cannot resolve symlink at '{}': {e}",
            path.display()
        ))
    })?;
    let canon_working = working_dir.canonicalize().map_err(|e| {
        RustyError::Tool(format!("Cannot resolve working directory: {e}"))
    })?;

    if !resolved.starts_with(&canon_working) {
        return Err(RustyError::Tool(format!(
            "SECURITY: path '{}' is a symlink pointing to '{}' which is outside the working directory ({}). \
             Operation blocked.",
            path.display(),
            resolved.display(),
            canon_working.display()
        )));
    }

    Ok(())
}

/// Post-write validation: verify the file at `path` hasn't been replaced by a symlink
/// that points outside the sandbox.
///
/// Call this **after** a file write/edit/patch completes. If a TOCTOU attacker swapped the
/// file for a symlink between resolution and write, this catches it by re-resolving the
/// path and confirming it still lands within `working_dir`.
///
/// Returns `Ok(canonical_path)` if safe, or `Err` if the resolved path escapes the sandbox.
pub fn verify_no_symlink_escape(path: &Path, working_dir: &Path) -> Result<PathBuf, RustyError> {
    // Re-canonicalize — this now follows any symlinks that may have been created.
    let resolved = path.canonicalize().map_err(|e| {
        RustyError::Tool(format!(
            "Post-write verification failed: cannot canonicalize '{}': {e}",
            path.display()
        ))
    })?;

    let canon_working = working_dir.canonicalize().map_err(|e| {
        RustyError::Tool(format!("Cannot resolve working directory: {e}"))
    })?;

    if !resolved.starts_with(&canon_working) {
        return Err(RustyError::Tool(format!(
            "SECURITY: file at '{}' resolved to '{}' which is outside the working directory ({}). \
             The file may have been replaced by a symlink. Write has been completed but the \
             path should be investigated.",
            path.display(),
            resolved.display(),
            canon_working.display()
        )));
    }

    Ok(resolved)
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

/// Returns all built-in tools (excluding AgentTool, MemoryTool, and TodoWriteTool which require special construction)
pub fn all_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(apply_patch::ApplyPatchTool),
        Box::new(bash::BashTool),
        Box::new(file_read::FileReadTool),
        Box::new(file_edit::FileEditTool),
        Box::new(file_write::FileWriteTool),
        Box::new(glob::GlobTool),
        Box::new(grep::GrepTool),
        Box::new(web_fetch::WebFetchTool::new()),
        Box::new(plan_mode::EnterPlanModeTool),
        Box::new(plan_mode::ExitPlanModeTool),
        Box::new(question::QuestionTool),
    ]
}

/// Returns read-only tools suitable for explore subagents.
/// These tools cannot modify the filesystem or execute commands.
pub fn explore_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(file_read::FileReadTool),
        Box::new(glob::GlobTool),
        Box::new(grep::GrepTool),
        Box::new(web_fetch::WebFetchTool::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temp dir, run a test closure, auto-cleanup.
    fn with_temp_dir(f: impl FnOnce(&Path)) {
        let dir = tempfile::tempdir().expect("tempdir");
        f(dir.path());
    }

    // ── resolve_path: basic cases ────────────────────────────────────

    #[test]
    fn resolve_relative_path_inside_sandbox() {
        with_temp_dir(|wd| {
            let result = resolve_path("foo.txt", wd);
            assert!(result.is_ok());
            let p = result.unwrap();
            assert!(p.starts_with(wd.canonicalize().unwrap()));
            assert!(p.ends_with("foo.txt"));
        });
    }

    #[test]
    fn resolve_nested_relative_path_inside_sandbox() {
        with_temp_dir(|wd| {
            // Create nested dirs so canonicalize can resolve parent
            fs::create_dir_all(wd.join("sub/dir")).unwrap();
            let result = resolve_path("sub/dir/file.rs", wd);
            assert!(result.is_ok());
            let p = result.unwrap();
            assert!(p.ends_with("file.rs"));
        });
    }

    #[test]
    fn resolve_absolute_path_inside_sandbox() {
        with_temp_dir(|wd| {
            let abs = wd.join("existing.txt");
            fs::write(&abs, "hello").unwrap();
            let result = resolve_path(abs.to_str().unwrap(), wd);
            assert!(result.is_ok());
        });
    }

    #[test]
    fn resolve_path_dot_slash_is_normalized() {
        with_temp_dir(|wd| {
            let result = resolve_path("./foo.txt", wd);
            assert!(result.is_ok());
            // Should not contain `.` components after canonicalization
            let p = result.unwrap();
            assert!(!p.to_string_lossy().contains("/./"));
        });
    }

    // ── resolve_path: sandbox escape attempts ────────────────────────

    #[test]
    fn resolve_dot_dot_escape_rejected() {
        with_temp_dir(|wd| {
            // Attempt to escape via ../
            let result = resolve_path("../outside.txt", wd);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(msg.contains("outside the working directory"));
        });
    }

    #[test]
    fn resolve_deep_dot_dot_escape_rejected() {
        with_temp_dir(|wd| {
            fs::create_dir_all(wd.join("sub")).unwrap();
            let result = resolve_path("sub/../../outside.txt", wd);
            assert!(result.is_err());
        });
    }

    // ── resolve_path: existing vs. new file ──────────────────────────

    #[test]
    fn resolve_existing_file_returns_canonical() {
        with_temp_dir(|wd| {
            let target = wd.join("real.txt");
            fs::write(&target, "content").unwrap();
            let result = resolve_path("real.txt", wd).unwrap();
            assert_eq!(result, target.canonicalize().unwrap());
        });
    }

    #[test]
    fn resolve_new_file_in_existing_dir() {
        with_temp_dir(|wd| {
            // Parent dir exists (wd itself), file doesn't yet
            let result = resolve_path("not_yet_created.txt", wd);
            assert!(result.is_ok());
            let p = result.unwrap();
            assert!(p.ends_with("not_yet_created.txt"));
            assert!(p.starts_with(wd.canonicalize().unwrap()));
        });
    }

    // ── resolve_path: symlink escape attempts ────────────────────────

    #[test]
    #[cfg(unix)]
    fn resolve_symlink_escape_rejected() {
        with_temp_dir(|wd| {
            // Create a target file outside the working directory
            let outside = tempfile::NamedTempFile::new().expect("temp file outside sandbox");
            fs::write(outside.path(), "escaped!").unwrap();

            // Create a symlink inside the working directory pointing outside
            let link = wd.join("sneaky_link");
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();

            let result = resolve_path("sneaky_link", wd);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("outside the working directory"),
                "Expected 'outside' error, got: {msg}"
            );
        });
    }

    // ── ToolResult construction ──────────────────────────────────────

    #[test]
    fn tool_result_success() {
        let r = ToolResult::success("all good");
        assert_eq!(r.content, "all good");
        assert!(!r.is_error);
    }

    #[test]
    fn tool_result_error() {
        let r = ToolResult::error("bad input");
        assert_eq!(r.content, "bad input");
        assert!(r.is_error);
    }
}
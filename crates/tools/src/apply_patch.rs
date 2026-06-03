// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use rusty_core::{PermissionLevel, RustyError};
use serde_json::{json, Value};
use tracing::debug;

use crate::{Tool, ToolContext, ToolResult};

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to create, update, or delete files. \
         Supports Claude Code-style patch format with fuzzy matching for context lines. \
         Use this for precise, multi-file edits in a single operation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "The patch content in unified diff format. \
                        Use *** Begin Patch / *** End Patch delimiters with \
                        *** Add File, *** Update File, or *** Delete File sections."
                }
            },
            "required": ["patch"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, RustyError> {
        let patch = input["patch"]
            .as_str()
            .ok_or_else(|| RustyError::Tool("Missing 'patch' parameter".into()))?;

        debug!("Applying patch ({} chars)", patch.len());

        let operations = parse_patch(patch)?;
        if operations.is_empty() {
            return Ok(ToolResult::error("Patch contains no operations"));
        }

        let mut results = Vec::new();

        for op in &operations {
            match op {
                PatchOp::AddFile { path, content } => {
                    let full_path = crate::resolve_path(path, &ctx.working_dir)?;
                    // Pre-write check: ensure no escaping symlink exists at this path
                    crate::verify_not_escaping_symlink(&full_path, &ctx.working_dir)?;
                    if let Some(parent) = full_path.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|e| {
                                RustyError::Tool(format!(
                                    "Failed to create directory for {}: {e}",
                                    full_path.display()
                                ))
                            })?;
                    }
                    if full_path.exists() {
                        return Ok(ToolResult::error(format!(
                            "Cannot add file {}: already exists",
                            full_path.display()
                        )));
                    }
                    tokio::fs::write(&full_path, content).await.map_err(|e| {
                        RustyError::Tool(format!("Failed to write {}: {e}", full_path.display()))
                    })?;
                    // Post-write check (defense in depth)
                    crate::verify_no_symlink_escape(&full_path, &ctx.working_dir)?;
                    results.push(format!("  Added {}", path));
                }
                PatchOp::UpdateFile { path, hunks } => {
                    let full_path = crate::resolve_path(path, &ctx.working_dir)?;
                    // Pre-read check: ensure path is not an escaping symlink
                    crate::verify_not_escaping_symlink(&full_path, &ctx.working_dir)?;
                    let original = tokio::fs::read_to_string(&full_path).await.map_err(|e| {
                        RustyError::Tool(format!(
                            "Failed to read {}: {e}",
                            full_path.display()
                        ))
                    })?;

                    let new_content = apply_hunks(&original, hunks).map_err(|e| {
                        RustyError::Tool(format!("Patch failed for {}: {e}", path))
                    })?;

                    tokio::fs::write(&full_path, &new_content).await.map_err(|e| {
                        RustyError::Tool(format!("Failed to write {}: {e}", full_path.display()))
                    })?;

                    // Post-write check (defense in depth)
                    crate::verify_no_symlink_escape(&full_path, &ctx.working_dir)?;

                    let diff = similar::TextDiff::from_lines(&original, &new_content);
                    let changes = diff
                        .iter_all_changes()
                        .filter(|c| c.tag() != similar::ChangeTag::Equal)
                        .count();
                    results.push(format!("  Updated {} ({} line changes)", path, changes));
                }
                PatchOp::DeleteFile { path } => {
                    let full_path = crate::resolve_path(path, &ctx.working_dir)?;
                    if !full_path.exists() {
                        return Ok(ToolResult::error(format!(
                            "Cannot delete file {}: not found",
                            full_path.display()
                        )));
                    }
                    // Pre-delete check: prevent deleting through an escaping symlink
                    crate::verify_not_escaping_symlink(&full_path, &ctx.working_dir)?;
                    tokio::fs::remove_file(&full_path).await.map_err(|e| {
                        RustyError::Tool(format!(
                            "Failed to delete {}: {e}",
                            full_path.display()
                        ))
                    })?;
                    results.push(format!("  Deleted {}", path));
                }
            }
        }

        let summary = format!(
            "Patch applied ({} operation{}):\n{}",
            operations.len(),
            if operations.len() == 1 { "" } else { "s" },
            results.join("\n")
        );
        Ok(ToolResult::success(summary))
    }
}

// ── Patch Parsing ────────────────────────────────────────────────────────────

#[allow(clippy::enum_variant_names)]
enum PatchOp {
    AddFile {
        path: String,
        content: String,
    },
    UpdateFile {
        path: String,
        hunks: Vec<Hunk>,
    },
    DeleteFile {
        path: String,
    },
}

struct Hunk {
    /// 1-based line number in the original file where this hunk applies
    start_line: usize,
    /// Context and change lines from the patch
    lines: Vec<HunkLine>,
}

enum HunkLine {
    /// Space-prefixed context line
    Context(String),
    /// '-' prefixed deletion
    Delete(String),
    /// '+' prefixed insertion
    Insert(String),
}

fn parse_patch(patch: &str) -> Result<Vec<PatchOp>, RustyError> {
    let trimmed = patch.trim();

    // Support both *** Begin/End Patch wrapper and raw sections
    let body = if trimmed.starts_with("*** Begin Patch") {
        trimmed
            .strip_prefix("*** Begin Patch")
            .unwrap()
            .strip_suffix("*** End Patch")
            .unwrap_or_else(|| {
                trimmed
                    .strip_prefix("*** Begin Patch")
                    .unwrap()
                    .trim_end()
            })
            .trim()
    } else {
        trimmed
    };

    let mut ops = Vec::new();
    let mut current_op: Option<OpBuilder> = None;

    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("*** Add File:") {
            // Flush previous op
            if let Some(op) = current_op.take() {
                ops.push(op.build()?);
            }
            current_op = Some(OpBuilder::AddFile {
                path: rest.trim().to_string(),
                lines: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("*** Update File:") {
            if let Some(op) = current_op.take() {
                ops.push(op.build()?);
            }
            current_op = Some(OpBuilder::UpdateFile {
                path: rest.trim().to_string(),
                hunks: Vec::new(),
                current_hunk: None,
            });
        } else if let Some(rest) = line.strip_prefix("*** Delete File:") {
            if let Some(op) = current_op.take() {
                ops.push(op.build()?);
            }
            current_op = Some(OpBuilder::DeleteFile {
                path: rest.trim().to_string(),
            });
        } else if line.starts_with("@@") {
            // Hunk header: @@ -old_start,old_count +new_start,new_count @@
            if let Some(OpBuilder::UpdateFile {
                ref mut current_hunk,
                ref mut hunks,
                ..
            }) = current_op
            {
                // Flush previous hunk
                if let Some(h) = current_hunk.take() {
                    hunks.push(h.build());
                }
                let start = parse_hunk_header(line)?;
                *current_hunk = Some(HunkBuilder {
                    start_line: start,
                    lines: Vec::new(),
                });
            }
        } else if let Some(rest) = line.strip_prefix('+') {
            if let Some(ref mut op) = current_op {
                match op {
                    OpBuilder::AddFile { lines, .. } => {
                        lines.push(rest.to_string());
                    }
                    OpBuilder::UpdateFile {
                        current_hunk: Some(h),
                        ..
                    } => {
                        h.lines.push(HunkLine::Insert(rest.to_string()));
                    }
                    _ => {}
                }
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if let Some(OpBuilder::UpdateFile {
                current_hunk: Some(h),
                ..
            }) = &mut current_op
            {
                h.lines.push(HunkLine::Delete(rest.to_string()));
            }
        } else if line.starts_with(' ') || line.is_empty() {
            // Context line (space-prefixed or empty line in hunk)
            let content = line.strip_prefix(' ').unwrap_or_default();
            match &mut current_op {
                Some(OpBuilder::UpdateFile {
                    current_hunk: Some(h),
                    ..
                }) => {
                    h.lines.push(HunkLine::Context(content.to_string()));
                }
                Some(OpBuilder::AddFile { lines, .. }) => {
                    lines.push(content.to_string());
                }
                _ => {}
            }
        }
        // Skip unknown lines (headers, comments, etc.)
    }

    // Flush remaining state
    if let Some(OpBuilder::UpdateFile {
        current_hunk,
        hunks,
        ..
    }) = &mut current_op
    {
        if let Some(h) = current_hunk.take() {
            hunks.push(h.build());
        }
    }
    if let Some(op) = current_op.take() {
        ops.push(op.build()?);
    }

    Ok(ops)
}

#[allow(clippy::enum_variant_names)]
enum OpBuilder {
    AddFile {
        path: String,
        lines: Vec<String>,
    },
    UpdateFile {
        path: String,
        hunks: Vec<Hunk>,
        current_hunk: Option<HunkBuilder>,
    },
    DeleteFile {
        path: String,
    },
}

struct HunkBuilder {
    start_line: usize,
    lines: Vec<HunkLine>,
}

impl HunkBuilder {
    fn build(self) -> Hunk {
        Hunk {
            start_line: self.start_line,
            lines: self.lines,
        }
    }
}

impl OpBuilder {
    fn build(self) -> Result<PatchOp, RustyError> {
        match self {
            OpBuilder::AddFile { path, lines } => Ok(PatchOp::AddFile {
                path,
                content: lines.join("\n"),
            }),
            OpBuilder::UpdateFile {
                path,
                mut hunks,
                current_hunk,
            } => {
                if let Some(h) = current_hunk {
                    hunks.push(Hunk {
                        start_line: h.start_line,
                        lines: h.lines,
                    });
                }
                if hunks.is_empty() {
                    return Err(RustyError::Tool(format!(
                        "Update File '{path}' has no hunks"
                    )));
                }
                Ok(PatchOp::UpdateFile { path, hunks })
            }
            OpBuilder::DeleteFile { path } => Ok(PatchOp::DeleteFile { path }),
        }
    }
}

/// Parse `@@ -start,count +start,count @@` and return the old start line (1-based).
fn parse_hunk_header(line: &str) -> Result<usize, RustyError> {
    // Expected format: @@ -N,M +N,M @@
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(RustyError::Tool(format!("Invalid hunk header: {line}")));
    }
    let old_range = parts[1]
        .strip_prefix('-')
        .ok_or_else(|| RustyError::Tool(format!("Invalid hunk header: {line}")))?;
    let start_str = old_range.split(',').next().unwrap_or("1");
    let start: usize = start_str
        .parse()
        .map_err(|_| RustyError::Tool(format!("Invalid hunk start line: {start_str}")))?;
    Ok(start)
}

// ── Hunk Application with Fuzzy Matching ─────────────────────────────────────

/// Maximum lines to search for a fuzzy match
const FUZZY_OFFSET: usize = 3;

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, RustyError> {
    let mut lines: Vec<String> = original.lines().map(|s| s.to_string()).collect();
    let mut offset: isize = 0; // cumulative offset from previous hunk insertions/deletions

    for hunk in hunks {
        let adjusted_start = hunk.start_line as isize + offset - 1; // 0-based

        // Try to find the best match position
        let actual_start = find_hunk_start(&lines, hunk, adjusted_start)?;

        // Apply the hunk
        let mut pos = actual_start;
        for hunk_line in &hunk.lines {
            match hunk_line {
                HunkLine::Context(expected) => {
                    if pos >= lines.len() {
                        return Err(RustyError::Tool(format!(
                            "Hunk context extends beyond end of file at line {}",
                            pos + 1
                        )));
                    }
                    // Normalize whitespace for comparison
                    if normalize_ws(&lines[pos]) != normalize_ws(expected) {
                        // Try to find the expected line nearby
                        let found = find_nearby(&lines, expected, pos, FUZZY_OFFSET);
                        match found {
                            Some(new_pos) => {
                                // Adjust for the shift
                                pos = new_pos;
                            }
                            None => {
                                return Err(RustyError::Tool(format!(
                                    "Context mismatch at line {}: expected {:?}, got {:?}",
                                    pos + 1,
                                    expected,
                                    lines[pos]
                                )));
                            }
                        }
                    }
                    pos += 1;
                }
                HunkLine::Delete(expected) => {
                    if pos >= lines.len() {
                        return Err(RustyError::Tool(format!(
                            "Hunk delete extends beyond end of file at line {}",
                            pos + 1
                        )));
                    }
                    // Verify the line matches (with fuzzy tolerance)
                    if normalize_ws(&lines[pos]) != normalize_ws(expected) {
                        let found = find_nearby(&lines, expected, pos, FUZZY_OFFSET);
                        if let Some(new_pos) = found {
                            // Delete intervening lines too? No — just note the shift
                            pos = new_pos;
                        } else {
                            return Err(RustyError::Tool(format!(
                                "Delete mismatch at line {}: expected {:?}, got {:?}",
                                pos + 1,
                                expected,
                                lines[pos]
                            )));
                        }
                    }
                    lines.remove(pos);
                    offset -= 1;
                }
                HunkLine::Insert(content) => {
                    lines.insert(pos, content.clone());
                    pos += 1;
                    offset += 1;
                }
            }
        }
    }

    Ok(lines.join("\n"))
}

/// Find the best starting position for a hunk by matching its context/delete lines.
fn find_hunk_start(
    lines: &[String],
    hunk: &Hunk,
    preferred: isize,
) -> Result<usize, RustyError> {
    // Collect the "pattern" lines (context + delete) from the hunk
    let pattern: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| match l {
            HunkLine::Context(s) | HunkLine::Delete(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();

    if pattern.is_empty() {
        // Pure insertion hunk — use preferred position
        let pos = preferred.max(0) as usize;
        return Ok(pos.min(lines.len()));
    }

    // Try the preferred position first
    let pref = preferred.max(0) as usize;
    if pref + pattern.len() <= lines.len()
        && pattern_matches(lines, &pattern, pref)
    {
        return Ok(pref);
    }

    // Search nearby (±FUZZY_OFFSET * 2)
    let search_range = FUZZY_OFFSET * 4;
    for offset in 1..=search_range {
        for &delta in &[offset as isize, -(offset as isize)] {
            let candidate = (preferred + delta).max(0) as usize;
            if candidate + pattern.len() <= lines.len()
                && pattern_matches(lines, &pattern, candidate)
            {
                return Ok(candidate);
            }
        }
    }

    // Last resort: scan the entire file
    for candidate in 0..lines.len().saturating_sub(pattern.len()).saturating_add(1) {
        if pattern_matches(lines, &pattern, candidate) {
            return Ok(candidate);
        }
    }

    Err(RustyError::Tool(format!(
        "Could not find matching context for hunk starting at line {}",
        preferred + 1
    )))
}

/// Check if the pattern matches at the given position (with whitespace normalization).
fn pattern_matches(lines: &[String], pattern: &[&str], start: usize) -> bool {
    for (i, &expected) in pattern.iter().enumerate() {
        if start + i >= lines.len() {
            return false;
        }
        if normalize_ws(&lines[start + i]) != normalize_ws(expected) {
            return false;
        }
    }
    true
}

/// Find a matching line nearby the expected position.
fn find_nearby(lines: &[String], expected: &str, pos: usize, range: usize) -> Option<usize> {
    let expected_norm = normalize_ws(expected);
    for delta in 0..=range {
        for &candidate in &[pos.wrapping_add(delta), pos.wrapping_sub(delta)] {
            if candidate < lines.len() && normalize_ws(&lines[candidate]) == expected_norm {
                return Some(candidate);
            }
        }
    }
    None
}

/// Normalize whitespace: collapse runs of whitespace to single space, trim.
fn normalize_ws(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space && !result.is_empty() {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(ch);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_add_file() {
        let patch = "\
*** Begin Patch
*** Add File: src/new.rs
+fn main() {
+    println!(\"hello\");
+}
*** End Patch";

        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            PatchOp::AddFile { path, content } => {
                assert_eq!(path, "src/new.rs");
                assert!(content.contains("fn main()"));
            }
            _ => panic!("Expected AddFile"),
        }
    }

    #[test]
    fn test_parse_update_file() {
        let patch = "\
*** Begin Patch
*** Update File: src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
 }
*** End Patch";

        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            PatchOp::UpdateFile { path, hunks } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].start_line, 1);
            }
            _ => panic!("Expected UpdateFile"),
        }
    }

    #[test]
    fn test_parse_delete_file() {
        let patch = "\
*** Begin Patch
*** Delete File: src/old.rs
*** End Patch";

        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            PatchOp::DeleteFile { path } => {
                assert_eq!(path, "src/old.rs");
            }
            _ => panic!("Expected DeleteFile"),
        }
    }

    #[test]
    fn test_parse_multiple_ops() {
        let patch = "\
*** Begin Patch
*** Add File: a.rs
+line 1
*** Update File: b.rs
@@ -1,1 +1,1 @@
-old
+new
*** Delete File: c.rs
*** End Patch";

        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 3);
    }

    #[test]
    fn test_apply_hunks_simple() {
        let original = "line 1\nline 2\nline 3";
        let hunks = vec![Hunk {
            start_line: 2,
            lines: vec![
                HunkLine::Context("line 2".to_string()),
                HunkLine::Delete("line 3".to_string()),
                HunkLine::Insert("line 3 modified".to_string()),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line 1\nline 2\nline 3 modified");
    }

    #[test]
    fn test_apply_hunks_insert_only() {
        let original = "line 1\nline 3";
        let hunks = vec![Hunk {
            start_line: 2,
            lines: vec![
                HunkLine::Insert("line 2".to_string()),
                HunkLine::Context("line 3".to_string()),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_fuzzy_matching() {
        let original = "aaa\nbbb\nccc\nddd\neee";
        let hunks = vec![Hunk {
            start_line: 2, // bbb is at line 2, but we search for it
            lines: vec![
                HunkLine::Context("bbb".to_string()),
                HunkLine::Delete("ccc".to_string()),
                HunkLine::Insert("CCC".to_string()),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "aaa\nbbb\nCCC\nddd\neee");
    }

    #[test]
    fn test_normalize_ws() {
        assert_eq!(normalize_ws("  hello   world  "), "hello world");
        assert_eq!(normalize_ws("no change"), "no change");
        assert_eq!(normalize_ws(""), "");
        assert_eq!(normalize_ws("   "), "");
    }

    #[test]
    fn test_parse_hunk_header() {
        assert_eq!(parse_hunk_header("@@ -10,5 +10,7 @@").unwrap(), 10);
        assert_eq!(parse_hunk_header("@@ -1,3 +1,3 @@").unwrap(), 1);
    }

    #[test]
    fn test_raw_patch_without_wrapper() {
        let patch = "\
*** Add File: test.rs
+hello
*** Delete File: old.rs";

        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 2);
    }
}

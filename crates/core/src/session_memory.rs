// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::config::{ensure_restricted_dir, set_restrictive_file_permissions};
use crate::memory::{find_git_root, slugify_path};

/// A single extracted memory from a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemory {
    /// The fact/decision to remember.
    pub content: String,
    /// When this memory was created.
    pub created_at: DateTime<Utc>,
    /// ID of the session this memory was extracted from.
    pub source_session: String,
}

impl SessionMemory {
    pub fn new(content: String, source_session: String) -> Self {
        Self {
            content,
            created_at: Utc::now(),
            source_session,
        }
    }
}

/// Persistent store for session memories, keyed by project.
pub struct SessionMemoryStore {
    project_id: String,
    memories: Vec<SessionMemory>,
}

impl SessionMemoryStore {
    /// Open (or create) the memory store for a working directory.
    pub async fn for_working_dir(working_dir: &Path) -> Self {
        let project_id = Self::resolve_project_id(working_dir).await;
        let memories = Self::load(&project_id).await.unwrap_or_default();
        debug!(
            "Loaded {} session memory/ies for project {}",
            memories.len(),
            project_id
        );
        Self {
            project_id,
            memories,
        }
    }

    /// Add new memories, deduplicating against existing ones.
    pub fn add_memories(&mut self, new: Vec<SessionMemory>) {
        for mem in new {
            // Skip if we already have something very similar (simple substring check)
            if self
                .memories
                .iter()
                .any(|m| m.content.to_lowercase().contains(&mem.content.to_lowercase()))
            {
                continue;
            }
            self.memories.push(mem);
        }
        // Cap at 100 memories to prevent unbounded growth
        if self.memories.len() > 100 {
            self.memories.drain(0..self.memories.len() - 100);
        }
    }

    /// Get the most recent memories formatted for injection into a system prompt.
    /// Returns at most `count` memories, newest first.
    pub fn format_for_prompt(&self, count: usize) -> Option<String> {
        if self.memories.is_empty() {
            return None;
        }
        let mut recent: Vec<_> = self.memories.iter().rev().take(count).collect();
        recent.reverse(); // oldest first for stable ordering

        let lines: Vec<String> = recent
            .iter()
            .map(|m| format!("- {}", m.content))
            .collect();

        Some(format!(
            "## Session Memory\n\n\
            Key facts from previous sessions:\n{}\n\n\
            These facts are automatically extracted and may be incomplete.",
            lines.join("\n")
        ))
    }

    /// Persist the store to disk.
    pub async fn save(&self) -> anyhow::Result<()> {
        let path = store_path(&self.project_id);
        if let Some(parent) = path.parent() {
            tokio::task::spawn_blocking({
                let parent = parent.to_path_buf();
                move || ensure_restricted_dir(&parent)
            })
            .await??;
        }

        let mut lines = Vec::new();
        for mem in &self.memories {
            match serde_json::to_string(mem) {
                Ok(json) => lines.push(json),
                Err(e) => {
                    warn!("Failed to serialize memory: {e}");
                }
            }
        }

        let content = lines.join("\n");
        tokio::fs::write(&path, content).await?;
        tokio::task::spawn_blocking({
            let path = path.clone();
            move || set_restrictive_file_permissions(&path)
        })
        .await?;

        info!(
            "Saved {} session memory/ies for project {}",
            self.memories.len(),
            self.project_id
        );
        Ok(())
    }

    /// Load memories from disk for a project.
    async fn load(project_id: &str) -> anyhow::Result<Vec<SessionMemory>> {
        let path = store_path(project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let mut memories = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionMemory>(line) {
                Ok(mem) => memories.push(mem),
                Err(e) => warn!("Failed to parse memory line: {e}"),
            }
        }
        Ok(memories)
    }

    async fn resolve_project_id(working_dir: &Path) -> String {
        let root = find_git_root(working_dir).await;
        slugify_path(&root.unwrap_or_else(|| working_dir.to_path_buf()))
    }
}

fn store_path(project_id: &str) -> PathBuf {
    crate::config::Config::config_dir()
        .join("sessions")
        .join(format!("{project_id}_memories.jsonl"))
}

/// Extract key memories from a conversation transcript using a lightweight LLM prompt.
/// Returns a list of memory strings.
pub async fn extract_memories_from_transcript(
    transcript: &str,
    session_id: &str,
) -> Vec<SessionMemory> {
    // If the transcript is very short, there's nothing worth extracting.
    if transcript.len() < 200 {
        return Vec::new();
    }

    // For now, use a simple heuristic extraction without an LLM call.
    // This avoids adding latency at session end. We extract:
    // - Lines that look like decisions (contain "decided", "chose", "using", etc.)
    // - File operations that might indicate structure
    let mut memories = Vec::new();

    // Simple keyword-based extraction from assistant messages
    let keywords = [
        "decided to",
        "chosen",
        "using ",
        "migrated to",
        "switched to",
        "renamed",
        "replaced",
        "added dependency",
        "removed",
        "configured",
        "set up",
    ];

    for line in transcript.lines() {
        let line_lower = line.to_lowercase();
        if line.len() > 20 && line.len() < 200 {
            for kw in &keywords {
                if line_lower.contains(kw) {
                    let content = line.trim().to_string();
                    if !memories.iter().any(|m: &SessionMemory| m.content == content) {
                        memories.push(SessionMemory::new(content, session_id.to_string()));
                    }
                    break;
                }
            }
        }
    }

    // Cap at 5 memories per extraction
    memories.truncate(5);
    debug!("Extracted {} memories from session {}", memories.len(), session_id);
    memories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_for_prompt_empty() {
        let store = SessionMemoryStore {
            project_id: "test".into(),
            memories: vec![],
        };
        assert!(store.format_for_prompt(5).is_none());
    }

    #[test]
    fn format_for_prompt_basic() {
        let store = SessionMemoryStore {
            project_id: "test".into(),
            memories: vec![
                SessionMemory::new("Using Axum instead of Actix".into(), "s1".into()),
                SessionMemory::new("Postgres chosen over SQLite".into(), "s1".into()),
            ],
        };
        let prompt = store.format_for_prompt(5).unwrap();
        assert!(prompt.contains("Using Axum instead of Actix"));
        assert!(prompt.contains("Postgres chosen over SQLite"));
        assert!(prompt.contains("Session Memory"));
    }

    #[test]
    fn deduplication_works() {
        let mut store = SessionMemoryStore {
            project_id: "test".into(),
            memories: vec![SessionMemory::new("Using Axum".into(), "s1".into())],
        };
        store.add_memories(vec![SessionMemory::new("using axum".into(), "s2".into())]);
        assert_eq!(store.memories.len(), 1);
    }

    #[test]
    fn cap_at_100() {
        let mut store = SessionMemoryStore {
            project_id: "test".into(),
            memories: Vec::new(),
        };
        for i in 0..150 {
            store.memories.push(SessionMemory::new(format!("mem {i}"), "s".into()));
        }
        store.add_memories(vec![]);
        assert_eq!(store.memories.len(), 100);
    }
}

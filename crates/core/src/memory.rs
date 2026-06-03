// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum number of memories per project.
const MAX_MEMORIES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMemory {
    pub project_path: String,
    pub memories: Vec<MemoryEntry>,
}

impl ProjectMemory {
    pub fn new(project_path: String) -> Self {
        Self {
            project_path,
            memories: Vec::new(),
        }
    }

    /// Derive the project ID from a working directory by finding the git root,
    /// then slugifying the path. Falls back to the working directory itself.
    pub async fn resolve_project_id(working_dir: &Path) -> String {
        let root = find_git_root(working_dir)
            .await
            .unwrap_or_else(|| working_dir.to_path_buf());
        slugify_path(&root)
    }

    /// Load the project memory for the given working directory.
    pub async fn load_for_project(working_dir: &Path) -> anyhow::Result<Self> {
        let project_id = Self::resolve_project_id(working_dir).await;
        let path = memory_file_path(&project_id);
        if !path.exists() {
            let root = find_git_root(working_dir)
                .await
                .unwrap_or_else(|| working_dir.to_path_buf());
            return Ok(Self::new(root.to_string_lossy().to_string()));
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let memory: Self = serde_json::from_str(&content)?;
        Ok(memory)
    }

    /// Save the project memory to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let project_id = slugify_path(Path::new(&self.project_path));
        let path = memory_file_path(&project_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Add a new memory. Returns the created entry. Enforces the 100-memory cap
    /// by removing the oldest entry when full.
    pub fn add(&mut self, content: String) -> MemoryEntry {
        if self.memories.len() >= MAX_MEMORIES {
            // Remove the oldest memory (first in the list)
            self.memories.remove(0);
        }
        let entry = MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            created_at: chrono::Utc::now(),
        };
        self.memories.push(entry.clone());
        entry
    }

    /// Remove a memory by ID. Returns true if found and removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.memories.len();
        self.memories.retain(|m| m.id != id);
        self.memories.len() < before
    }

    /// Search memories by substring match (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        let lower = query.to_lowercase();
        self.memories
            .iter()
            .filter(|m| m.content.to_lowercase().contains(&lower))
            .collect()
    }

    /// Format all memories as a human-readable list for system prompt injection.
    pub fn format_for_context(&self) -> String {
        if self.memories.is_empty() {
            return String::new();
        }
        let mut out = String::from("## Project Memories\n\n");
        out.push_str(&format!(
            "The following memories have been saved for this project ({}):\n\n",
            self.memories.len()
        ));
        for m in &self.memories {
            out.push_str(&format!("- {}\n", m.content));
        }
        out
    }
}

/// Find the git root for the given directory.
async fn find_git_root(dir: &Path) -> Option<PathBuf> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// Convert a path to a safe filename slug.
/// e.g. `/Users/jeremy/Development/rusty` -> `Users_jeremy_Development_rusty`
fn slugify_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.trim_start_matches('/')
        .trim_start_matches('\\')
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_")
        .replace(' ', "_")
}

/// Get the file path for a project memory.
fn memory_file_path(project_id: &str) -> PathBuf {
    crate::Config::memory_dir().join(format!("{project_id}.json"))
}

/// Load memories for a project and return formatted context for system prompt injection.
/// Returns `Ok(None)` if there are no memories.
pub async fn load_memories_for_prompt(working_dir: &Path) -> anyhow::Result<Option<String>> {
    let memory = ProjectMemory::load_for_project(working_dir).await?;
    let ctx = memory.format_for_context();
    if ctx.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn slugify_unix_path() {
        let path = PathBuf::from("/Users/jeremy/Development/rusty");
        assert_eq!(slugify_path(&path), "Users_jeremy_Development_rusty");
    }

    #[test]
    fn slugify_strips_leading_slash() {
        let path = PathBuf::from("/tmp/work");
        assert_eq!(slugify_path(&path), "tmp_work");
    }

    #[test]
    fn slugify_handles_spaces() {
        let path = PathBuf::from("/home/user/my project");
        assert_eq!(slugify_path(&path), "home_user_my_project");
    }

    #[test]
    fn add_memory_pushes_entry() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        let entry = pm.add("remember this".into());
        assert_eq!(pm.memories.len(), 1);
        assert_eq!(pm.memories[0].content, "remember this");
        assert_eq!(pm.memories[0].id, entry.id);
    }

    #[test]
    fn add_memory_evicts_oldest_at_cap() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        for i in 0..MAX_MEMORIES {
            pm.add(format!("memory {i}"));
        }
        assert_eq!(pm.memories.len(), MAX_MEMORIES);
        let first_id = pm.memories[0].id.clone();

        pm.add("one more".into());
        assert_eq!(pm.memories.len(), MAX_MEMORIES);
        // First entry should have been evicted
        assert!(!pm.memories.iter().any(|m| m.id == first_id));
        assert_eq!(pm.memories.last().unwrap().content, "one more");
    }

    #[test]
    fn remove_memory_by_id() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        let entry = pm.add("to delete".into());
        assert_eq!(pm.memories.len(), 1);
        assert!(pm.remove(&entry.id));
        assert!(pm.memories.is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        assert!(!pm.remove("no-such-id"));
    }

    #[test]
    fn search_case_insensitive() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        pm.add("Use cargo check for validation".into());
        pm.add("Deploy to production on Fridays".into());
        pm.add("CARGO_BUILD_FLAGS set in env".into());

        let results = pm.search("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_no_match() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        pm.add("something".into());
        assert!(pm.search("nonexistent").is_empty());
    }

    #[test]
    fn format_empty_memories() {
        let pm = ProjectMemory::new("/tmp/test".into());
        assert_eq!(pm.format_for_context(), "");
    }

    #[test]
    fn format_with_memories() {
        let mut pm = ProjectMemory::new("/tmp/test".into());
        pm.add("first memory".into());
        pm.add("second memory".into());
        let output = pm.format_for_context();
        assert!(output.contains("Project Memories"));
        assert!(output.contains("- first memory"));
        assert!(output.contains("- second memory"));
        assert!(output.contains("(2)"));
    }

    #[tokio::test]
    async fn load_nonexistent_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let pm = ProjectMemory::load_for_project(dir.path()).await.unwrap();
        assert!(pm.memories.is_empty());
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut pm = ProjectMemory::new(dir.path().to_string_lossy().to_string());
        pm.add("test memory".into());
        pm.save().unwrap();

        let loaded = ProjectMemory::load_for_project(dir.path()).await.unwrap();
        assert_eq!(loaded.memories.len(), 1);
        assert_eq!(loaded.memories[0].content, "test memory");
    }

    #[test]
    fn memory_file_path_uses_config_dir() {
        let path = memory_file_path("test_project");
        assert!(path.to_string_lossy().contains(".rusty"));
        assert!(path.to_string_lossy().contains("memory"));
        assert!(path.to_string_lossy().contains("test_project.json"));
    }
}

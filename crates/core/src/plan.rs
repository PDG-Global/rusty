// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{ensure_restricted_dir, set_restrictive_file_permissions};
use crate::memory::{slugify_path, find_git_root};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanItemStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl PlanItemStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn indicator(&self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::InProgress => "[~]",
            Self::Completed => "[x]",
            Self::Cancelled => "[-]",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "in_progress" | "in progress" => Self::InProgress,
            "completed" | "done" => Self::Completed,
            "cancelled" | "canceled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanItemPriority {
    High,
    Medium,
    Low,
}

impl PlanItemPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "high" => Self::High,
            "low" => Self::Low,
            _ => Self::Medium,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub content: String,
    pub status: PlanItemStatus,
    pub priority: PlanItemPriority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub project_path: String,
    pub items: Vec<PlanItem>,
    #[serde(default)]
    pub description: Option<String>,
}

impl Plan {
    pub fn new(project_path: String) -> Self {
        Self {
            project_path,
            items: Vec::new(),
            description: None,
        }
    }

    /// Load the plan for the given working directory.
    /// Returns an empty plan if no file exists.
    pub async fn load_for_project(working_dir: &Path) -> anyhow::Result<Self> {
        let project_id = project_id_for(working_dir).await;
        let path = plan_file_path(&project_id);
        if !path.exists() {
            let root = find_git_root(working_dir)
                .await
                .unwrap_or_else(|| working_dir.to_path_buf());
            return Ok(Self::new(root.to_string_lossy().to_string()));
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let plan: Self = serde_json::from_str(&content)?;
        Ok(plan)
    }

    /// Save the plan to disk with restrictive permissions.
    pub fn save(&self) -> anyhow::Result<()> {
        let project_id = slugify_path(Path::new(&self.project_path));
        let path = plan_file_path(&project_id);
        if let Some(parent) = path.parent() {
            ensure_restricted_dir(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        set_restrictive_file_permissions(&path);
        Ok(())
    }

    /// Replace all items with a new list. Used by the todowrite tool when the model
    /// sends a full task list update.
    pub fn set_items(&mut self, items: Vec<PlanItem>) {
        self.items = items;
    }

    /// Mark an item by index with a new status.
    pub fn update_status(&mut self, index: usize, status: PlanItemStatus) -> bool {
        if let Some(item) = self.items.get_mut(index) {
            item.status = status;
            true
        } else {
            false
        }
    }

    /// Add a new item to the plan.
    pub fn add_item(&mut self, content: String, priority: PlanItemPriority) {
        self.items.push(PlanItem {
            content,
            status: PlanItemStatus::Pending,
            priority,
        });
    }

    /// Count items with incomplete status (pending or in_progress).
    pub fn incomplete_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.status == PlanItemStatus::Pending || i.status == PlanItemStatus::InProgress)
            .count()
    }

    /// Get details of incomplete items as (status_string, content) pairs.
    pub fn incomplete_details(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter(|i| i.status == PlanItemStatus::Pending || i.status == PlanItemStatus::InProgress)
            .map(|i| (i.status.as_str().to_string(), i.content.clone()))
            .collect()
    }

    /// Format the plan for injection into the system prompt.
    /// Returns an empty string if there are no items.
    pub fn format_for_system_prompt(&self) -> String {
        if self.items.is_empty() {
            return String::new();
        }

        let total = self.items.len();
        let completed = self
            .items
            .iter()
            .filter(|i| i.status == PlanItemStatus::Completed)
            .count();
        let incomplete = self.incomplete_count();

        let mut out = format!(
            "## Active Task Plan ({completed}/{total} completed, {incomplete} remaining)\n\n"
        );

        // Group by priority
        let priority_order = [
            PlanItemPriority::High,
            PlanItemPriority::Medium,
            PlanItemPriority::Low,
        ];

        for priority in &priority_order {
            let group: Vec<&PlanItem> = self
                .items
                .iter()
                .filter(|i| i.priority == *priority)
                .collect();
            if group.is_empty() {
                continue;
            }
            out.push_str(&format!("[{}]\n", priority.as_str().to_uppercase()));
            for item in &group {
                let indicator = item.status.indicator();
                out.push_str(&format!("  {indicator} {}\n", item.content));
            }
            out.push('\n');
        }

        out
    }
}

/// Resolve the project ID for a working directory.
async fn project_id_for(working_dir: &Path) -> String {
    let root = find_git_root(working_dir)
        .await
        .unwrap_or_else(|| working_dir.to_path_buf());
    slugify_path(&root)
}

/// Get the file path for a project plan.
fn plan_file_path(project_id: &str) -> PathBuf {
    crate::Config::config_dir().join("plans").join(format!("{project_id}.json"))
}

/// Load the plan for a project and return formatted context for system prompt injection.
/// Returns `Ok(None)` if the plan has no items.
pub async fn load_plan_for_prompt(working_dir: &Path) -> anyhow::Result<Option<String>> {
    let plan = Plan::load_for_project(working_dir).await?;
    let ctx = plan.format_for_system_prompt();
    if ctx.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_item_status_roundtrip() {
        for (input, expected) in [
            ("pending", PlanItemStatus::Pending),
            ("in_progress", PlanItemStatus::InProgress),
            ("completed", PlanItemStatus::Completed),
            ("done", PlanItemStatus::Completed),
            ("cancelled", PlanItemStatus::Cancelled),
            ("canceled", PlanItemStatus::Cancelled),
            ("unknown", PlanItemStatus::Pending),
        ] {
            assert_eq!(PlanItemStatus::from_str(input), expected);
        }
    }

    #[test]
    fn plan_item_priority_roundtrip() {
        for (input, expected) in [
            ("high", PlanItemPriority::High),
            ("medium", PlanItemPriority::Medium),
            ("low", PlanItemPriority::Low),
            ("unknown", PlanItemPriority::Medium),
        ] {
            assert_eq!(PlanItemPriority::from_str(input), expected);
        }
    }

    #[test]
    fn empty_plan_formats_empty() {
        let plan = Plan::new("/tmp/test".into());
        assert_eq!(plan.format_for_system_prompt(), "");
    }

    #[test]
    fn plan_format_groups_by_priority() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.add_item("Low task".into(), PlanItemPriority::Low);
        plan.add_item("High task".into(), PlanItemPriority::High);
        plan.add_item("Medium task".into(), PlanItemPriority::Medium);

        let output = plan.format_for_system_prompt();
        let high_pos = output.find("[HIGH]").unwrap();
        let med_pos = output.find("[MEDIUM]").unwrap();
        let low_pos = output.find("[LOW]").unwrap();
        assert!(high_pos < med_pos);
        assert!(med_pos < low_pos);
    }

    #[test]
    fn plan_format_shows_counts() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.add_item("Task 1".into(), PlanItemPriority::High);
        plan.add_item("Task 2".into(), PlanItemPriority::Medium);
        plan.items[0].status = PlanItemStatus::Completed;

        let output = plan.format_for_system_prompt();
        assert!(output.contains("1/2 completed"));
        assert!(output.contains("1 remaining"));
    }

    #[test]
    fn incomplete_count_filters_correctly() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.add_item("A".into(), PlanItemPriority::High);
        plan.add_item("B".into(), PlanItemPriority::High);
        plan.add_item("C".into(), PlanItemPriority::High);
        plan.items[0].status = PlanItemStatus::Completed;
        plan.items[2].status = PlanItemStatus::Cancelled;

        assert_eq!(plan.incomplete_count(), 1);
        let details = plan.incomplete_details();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].1, "B");
    }

    #[test]
    fn set_items_replaces_all() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.add_item("Old".into(), PlanItemPriority::Low);
        plan.set_items(vec![PlanItem {
            content: "New".into(),
            status: PlanItemStatus::Pending,
            priority: PlanItemPriority::High,
        }]);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].content, "New");
    }

    #[test]
    fn update_status_by_index() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.add_item("Task".into(), PlanItemPriority::Medium);
        assert!(plan.update_status(0, PlanItemStatus::Completed));
        assert_eq!(plan.items[0].status, PlanItemStatus::Completed);
    }

    #[test]
    fn update_status_out_of_bounds() {
        let mut plan = Plan::new("/tmp/test".into());
        assert!(!plan.update_status(5, PlanItemStatus::Completed));
    }

    #[test]
    fn plan_file_path_uses_config_dir() {
        let path = plan_file_path("test_project");
        let s = path.to_string_lossy();
        assert!(s.contains(".rusty"));
        assert!(s.contains("plans"));
        assert!(s.contains("test_project.json"));
    }

    #[test]
    fn plan_serialisation_roundtrip() {
        let mut plan = Plan::new("/tmp/test".into());
        plan.description = Some("Test plan".into());
        plan.add_item("First task".into(), PlanItemPriority::High);
        plan.items[0].status = PlanItemStatus::InProgress;
        plan.add_item("Second task".into(), PlanItemPriority::Low);

        let json = serde_json::to_string_pretty(&plan).unwrap();
        let loaded: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.project_path, "/tmp/test");
        assert_eq!(loaded.description, Some("Test plan".into()));
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].content, "First task");
        assert_eq!(loaded.items[0].status, PlanItemStatus::InProgress);
        assert_eq!(loaded.items[1].priority, PlanItemPriority::Low);
    }

    #[tokio::test]
    async fn load_nonexistent_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let plan = Plan::load_for_project(dir.path()).await.unwrap();
        assert!(plan.items.is_empty());
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = Plan::new(dir.path().to_string_lossy().to_string());
        plan.add_item("Test task".into(), PlanItemPriority::High);
        plan.save().unwrap();

        let loaded = Plan::load_for_project(dir.path()).await.unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].content, "Test task");
    }
}

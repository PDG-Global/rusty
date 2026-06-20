// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sidebar panel for the TUI: file tree, tool summary, and task list.

use std::path::PathBuf;

use crate::app::ToolStatus;

/// Which panel in the sidebar currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarPanel {
    Files,
    Tools,
    Tasks,
}

/// A single entry in the file tree.
#[derive(Debug, Clone)]
pub struct FileTreeEntry {
    pub path: PathBuf,
    pub display_name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub is_active: bool,
}

/// Aggregated summary of a tool type used in the session.
#[derive(Debug, Clone)]
pub struct ToolSummary {
    pub name: String,
    pub count: usize,
    pub last_status: ToolStatus,
}

/// Full sidebar state.
#[derive(Debug)]
pub struct SidebarState {
    pub visible: bool,
    pub focused_panel: SidebarPanel,
    pub files: Vec<FileTreeEntry>,
    pub tools: Vec<ToolSummary>,
    pub file_scroll: usize,
    pub tool_scroll: usize,
    pub task_scroll: usize,
    pub files_collapsed: bool,
    pub tools_collapsed: bool,
    pub tasks_collapsed: bool,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            visible: false,
            focused_panel: SidebarPanel::Files,
            files: Vec::new(),
            tools: Vec::new(),
            file_scroll: 0,
            tool_scroll: 0,
            task_scroll: 0,
            files_collapsed: false,
            tools_collapsed: false,
            tasks_collapsed: false,
        }
    }
}

impl SidebarState {
    /// Toggle sidebar visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Cycle focus: Files -> Tools -> Tasks -> Files.
    pub fn cycle_panel(&mut self) {
        self.focused_panel = match self.focused_panel {
            SidebarPanel::Files => SidebarPanel::Tools,
            SidebarPanel::Tools => SidebarPanel::Tasks,
            SidebarPanel::Tasks => SidebarPanel::Files,
        };
    }

    /// Navigate up within the focused panel.
    pub fn nav_up(&mut self) {
        match self.focused_panel {
            SidebarPanel::Files => {
                if self.file_scroll > 0 {
                    self.file_scroll -= 1;
                }
            }
            SidebarPanel::Tools => {
                if self.tool_scroll > 0 {
                    self.tool_scroll -= 1;
                }
            }
            SidebarPanel::Tasks => {
                if self.task_scroll > 0 {
                    self.task_scroll -= 1;
                }
            }
        }
    }

    /// Navigate down within the focused panel.
    pub fn nav_down(&mut self) {
        match self.focused_panel {
            SidebarPanel::Files => {
                let max = self.files.len().saturating_sub(1);
                if self.file_scroll < max {
                    self.file_scroll += 1;
                }
            }
            SidebarPanel::Tools => {
                let max = self.tools.len().saturating_sub(1);
                if self.tool_scroll < max {
                    self.tool_scroll += 1;
                }
            }
            SidebarPanel::Tasks => {
                // Task count comes from pinned_todos lines; caller handles bounds
                self.task_scroll += 1;
            }
        }
    }

    /// Toggle collapse of the focused panel header.
    pub fn toggle_collapse(&mut self) {
        match self.focused_panel {
            SidebarPanel::Files => self.files_collapsed = !self.files_collapsed,
            SidebarPanel::Tools => self.tools_collapsed = !self.tools_collapsed,
            SidebarPanel::Tasks => self.tasks_collapsed = !self.tasks_collapsed,
        }
    }

    /// Populate the file tree by walking the working directory (2 levels deep).
    pub fn populate_files(&mut self, working_dir: &str) {
        self.files.clear();
        let root = PathBuf::from(working_dir);
        if !root.is_dir() {
            return;
        }
        self.walk_dir(&root, 0, 2);
    }

    fn walk_dir(&mut self, dir: &PathBuf, depth: usize, max_depth: usize) {
        if depth > max_depth {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden dirs/files and build artifacts
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }

        dirs.sort();
        files.sort();

        for path in &dirs {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.files.push(FileTreeEntry {
                path: path.clone(),
                display_name: format!("{name}/"),
                depth,
                is_dir: true,
                is_expanded: depth < 1, // auto-expand first level
                is_active: false,
            });
            if depth < 1 {
                self.walk_dir(path, depth + 1, max_depth);
            }
        }

        for path in &files {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.files.push(FileTreeEntry {
                path: path.clone(),
                display_name: name,
                depth,
                is_dir: false,
                is_expanded: false,
                is_active: false,
            });
        }
    }

    /// Update tool summaries from completed tool blocks in messages.
    pub fn update_tools(&mut self, messages: &[crate::app::ChatMessage]) {
        use std::collections::HashMap;
        let mut counts: HashMap<String, (usize, ToolStatus)> = HashMap::new();
        for msg in messages {
            for block in &msg.tool_blocks {
                let entry = counts.entry(block.name.clone()).or_insert((0, ToolStatus::Running));
                entry.0 += 1;
                entry.1 = block.status.clone();
            }
        }
        self.tools.clear();
        for (name, (count, status)) in counts {
            self.tools.push(ToolSummary {
                name: name.clone(),
                count,
                last_status: status,
            });
        }
        self.tools.sort_by(|a, b| b.count.cmp(&a.count));
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! SQLite-backed task registry with stable hierarchical IDs, a proper state
//! machine, and an event log.  Replaces the flat JSON-file `Plan` system.
//!
//! IDs are hierarchical: `T1`, `T1.1`, `T1.2`, `T1.1.1`.  The prefix is
//! derived from the parent; siblings are queried from the DB to find the next
//! available number.
//!
//! Status lifecycle:
//!   open ⇄ in_progress → blocked → done | abandoned

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::memory::{find_git_root, slugify_path};

// ── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Open,
    InProgress,
    Blocked,
    Done,
    Abandoned,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "in_progress" | "in progress" => Self::InProgress,
            "blocked" => Self::Blocked,
            "done" | "completed" => Self::Done,
            "abandoned" | "cancelled" | "canceled" => Self::Abandoned,
            _ => Self::Open,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Abandoned)
    }

    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Open | Self::InProgress)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub session_id: String,
    pub parent_task_id: Option<String>,
    pub status: TaskStatus,
    pub summary: String,
    pub owner: Option<String>,
    pub created_at: i64,
    pub last_event_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    pub at: i64,
    pub kind: String,
    pub summary: Option<String>,
}

// ── TaskRegistry ───────────────────────────────────────────────────────────

pub struct TaskRegistry {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL,
    parent_task_id TEXT,
    status       TEXT NOT NULL DEFAULT 'open',
    summary      TEXT NOT NULL,
    owner        TEXT,
    created_at   INTEGER NOT NULL,
    last_event_at INTEGER NOT NULL,
    ended_at     INTEGER,
    cleanup_after INTEGER
);

CREATE TABLE IF NOT EXISTS task_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    session_id TEXT NOT NULL,
    at         INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    summary    TEXT
);

CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(session_id);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id);
";

impl TaskRegistry {
    /// Open (or create) a task registry at the given path.
    pub fn open(db_path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open a registry for a project (derived from working directory).
    pub fn for_project(working_dir: &Path) -> Result<Self, rusqlite::Error> {
        let project_id = project_id_for(working_dir);
        let db_path = crate::Config::config_dir()
            .join("tasks")
            .join(format!("{project_id}.db"));
        Self::open(&db_path)
    }

    /// Create a new task. Returns the assigned hierarchical ID.
    pub fn create(
        &self,
        session_id: &str,
        summary: &str,
        parent_id: Option<&str>,
        owner: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Generate hierarchical ID
        let id = next_child_id(&conn, parent_id)?;

        conn.execute(
            "INSERT INTO tasks (id, session_id, parent_task_id, status, summary, owner, created_at, last_event_at)
             VALUES (?1, ?2, ?3, 'open', ?4, ?5, ?6, ?6)",
            params![id, session_id, parent_id, summary, owner, now],
        )?;

        insert_event(&conn, &id, session_id, now, "created", None)?;

        debug!("Task {} created: {}", id, summary);

        Ok(Task {
            id,
            session_id: session_id.to_string(),
            parent_task_id: parent_id.map(|s| s.to_string()),
            status: TaskStatus::Open,
            summary: summary.to_string(),
            owner: owner.map(|s| s.to_string()),
            created_at: now,
            last_event_at: now,
            ended_at: None,
        })
    }

    /// List tasks, optionally filtered.
    pub fn list(
        &self,
        session_id: &str,
        status: Option<&str>,
        include_terminal: bool,
    ) -> Result<Vec<Task>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, session_id, parent_task_id, status, summary, owner, created_at, last_event_at, ended_at
             FROM tasks WHERE session_id = ?1",
        );

        if !include_terminal {
            sql.push_str(" AND status NOT IN ('done', 'abandoned')");
        }
        if let Some(s) = status {
            sql.push_str(&format!(" AND status = '{}'", s));
        }
        sql.push_str(" ORDER BY created_at ASC");

        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![session_id], |row| {
                Ok(Task {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_task_id: row.get(2)?,
                    status: TaskStatus::from_str(&row.get::<_, String>(3)?),
                    summary: row.get(4)?,
                    owner: row.get(5)?,
                    created_at: row.get(6)?,
                    last_event_at: row.get(7)?,
                    ended_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(tasks)
    }

    /// Get a single task by ID.
    pub fn get(&self, session_id: &str, id: &str) -> Result<Option<Task>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, parent_task_id, status, summary, owner, created_at, last_event_at, ended_at
             FROM tasks WHERE session_id = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query_map(params![session_id, id], |row| {
            Ok(Task {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_task_id: row.get(2)?,
                status: TaskStatus::from_str(&row.get::<_, String>(3)?),
                summary: row.get(4)?,
                owner: row.get(5)?,
                created_at: row.get(6)?,
                last_event_at: row.get(7)?,
                ended_at: row.get(8)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Transition a task to `in_progress`.
    pub fn start(
        &self,
        session_id: &str,
        id: &str,
        owner: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        self.transition(session_id, id, TaskStatus::InProgress, "started", owner)
    }

    /// Transition a task to `blocked`.
    pub fn block(
        &self,
        session_id: &str,
        id: &str,
        reason: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        self.transition(session_id, id, TaskStatus::Blocked, "blocked", None)
            .map(|t| {
                if let Some(r) = reason {
                    let conn = self.conn.lock().unwrap();
                    let now = chrono::Utc::now().timestamp();
                    let _ = insert_event(&conn, id, session_id, now, "blocked_reason", Some(r));
                }
                t
            })
    }

    /// Transition from `blocked` back to `open`.
    pub fn unblock(
        &self,
        session_id: &str,
        id: &str,
        reason: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        self.transition(session_id, id, TaskStatus::Open, "unblocked", None)
            .map(|t| {
                if let Some(r) = reason {
                    let conn = self.conn.lock().unwrap();
                    let now = chrono::Utc::now().timestamp();
                    let _ = insert_event(&conn, id, session_id, now, "unblock_reason", Some(r));
                }
                t
            })
    }

    /// Transition a task to `done`.
    pub fn done(
        &self,
        session_id: &str,
        id: &str,
        summary: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        let task = self.transition(session_id, id, TaskStatus::Done, "done", None)?;
        if let Some(s) = summary {
            let conn = self.conn.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            let _ = insert_event(&conn, id, session_id, now, "done_summary", Some(s));
        }
        Ok(task)
    }

    /// Transition a task to `abandoned`.
    pub fn abandon(
        &self,
        session_id: &str,
        id: &str,
        reason: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        let task = self.transition(session_id, id, TaskStatus::Abandoned, "abandoned", None)?;
        if let Some(r) = reason {
            let conn = self.conn.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            let _ = insert_event(&conn, id, session_id, now, "abandon_reason", Some(r));
        }
        Ok(task)
    }

    /// Rename a task's summary.
    pub fn rename(
        &self,
        session_id: &str,
        id: &str,
        new_summary: &str,
    ) -> Result<Task, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let updated = conn.execute(
            "UPDATE tasks SET summary = ?1, last_event_at = ?2 WHERE session_id = ?3 AND id = ?4",
            params![new_summary, now, session_id, id],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        insert_event(&conn, id, session_id, now, "renamed", Some(new_summary))?;
        drop(conn);
        self.get(session_id, id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// Get incomplete tasks as (status, content) pairs. Used by the task gate.
    pub fn incomplete_details(&self, session_id: &str) -> Vec<(String, String)> {
        let tasks = self.list(session_id, None, false).unwrap_or_default();
        tasks
            .into_iter()
            .filter(|t| t.status.is_actionable())
            .map(|t| (t.status.as_str().to_string(), t.summary.clone()))
            .collect()
    }

    /// Format tasks for system prompt injection.
    pub fn format_for_prompt(&self, session_id: &str) -> String {
        let tasks = self.list(session_id, None, true).unwrap_or_default();
        if tasks.is_empty() {
            return String::new();
        }

        let incomplete = tasks.iter().filter(|t| !t.status.is_terminal()).count();
        let completed = tasks.iter().filter(|t| t.status.is_terminal()).count();
        let total = tasks.len();

        let mut out = format!(
            "## Active Tasks ({completed}/{total} completed, {incomplete} remaining)\n\n"
        );

        for task in &tasks {
            let icon = match task.status {
                TaskStatus::Open => "🔵",
                TaskStatus::InProgress => "🔄",
                TaskStatus::Blocked => "🟡",
                TaskStatus::Done => "✅",
                TaskStatus::Abandoned => "❌",
            };
            let parent_indent = if task.parent_task_id.is_some() {
                "  "
            } else {
                ""
            };
            out.push_str(&format!(
                "{parent_indent}{icon} {} — {}\n",
                task.id, task.summary
            ));
        }

        out
    }

    /// Format tasks for todowrite tool output (what the model sees in conversation).
    pub fn render_for_tool_output(&self, session_id: &str) -> String {
        let tasks = self.list(session_id, None, true).unwrap_or_default();
        if tasks.is_empty() {
            return "No tasks.".to_string();
        }
        let lines: Vec<String> = tasks
            .iter()
            .map(|t| format!("{} ({}) — {}", t.id, t.status.as_str(), t.summary))
            .collect();
        lines.join("\n")
    }

    /// Internal state transition.
    fn transition(
        &self,
        session_id: &str,
        id: &str,
        new_status: TaskStatus,
        event_kind: &str,
        owner: Option<&str>,
    ) -> Result<Task, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Check current status
        let current: String = conn.query_row(
            "SELECT status FROM tasks WHERE session_id = ?1 AND id = ?2",
            params![session_id, id],
            |row| row.get(0),
        )?;

        let current_status = TaskStatus::from_str(&current);

        // Don't resurrect terminal tasks
        if current_status.is_terminal() {
            debug!(
                "Task {} is already {}, ignoring transition to {}",
                id,
                current_status.as_str(),
                new_status.as_str()
            );
            drop(conn);
            return self
                .get(session_id, id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows);
        }

        let ended_at = if new_status.is_terminal() {
            Some(now)
        } else {
            None::<i64>
        };

        let cleanup_after: Option<i64> = if new_status.is_terminal() {
            Some(now + 7 * 24 * 60 * 60) // 7 days
        } else {
            None
        };

        if let Some(o) = owner {
            conn.execute(
                "UPDATE tasks SET status = ?1, last_event_at = ?2, ended_at = ?3, cleanup_after = ?4, owner = ?5
                 WHERE session_id = ?6 AND id = ?7",
                params![new_status.as_str(), now, ended_at, cleanup_after, o, session_id, id],
            )?;
        } else {
            conn.execute(
                "UPDATE tasks SET status = ?1, last_event_at = ?2, ended_at = ?3, cleanup_after = ?4
                 WHERE session_id = ?5 AND id = ?6",
                params![new_status.as_str(), now, ended_at, cleanup_after, session_id, id],
            )?;
        }

        insert_event(&conn, id, session_id, now, event_kind, None)?;

        debug!("Task {} transitioned to {}", id, new_status.as_str());

        drop(conn);
        self.get(session_id, id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Generate the next child ID under a parent.
/// Root tasks: `T1`, `T2`, ...
/// Subtasks: `T1.1`, `T1.2`, ...
fn next_child_id(conn: &Connection, parent_id: Option<&str>) -> Result<String, rusqlite::Error> {
    let prefix = match parent_id {
        Some(pid) => format!("{pid}."),
        None => "T".to_string(),
    };

    let pattern = if parent_id.is_some() {
        format!("{prefix}%")
    } else {
        "T%".to_string()
    };

    // Find existing siblings
    let mut stmt = conn.prepare(
        "SELECT id FROM tasks WHERE id LIKE ?1 AND id NOT LIKE ?2",
    )?;
    // For root: LIKE 'T%' AND NOT LIKE 'T%.%'
    // For child: LIKE 'T1.%' AND NOT LIKE 'T1.%.%'
    let child_pattern = format!("{prefix}%.%");
    let siblings: Vec<String> = stmt
        .query_map(params![pattern, child_pattern], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let next_num = siblings
        .iter()
        .filter_map(|s| {
            let tail = s.strip_prefix(&prefix)?;
            tail.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
        + 1;

    Ok(format!("{prefix}{next_num}"))
}

fn insert_event(
    conn: &Connection,
    task_id: &str,
    session_id: &str,
    at: i64,
    kind: &str,
    summary: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO task_events (task_id, session_id, at, kind, summary) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task_id, session_id, at, kind, summary],
    )?;
    Ok(())
}

fn project_id_for(working_dir: &Path) -> String {
    let root = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(async { find_git_root(working_dir).await })
    })
    .unwrap_or_else(|| working_dir.to_path_buf());
    slugify_path(&root)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_registry() -> (TaskRegistry, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let reg = TaskRegistry::open(&db_path).unwrap();
        (reg, dir)
    }

    #[test]
    fn create_and_get() {
        let (reg, _dir) = make_registry();
        let task = reg.create("s1", "Implement auth", None, None).unwrap();
        assert_eq!(task.id, "T1");
        assert_eq!(task.status, TaskStatus::Open);

        let fetched = reg.get("s1", "T1").unwrap().unwrap();
        assert_eq!(fetched.summary, "Implement auth");
    }

    #[test]
    fn hierarchical_ids() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Parent", None, None).unwrap();
        let child = reg.create("s1", "Child A", Some("T1"), None).unwrap();
        assert_eq!(child.id, "T1.1");
        let child2 = reg.create("s1", "Child B", Some("T1"), None).unwrap();
        assert_eq!(child2.id, "T1.2");

        let root2 = reg.create("s1", "Second root", None, None).unwrap();
        assert_eq!(root2.id, "T2");
    }

    #[test]
    fn lifecycle_transitions() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Task", None, None).unwrap();

        let t = reg.start("s1", "T1", None).unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);

        let t = reg.block("s1", "T1", Some("waiting on dep")).unwrap();
        assert_eq!(t.status, TaskStatus::Blocked);

        let t = reg.unblock("s1", "T1", None).unwrap();
        assert_eq!(t.status, TaskStatus::Open);

        let t = reg.done("s1", "T1", Some("all tests pass")).unwrap();
        assert_eq!(t.status, TaskStatus::Done);
        assert!(t.ended_at.is_some());
    }

    #[test]
    fn terminal_tasks_not_resurrected() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Task", None, None).unwrap();
        reg.done("s1", "T1", None).unwrap();
        let t = reg.start("s1", "T1", None).unwrap();
        assert_eq!(t.status, TaskStatus::Done); // still done
    }

    #[test]
    fn abandon_task() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Task", None, None).unwrap();
        let t = reg.abandon("s1", "T1", Some("out of scope")).unwrap();
        assert_eq!(t.status, TaskStatus::Abandoned);
    }

    #[test]
    fn rename_task() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Old name", None, None).unwrap();
        let t = reg.rename("s1", "T1", "New name").unwrap();
        assert_eq!(t.summary, "New name");
    }

    #[test]
    fn list_filters_terminal() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Open task", None, None).unwrap();
        reg.create("s1", "Done task", None, None).unwrap();
        reg.done("s1", "T2", None).unwrap();

        let active = reg.list("s1", None, false).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "T1");

        let all = reg.list("s1", None, true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn incomplete_details() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Open task", None, None).unwrap();
        reg.create("s1", "In progress", None, None).unwrap();
        reg.start("s1", "T2", None).unwrap();
        reg.create("s1", "Done task", None, None).unwrap();
        reg.done("s1", "T3", None).unwrap();

        let incomplete = reg.incomplete_details("s1");
        assert_eq!(incomplete.len(), 2);
        assert_eq!(incomplete[0].0, "open");
        assert_eq!(incomplete[1].0, "in_progress");
    }

    #[test]
    fn event_log_recorded() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Task", None, None).unwrap();
        reg.start("s1", "T1", None).unwrap();
        reg.done("s1", "T1", None).unwrap();

        let conn = reg.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task_events WHERE task_id = 'T1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3); // created, started, done
    }

    #[test]
    fn format_for_prompt_empty() {
        let (reg, _dir) = make_registry();
        let output = reg.format_for_prompt("s1");
        assert!(output.is_empty());
    }

    #[test]
    fn format_for_prompt_shows_tasks() {
        let (reg, _dir) = make_registry();
        reg.create("s1", "Build feature", None, None).unwrap();
        reg.create("s1", "Add tests", None, None).unwrap();
        reg.start("s1", "T1", None).unwrap();

        let output = reg.format_for_prompt("s1");
        assert!(output.contains("🔄"));
        assert!(output.contains("T1"));
        assert!(output.contains("🔵"));
        assert!(output.contains("T2"));
    }

    #[test]
    fn render_for_tool_output() {
        let (reg, _dir) = make_registry();
        assert_eq!(reg.render_for_tool_output("s1"), "No tasks.");

        reg.create("s1", "Task A", None, None).unwrap();
        let output = reg.render_for_tool_output("s1");
        assert!(output.contains("T1 (open)"));
        assert!(output.contains("Task A"));
    }
}

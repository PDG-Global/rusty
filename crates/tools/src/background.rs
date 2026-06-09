// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Status of a background subagent task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl BackgroundTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// A tracked background subagent task.
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub description: String,
    pub status: BackgroundTaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub start_time: Instant,
}

/// Manages background subagent tasks.
#[derive(Debug, Clone)]
pub struct BackgroundManager {
    tasks: Arc<Mutex<HashMap<String, BackgroundTask>>>,
    counter: Arc<AtomicU64>,
}

impl BackgroundManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Register a new background task before it starts.
    /// Returns the assigned task_id.
    pub fn register(&self, description: String) -> String {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        let task_id = format!("task-{id:04}");
        let task = BackgroundTask {
            description,
            status: BackgroundTaskStatus::Running,
            result: None,
            error: None,
            start_time: Instant::now(),
        };
        // We can't block in a sync function, so spawn a task to insert
        let tasks = self.tasks.clone();
        let tid = task_id.clone();
        tokio::spawn(async move {
            tasks.lock().await.insert(tid, task);
        });
        task_id
    }

    /// Mark a task as completed with its result.
    pub async fn complete(&self, task_id: &str, result: String) {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = BackgroundTaskStatus::Completed;
            task.result = Some(result);
        }
    }

    /// Mark a task as failed with an error message.
    pub async fn fail(&self, task_id: &str, error: String) {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = BackgroundTaskStatus::Failed;
            task.error = Some(error);
        }
    }

    /// Mark a task as stopped.
    pub async fn stop(&self, task_id: &str) {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            if task.status == BackgroundTaskStatus::Running {
                task.status = BackgroundTaskStatus::Stopped;
            }
        }
    }

    /// Get a copy of a task by id.
    pub async fn get(&self, task_id: &str) -> Option<BackgroundTask> {
        self.tasks.lock().await.get(task_id).cloned()
    }

    /// List all tracked tasks.
    pub async fn list(&self) -> Vec<(String, BackgroundTask)> {
        self.tasks
            .lock()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Remove a task from tracking.
    pub async fn remove(&self, task_id: &str) {
        self.tasks.lock().await.remove(task_id);
    }
}

impl Default for BackgroundManager {
    fn default() -> Self {
        Self::new()
    }
}

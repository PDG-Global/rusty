// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{ensure_restricted_dir, set_restrictive_file_permissions};

use crate::Message;

/// Sessions older than this are cleaned up automatically.
const SESSION_TTL_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<Message>,
    pub model: String,
    pub working_dir: String,
}

impl ConversationSession {
    pub fn new(model: String, working_dir: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            messages: Vec::new(),
            model,
            working_dir,
        }
    }

    fn session_path(id: &str) -> PathBuf {
        crate::Config::sessions_dir().join(format!("{id}.json"))
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let dir = crate::Config::sessions_dir();
        {
            let dir = dir.clone();
            tokio::task::spawn_blocking(move || ensure_restricted_dir(&dir)).await??;
        }
        let path = Self::session_path(&self.id);
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&path, content).await?;
        {
            let path = path.clone();
            tokio::task::spawn_blocking(move || set_restrictive_file_permissions(&path)).await?;
        }
        Ok(())
    }

    pub async fn load(id: &str) -> anyhow::Result<Option<Self>> {
        let path = Self::session_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let session: Self = serde_json::from_str(&content)?;
        Ok(Some(session))
    }

    pub async fn list() -> anyhow::Result<Vec<Self>> {
        let dir = crate::Config::sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(session) = serde_json::from_str::<Self>(&content) {
                        sessions.push(session);
                    }
                }
            }
        }
        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(sessions)
    }

    /// Remove session files older than `SESSION_TTL_DAYS`. Returns the number of
    /// sessions removed. Called lazily during `list()` to avoid blocking startup.
    pub async fn cleanup_expired() -> anyhow::Result<usize> {
        let dir = crate::Config::sessions_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now() - chrono::Duration::days(SESSION_TTL_DAYS);
        let mut removed = 0usize;
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(session) = serde_json::from_str::<Self>(&content) {
                        if session.updated_at < cutoff {
                            if tokio::fs::remove_file(&path).await.is_ok() {
                                removed += 1;
                            }
                        }
                    }
                }
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ConversationSession::new ─────────────────────────────────────

    #[test]
    fn new_session_has_correct_model_and_dir() {
        let session = ConversationSession::new("test-model".into(), "/tmp/work".into());
        assert_eq!(session.model, "test-model");
        assert_eq!(session.working_dir, "/tmp/work");
        assert!(session.name.is_none());
        assert!(session.messages.is_empty());
        assert!(!session.id.is_empty());
    }

    #[test]
    fn new_session_has_valid_uuid() {
        let session = ConversationSession::new("m".into(), "d".into());
        // Should parse as a valid UUID
        assert!(uuid::Uuid::parse_str(&session.id).is_ok());
    }

    #[test]
    fn new_session_timestamps_are_recent() {
        let before = chrono::Utc::now();
        let session = ConversationSession::new("m".into(), "d".into());
        let after = chrono::Utc::now();

        assert!(session.created_at >= before);
        assert!(session.created_at <= after);
        // Allow up to 1ms difference — timestamps are set via separate Utc::now() calls
        let diff = (session.created_at - session.updated_at).num_microseconds().unwrap_or(0).abs();
        assert!(diff < 1000, "created_at and updated_at differ by {diff}μs");
    }

    #[test]
    fn new_sessions_have_unique_ids() {
        let s1 = ConversationSession::new("m".into(), "d".into());
        let s2 = ConversationSession::new("m".into(), "d".into());
        assert_ne!(s1.id, s2.id);
    }
}

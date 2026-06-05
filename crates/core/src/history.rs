// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{ensure_restricted_dir, set_restrictive_file_permissions};

use crate::Message;

/// Sessions older than this are cleaned up automatically.
const SESSION_TTL_DAYS: i64 = 30;

/// Maximum number of sessions retained per project. Oldest are evicted on save.
const MAX_SESSIONS_PER_PROJECT: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<Message>,
    pub model: String,
    /// Deprecated: kept for backward compatibility with old session files.
    /// New sessions no longer persist this field.
    #[serde(default, skip_serializing)]
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

    fn session_path(sessions_dir: &Path, id: &str) -> PathBuf {
        sessions_dir.join(format!("{id}.json"))
    }

    pub async fn save(&self, sessions_dir: &Path) -> anyhow::Result<()> {
        {
            let dir = sessions_dir.to_path_buf();
            tokio::task::spawn_blocking(move || ensure_restricted_dir(&dir)).await??;
        }
        let path = Self::session_path(sessions_dir, &self.id);
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&path, content).await?;
        {
            let path = path.clone();
            tokio::task::spawn_blocking(move || set_restrictive_file_permissions(&path)).await?;
        }
        // Evict old / excess sessions to keep storage bounded.
        Self::cleanup(sessions_dir).await;
        Ok(())
    }

    pub async fn load(sessions_dir: &Path, id: &str) -> anyhow::Result<Option<Self>> {
        let path = Self::session_path(sessions_dir, id);
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let session: Self = serde_json::from_str(&content)?;
        Ok(Some(session))
    }

    pub async fn list(sessions_dir: &Path) -> anyhow::Result<Vec<Self>> {
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(sessions_dir).await?;
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

    /// Remove expired sessions and enforce the per-project cap.
    ///
    /// Sessions older than `SESSION_TTL_DAYS` are deleted first, then the
    /// oldest sessions beyond `MAX_SESSIONS_PER_PROJECT` are evicted.
    pub async fn cleanup(sessions_dir: &Path) {
        if !sessions_dir.exists() {
            return;
        }
        let cutoff = chrono::Utc::now() - chrono::Duration::days(SESSION_TTL_DAYS);

        let mut sessions: Vec<(PathBuf, Self)> = Vec::new();
        let mut entries = match tokio::fs::read_dir(sessions_dir).await {
            Ok(e) => e,
            Err(_) => return,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(session) = serde_json::from_str::<Self>(&content) {
                        sessions.push((path, session));
                    }
                }
            }
        }

        // Sort newest-first so we keep the most recent ones.
        sessions.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));

        let mut kept = 0usize;
        for (path, session) in &sessions {
            let expired = session.updated_at < cutoff;
            let over_cap = kept >= MAX_SESSIONS_PER_PROJECT;
            if expired || over_cap {
                let _ = tokio::fs::remove_file(path).await;
            } else {
                kept += 1;
            }
        }
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

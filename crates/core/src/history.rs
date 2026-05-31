use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSession {
    pub id: String,
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
        tokio::fs::create_dir_all(&dir).await?;
        let path = Self::session_path(&self.id);
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&path, content).await?;
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
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }
}

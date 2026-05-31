use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::permissions::PermissionMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    pub permission_mode: PermissionMode,
    pub verbose: bool,
    pub max_turns: u32,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub no_claude_md: bool,
    pub auto_compact: bool,
    pub thinking_budget: Option<u32>,
    pub temperature: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            model: "mimo-v2.5-pro".to_string(),
            max_tokens: 16384,
            permission_mode: PermissionMode::Default,
            verbose: false,
            max_turns: 50,
            system_prompt: None,
            append_system_prompt: None,
            no_claude_md: false,
            auto_compact: true,
            thinking_budget: None,
            temperature: None,
        }
    }
}

impl Config {
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .or_else(|| std::env::var("RUSTY_API_KEY").ok())
    }

    pub fn resolve_api_base(&self) -> String {
        self.api_base
            .clone()
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://token-plan-cn.xiaomimimo.com/v1".to_string())
    }

    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".rusty")
    }

    pub fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn sessions_dir() -> PathBuf {
        Self::config_dir().join("sessions")
    }

    pub fn memory_dir() -> PathBuf {
        Self::config_dir().join("memory")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub permissions: HashMap<String, PermissionMode>,
    /// Tool names that are permanently allowed without prompting
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            default_model: None,
            permissions: HashMap::new(),
            allowed_tools: Vec::new(),
        }
    }
}

impl Settings {
    pub async fn load() -> anyhow::Result<Self> {
        let path = Config::settings_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let settings: Self = serde_json::from_str(&content)?;
        Ok(settings)
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let path = Config::settings_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        self.allowed_tools.iter().any(|t| t == tool_name)
    }

    pub fn allowed_tools_set(&self) -> std::collections::HashSet<String> {
        self.allowed_tools.iter().cloned().collect()
    }
}

/// Add a tool name to the permanent allowlist in ~/.rusty/settings.json
pub async fn add_permanent_permission(tool_key: &str) -> anyhow::Result<()> {
    let mut settings = Settings::load().await.unwrap_or_default();
    if !settings.allowed_tools.contains(&tool_key.to_string()) {
        settings.allowed_tools.push(tool_key.to_string());
        settings.save().await?;
    }
    Ok(())
}

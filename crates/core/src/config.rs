// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::permissions::PermissionMode;

/// Where API credentials are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStore {
    /// Stored in the OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service).
    Keyring,
    /// Stored in `~/.rusty/settings.json` (fallback for environments without a keyring).
    SettingsFile,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::SettingsFile
    }
}

impl std::fmt::Display for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keyring => write!(f, "OS Keyring"),
            Self::SettingsFile => write!(f, "settings file"),
        }
    }
}

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
    /// When true, instructs the model to actively use `todowrite` for task tracking.
    /// Implies Plan permission mode (read-only + todowrite).
    pub plan_with_tasks: bool,
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
            plan_with_tasks: false,
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
#[derive(Default)]
pub struct Settings {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub default_model: Option<String>,
    #[serde(default)]
    pub permissions: HashMap<String, PermissionMode>,
    /// Tool names that are permanently allowed without prompting
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Where API credentials are stored (keyring or settings file).
    #[serde(default)]
    pub credential_store: CredentialStore,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Config::default ──────────────────────────────────────────────

    #[test]
    fn default_config_has_expected_values() {
        let cfg = Config::default();
        assert_eq!(cfg.model, "mimo-v2.5-pro");
        assert_eq!(cfg.max_tokens, 16384);
        assert_eq!(cfg.max_turns, 50);
        assert!(cfg.api_key.is_none());
        assert!(cfg.api_base.is_none());
        assert!(!cfg.verbose);
        assert!(!cfg.no_claude_md);
        assert!(cfg.auto_compact);
        assert!(cfg.system_prompt.is_none());
        assert!(cfg.append_system_prompt.is_none());
        assert!(cfg.thinking_budget.is_none());
        assert!(cfg.temperature.is_none());
        assert!(!cfg.plan_with_tasks);
    }

    // ── Config::resolve_api_key ──────────────────────────────────────

    #[test]
    fn resolve_api_key_prefers_explicit() {
        let cfg = Config {
            api_key: Some("explicit-key".into()),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_api_key().as_deref(), Some("explicit-key"));
    }

    // ── Config::resolve_api_base ─────────────────────────────────────

    #[test]
    fn resolve_api_base_prefers_explicit() {
        let cfg = Config {
            api_base: Some("https://custom.api/v1".into()),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_api_base(), "https://custom.api/v1");
    }

    #[test]
    fn resolve_api_base_defaults_to_xiaomi() {
        // When no explicit base and no env var, should return the default
        let cfg = Config {
            api_base: None,
            ..Config::default()
        };
        // We can't easily unset env vars in tests, but we can verify
        // the default value is well-formed
        let base = cfg.resolve_api_base();
        assert!(base.starts_with("https://"), "base should be HTTPS: {base}");
        assert!(base.ends_with("/v1") || base.contains("/v1/"), "base should point to v1: {base}");
    }

    // ── Config::path helpers ─────────────────────────────────────────

    #[test]
    fn config_dir_ends_with_dot_rusty() {
        let dir = Config::config_dir();
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), ".rusty");
    }

    #[test]
    fn settings_path_contains_settings_json() {
        let path = Config::settings_path();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "settings.json");
    }

    #[test]
    fn sessions_dir_contains_sessions() {
        let path = Config::sessions_dir();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "sessions");
    }

    #[test]
    fn memory_dir_contains_memory() {
        let path = Config::memory_dir();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "memory");
    }

    // ── Settings::is_tool_allowed ────────────────────────────────────

    #[test]
    fn is_tool_allowed_true_for_allowed_tool() {
        let settings = Settings {
            allowed_tools: vec!["bash".into(), "file_read".into()],
            ..Settings::default()
        };
        assert!(settings.is_tool_allowed("bash"));
        assert!(settings.is_tool_allowed("file_read"));
    }

    #[test]
    fn is_tool_allowed_false_for_missing_tool() {
        let settings = Settings {
            allowed_tools: vec!["bash".into()],
            ..Settings::default()
        };
        assert!(!settings.is_tool_allowed("file_write"));
        assert!(!settings.is_tool_allowed(""));
    }

    #[test]
    fn is_tool_allowed_false_for_empty_list() {
        let settings = Settings::default();
        assert!(!settings.is_tool_allowed("anything"));
    }

    // ── Settings::allowed_tools_set ──────────────────────────────────

    #[test]
    fn allowed_tools_set_returns_correct_items() {
        let settings = Settings {
            allowed_tools: vec!["bash".into(), "file_read".into(), "bash".into()],
            ..Settings::default()
        };
        let set = settings.allowed_tools_set();
        assert_eq!(set.len(), 2); // deduplicates
        assert!(set.contains("bash"));
        assert!(set.contains("file_read"));
    }

    #[test]
    fn allowed_tools_set_empty_for_default() {
        let settings = Settings::default();
        assert!(settings.allowed_tools_set().is_empty());
    }
}

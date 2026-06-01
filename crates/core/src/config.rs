// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::permissions::PermissionMode;

/// Thinking/reasoning depth tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    /// Minimal reasoning (~1 024 tokens).
    Minimal,
    /// Normal reasoning (~4 096 tokens). Default for fresh sessions.
    Normal,
    /// Deep reasoning (~16 384 tokens).
    Deep,
}

impl std::fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Minimal => write!(f, "minimal"),
            Self::Normal => write!(f, "normal"),
            Self::Deep => write!(f, "deep"),
        }
    }
}

impl ThinkingLevel {
    /// Short single-letter label for the status bar.
    pub fn short_label(&self) -> &'static str {
        match self {
            Self::Minimal => "M",
            Self::Normal => "N",
            Self::Deep => "D",
        }
    }
}

/// Map a thinking level to a token budget.
pub fn level_to_budget(level: ThinkingLevel) -> u32 {
    match level {
        ThinkingLevel::Minimal => 1024,
        ThinkingLevel::Normal => 4096,
        ThinkingLevel::Deep => 16384,
    }
}

/// Map a raw token budget to the nearest thinking level.
pub fn budget_to_level(budget: u32) -> ThinkingLevel {
    if budget <= 2048 {
        ThinkingLevel::Minimal
    } else if budget <= 8192 {
        ThinkingLevel::Normal
    } else {
        ThinkingLevel::Deep
    }
}

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
    pub thinking_level: Option<ThinkingLevel>,
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
            thinking_level: None,
            temperature: None,
            plan_with_tasks: false,
        }
    }
}

/// Return the known context-window size (in tokens) for a model name.
/// Falls back to 128k for unknown models.
pub fn model_context_window(model: &str) -> u32 {
    let lower = model.to_lowercase();
    if lower.contains("mimo") {
        200_000
    } else if lower.contains("kimi") || lower.contains("k2.6") || lower.contains("moonshot") {
        200_000
    } else if lower.contains("gpt-4o") || lower.contains("gpt-4-turbo") {
        128_000
    } else if lower.contains("gpt-4") {
        8_192
    } else if lower.contains("deepseek") {
        64_000
    } else if lower.contains("llama3") || lower.contains("llama-3") {
        128_000
    } else {
        128_000
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

    /// Resolve the effective thinking level.
    ///
    /// Priority:
    /// 1. Explicit `thinking_level`
    /// 2. `thinking_budget` mapped to nearest level
    /// 3. Default to `Normal`
    pub fn resolve_thinking_level(&self) -> ThinkingLevel {
        self.thinking_level
            .clone()
            .or_else(|| self.thinking_budget.map(budget_to_level))
            .unwrap_or(ThinkingLevel::Normal)
    }
}

/// Adjust thinking level downward based on how full the context window is.
///
/// Heuristic:
/// - < 50 %  → keep base level
/// - 50–75 % → step down one level
/// - > 75 %  → force Minimal
pub fn dynamic_thinking_level(base: ThinkingLevel, context_pct: f64) -> ThinkingLevel {
    if context_pct > 0.75 {
        ThinkingLevel::Minimal
    } else if context_pct > 0.50 {
        match base {
            ThinkingLevel::Deep => ThinkingLevel::Normal,
            _ => ThinkingLevel::Minimal,
        }
    } else {
        base
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
        assert!(cfg.thinking_level.is_none());
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

    // ── ThinkingLevel mapping ────────────────────────────────────────

    #[test]
    fn level_to_budget_values() {
        assert_eq!(level_to_budget(ThinkingLevel::Minimal), 1024);
        assert_eq!(level_to_budget(ThinkingLevel::Normal), 4096);
        assert_eq!(level_to_budget(ThinkingLevel::Deep), 16384);
    }

    #[test]
    fn budget_to_level_maps_correctly() {
        assert_eq!(budget_to_level(512), ThinkingLevel::Minimal);
        assert_eq!(budget_to_level(2048), ThinkingLevel::Minimal);
        assert_eq!(budget_to_level(4096), ThinkingLevel::Normal);
        assert_eq!(budget_to_level(8192), ThinkingLevel::Normal);
        assert_eq!(budget_to_level(16384), ThinkingLevel::Deep);
    }

    #[test]
    fn resolve_thinking_level_prefers_explicit_level() {
        let cfg = Config {
            thinking_level: Some(ThinkingLevel::Deep),
            thinking_budget: Some(1024),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_thinking_level(), ThinkingLevel::Deep);
    }

    #[test]
    fn resolve_thinking_level_falls_back_to_budget() {
        let cfg = Config {
            thinking_level: None,
            thinking_budget: Some(1024),
            ..Config::default()
        };
        assert_eq!(cfg.resolve_thinking_level(), ThinkingLevel::Minimal);
    }

    #[test]
    fn resolve_thinking_level_defaults_to_normal() {
        let cfg = Config {
            thinking_level: None,
            thinking_budget: None,
            ..Config::default()
        };
        assert_eq!(cfg.resolve_thinking_level(), ThinkingLevel::Normal);
    }

    // ── dynamic_thinking_level ───────────────────────────────────────

    #[test]
    fn dynamic_level_unchanged_below_half() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.25),
            ThinkingLevel::Deep
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.49),
            ThinkingLevel::Normal
        );
    }

    #[test]
    fn dynamic_level_steps_down_at_half() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.60),
            ThinkingLevel::Normal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.60),
            ThinkingLevel::Minimal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.60),
            ThinkingLevel::Minimal
        );
    }

    #[test]
    fn dynamic_level_forces_minimal_near_limit() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.76),
            ThinkingLevel::Minimal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.90),
            ThinkingLevel::Minimal
        );
    }
}

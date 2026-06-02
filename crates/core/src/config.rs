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

/// Which LLM backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAI,
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "OpenAI"),
        }
    }
}

/// A single provider/model configuration in the model registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Provider group name for hierarchical UI display: "Xiaomi", "Kimi", "DeepSeek", etc.
    /// Empty string means ungrouped (backward compat with old configs).
    #[serde(default)]
    pub group: String,
    /// User-chosen label: "mimo", "openai", "local", etc.
    pub name: String,
    /// Which backend protocol to speak.
    pub provider: ProviderType,
    /// API base URL (e.g. `https://api.openai.com/v1`).
    pub api_base: String,
    /// Model identifier (e.g. `mimo-v2.5-pro`, `gpt-4o`).
    pub model: String,
    /// All model identifiers available on this endpoint.
    /// The first element should match `model` (the primary/default).
    /// When empty, falls back to `vec![model.clone()]`.
    #[serde(default)]
    pub available_models: Vec<String>,
    /// Maximum output tokens.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Thinking/reasoning token budget.
    #[serde(default)]
    pub thinking_budget: Option<u32>,
}

impl ModelEntry {
    /// Returns the list of all available model identifiers for this entry.
    /// Falls back to a single-element vec of `self.model` when `available_models` is empty.
    pub fn model_list(&self) -> Vec<&str> {
        if self.available_models.is_empty() {
            vec![self.model.as_str()]
        } else {
            self.available_models.iter().map(|s| s.as_str()).collect()
        }
    }
}

fn default_max_tokens() -> u32 {
    16384
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
            .or_else(|| self.thinking_budget.map(budget_to_level))
            .unwrap_or(ThinkingLevel::Normal)
    }
}

/// Adjust thinking level based on context fill and task complexity.
///
/// Heuristic:
/// - **Complex tasks** (≥ 2 tool-turns): boost to at least Normal
/// - < 70 %  → keep effective base level
/// - 70–85 % → step down one level
/// - > 85 %  → force Minimal
///
/// The `turn` parameter is the current agent-loop turn index (0-based).
/// When the agent is executing multiple tool calls it signals a complex
/// multi-step task that benefits from deeper reasoning.
pub fn dynamic_thinking_level(base: ThinkingLevel, context_pct: f64, turn: u32) -> ThinkingLevel {
    // For multi-step tasks (2+ turns of tool use), ensure at least Normal thinking.
    let effective_base = if turn >= 2 {
        match base {
            ThinkingLevel::Minimal => ThinkingLevel::Normal,
            other => other,
        }
    } else {
        base
    };

    if context_pct > 0.85 {
        ThinkingLevel::Minimal
    } else if context_pct > 0.70 {
        match effective_base {
            ThinkingLevel::Deep => ThinkingLevel::Normal,
            _ => ThinkingLevel::Minimal,
        }
    } else {
        effective_base
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // ── Legacy flat fields (kept for backward compat / migration) ──
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,

    // ── Model registry ──
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Name of the active entry in `models`.
    #[serde(default)]
    pub active_model: String,
    /// Per-model API keys (plaintext fallback when keyring is unavailable).
    /// Keys are model entry names, values are the API keys.
    #[serde(default)]
    pub api_keys: HashMap<String, String>,

    // ── General settings ──
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub permissions: HashMap<String, PermissionMode>,
    /// Tool names that are permanently allowed without prompting
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Where API credentials are stored (keyring or settings file).
    #[serde(default)]
    pub credential_store: CredentialStore,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: None,
            default_model: None,
            models: Vec::new(),
            active_model: String::new(),
            api_keys: HashMap::new(),
            thinking_level: None,
            permission_mode: None,
            permissions: HashMap::new(),
            allowed_tools: Vec::new(),
            credential_store: CredentialStore::default(),
        }
    }
}


impl Settings {
    pub async fn load() -> anyhow::Result<Self> {
        let path = Config::settings_path();
        let mut settings = if !path.exists() {
            Self::default()
        } else {
            let content = tokio::fs::read_to_string(&path).await?;
            serde_json::from_str(&content)?
        };
        settings.migrate();
        Ok(settings)
    }

    /// Migrate from flat `api_key`/`api_base`/`default_model` fields to the model registry.
    /// Called automatically after deserialization. Idempotent.
    pub fn migrate(&mut self) {
        if !self.models.is_empty() {
            return; // already migrated
        }
        let model_name = self.default_model.clone().unwrap_or_else(|| "mimo-v2.5-pro".to_string());
        let api_base = self.api_base.clone()
            .unwrap_or_else(|| "https://token-plan-cn.xiaomimimo.com/v1".to_string());

        let entry = ModelEntry {
            group: String::new(), // legacy entries are ungrouped
            name: "default".to_string(),
            provider: ProviderType::OpenAI,
            api_base,
            model: model_name,
            available_models: vec![],
            max_tokens: 16384,
            temperature: None,
            thinking_budget: None,
        };
        self.models.push(entry);
        self.active_model = "default".to_string();

        // Migrate the API key into per-model storage
        if let Some(key) = self.api_key.clone() {
            self.api_keys.insert("default".to_string(), key);
        }
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

    // ── Model registry helpers ──────────────────────────────────────

    /// Get the currently active model entry, if it exists.
    pub fn active_model_entry(&self) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.name == self.active_model)
    }

    /// Resolve the API key for the active model entry.
    ///
    /// Priority: per-model key in `api_keys` → env `OPENAI_API_KEY` → env `RUSTY_API_KEY`.
    pub fn resolve_active_api_key(&self) -> Option<String> {
        self.resolve_api_key_for(&self.active_model)
    }

    /// Resolve the API key for a specific model entry by name.
    ///
    /// Priority: env vars → per-model key in `api_keys` → OS keyring → legacy flat `api_key`.
    pub fn resolve_api_key_for(&self, name: &str) -> Option<String> {
        // Priority 1: Environment variables
        for var in &["RUSTY_API_KEY", "OPENAI_API_KEY"] {
            if let Ok(val) = std::env::var(var) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }

        // Priority 2: Per-model key in settings file
        if let Some(key) = self.api_keys.get(name).cloned() {
            if !key.is_empty() {
                return Some(key);
            }
        }

        // Priority 3: OS Keyring
        #[cfg(feature = "os-keyring")]
        if self.credential_store == CredentialStore::Keyring {
            if let Some(key) = crate::credentials::CredentialManager::get_from_keyring() {
                return Some(key);
            }
        }

        // Priority 4: Legacy flat api_key
        if let Some(ref key) = self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        None
    }

    /// Set the API key for a specific model entry in `api_keys`.
    pub fn set_model_api_key(&mut self, name: &str, key: &str) {
        self.api_keys.insert(name.to_string(), key.to_string());
    }

    /// Switch the active model by entry name. Returns `false` if no such entry.
    pub fn switch_active_model(&mut self, name: &str) -> bool {
        if self.models.iter().any(|m| m.name == name) {
            self.active_model = name.to_string();
            true
        } else {
            false
        }
    }

    /// Add a new model entry. If an entry with the same name exists, it is replaced.
    pub fn add_model(&mut self, entry: ModelEntry) {
        self.models.retain(|m| m.name != entry.name);
        self.models.push(entry);
    }

    /// Remove a model entry by name. Returns `true` if found and removed.
    /// Cannot remove the active model.
    pub fn remove_model(&mut self, name: &str) -> bool {
        if name == self.active_model {
            return false;
        }
        let before = self.models.len();
        self.models.retain(|m| m.name != name);
        self.api_keys.remove(name);
        self.models.len() < before
    }

    /// Set the API key for a model entry (stored in the plaintext `api_keys` map).
    pub fn set_api_key(&mut self, model_name: &str, key: String) {
        self.api_keys.insert(model_name.to_string(), key);
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

/// Remove a tool name from the permanent allowlist in ~/.rusty/settings.json.
/// Returns `true` if the tool was found and removed, `false` if it wasn't in the list.
pub async fn remove_permanent_permission(tool_key: &str) -> anyhow::Result<bool> {
    let mut settings = Settings::load().await.unwrap_or_default();
    let before = settings.allowed_tools.len();
    settings.allowed_tools.retain(|t| t != tool_key);
    let removed = settings.allowed_tools.len() < before;
    if removed {
        settings.save().await?;
    }
    Ok(removed)
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
    fn dynamic_level_unchanged_below_seventy() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.25, 0),
            ThinkingLevel::Deep
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.69, 0),
            ThinkingLevel::Normal
        );
    }

    #[test]
    fn dynamic_level_steps_down_at_seventy() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.75, 0),
            ThinkingLevel::Normal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.75, 0),
            ThinkingLevel::Minimal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.75, 0),
            ThinkingLevel::Minimal
        );
    }

    #[test]
    fn dynamic_level_forces_minimal_near_limit() {
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.86, 0),
            ThinkingLevel::Minimal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.90, 0),
            ThinkingLevel::Minimal
        );
    }

    #[test]
    fn dynamic_level_boosts_minimal_for_complex_tasks() {
        // On turn 0 or 1, Minimal stays Minimal
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.30, 0),
            ThinkingLevel::Minimal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.30, 1),
            ThinkingLevel::Minimal
        );
        // On turn 2+, Minimal gets boosted to Normal (complex task)
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.30, 2),
            ThinkingLevel::Normal
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.30, 5),
            ThinkingLevel::Normal
        );
        // Deep and Normal stay the same regardless of turn
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.30, 3),
            ThinkingLevel::Deep
        );
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Normal, 0.30, 3),
            ThinkingLevel::Normal
        );
    }

    #[test]
    fn dynamic_level_complex_task_context_pressure() {
        // Complex task (turn 3) with high context: Minimal boosted to Normal,
        // then stepped down to Minimal due to context pressure
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Minimal, 0.75, 3),
            ThinkingLevel::Minimal // boosted to Normal, then 70-85% → Minimal
        );
        // Complex task with Deep base at high context
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.75, 3),
            ThinkingLevel::Normal
        );
        // Complex task over 85%: forced to Minimal
        assert_eq!(
            dynamic_thinking_level(ThinkingLevel::Deep, 0.90, 3),
            ThinkingLevel::Minimal
        );
    }

    // ── Model registry ───────────────────────────────────────────────

    #[test]
    fn migrate_creates_default_entry_from_legacy_fields() {
        let mut settings = Settings {
            api_key: Some("sk-test".into()),
            api_base: Some("https://custom.api.com/v1".into()),
            default_model: Some("gpt-4o".into()),
            ..Settings::default()
        };
        settings.migrate();

        assert_eq!(settings.models.len(), 1);
        assert_eq!(settings.active_model, "default");
        let entry = &settings.models[0];
        assert_eq!(entry.name, "default");
        assert_eq!(entry.provider, ProviderType::OpenAI);
        assert_eq!(entry.api_base, "https://custom.api.com/v1");
        assert_eq!(entry.model, "gpt-4o");
        assert_eq!(entry.max_tokens, 16384);
        // API key migrated to per-model map
        assert_eq!(settings.api_keys.get("default").unwrap(), "sk-test");
    }

    #[test]
    fn migrate_defaults_when_no_legacy_values() {
        let mut settings = Settings::default();
        settings.migrate();

        assert_eq!(settings.models.len(), 1);
        let entry = &settings.models[0];
        assert_eq!(entry.name, "default");
        assert_eq!(entry.model, "mimo-v2.5-pro");
        assert_eq!(entry.api_base, "https://token-plan-cn.xiaomimimo.com/v1");
    }

    #[test]
    fn migrate_skips_if_models_already_populated() {
        let existing = ModelEntry {
            group: "Other".into(),
            name: "custom".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://other.com/v1".into(),
            model: "other-model".into(),
            available_models: vec![],
            max_tokens: 8192,
            temperature: None,
            thinking_budget: None,
        };
        let mut settings = Settings {
            api_key: Some("sk-legacy".into()),
            models: vec![existing],
            active_model: "custom".into(),
            ..Settings::default()
        };
        settings.migrate();

        // Should NOT create a "default" entry
        assert_eq!(settings.models.len(), 1);
        assert_eq!(settings.models[0].name, "custom");
        // Legacy key should NOT be migrated
        assert!(!settings.api_keys.contains_key("default"));
    }

    #[test]
    fn active_model_entry_returns_correct_entry() {
        let mut settings = Settings::default();
        settings.migrate();
        assert!(settings.active_model_entry().is_some());
        assert_eq!(settings.active_model_entry().unwrap().name, "default");
    }

    #[test]
    fn active_model_entry_returns_none_when_no_match() {
        let settings = Settings {
            active_model: "nonexistent".into(),
            ..Settings::default()
        };
        assert!(settings.active_model_entry().is_none());
    }

    #[test]
    fn switch_active_model_works() {
        let mut settings = Settings::default();
        settings.migrate();
        settings.add_model(ModelEntry {
            group: "OpenAI".into(),
            name: "gpt4".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            available_models: vec![],
            max_tokens: 4096,
            temperature: None,
            thinking_budget: None,
        });

        assert!(settings.switch_active_model("gpt4"));
        assert_eq!(settings.active_model, "gpt4");
        assert_eq!(settings.active_model_entry().unwrap().model, "gpt-4o");
    }

    #[test]
    fn switch_active_model_rejects_unknown_name() {
        let mut settings = Settings::default();
        settings.migrate();
        assert!(!settings.switch_active_model("nonexistent"));
        assert_eq!(settings.active_model, "default");
    }

    #[test]
    fn add_model_replaces_existing_with_same_name() {
        let mut settings = Settings::default();
        settings.migrate();

        let updated = ModelEntry {
            group: "Custom".into(),
            name: "default".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://new-api.com/v1".into(),
            model: "new-model".into(),
            available_models: vec![],
            max_tokens: 8192,
            temperature: Some(0.7),
            thinking_budget: None,
        };
        settings.add_model(updated);

        assert_eq!(settings.models.len(), 1);
        assert_eq!(settings.models[0].api_base, "https://new-api.com/v1");
        assert_eq!(settings.models[0].temperature, Some(0.7));
    }

    #[test]
    fn remove_model_prevents_removing_active() {
        let mut settings = Settings::default();
        settings.migrate();

        // Cannot remove active model
        assert!(!settings.remove_model("default"));
        assert_eq!(settings.models.len(), 1);
    }

    #[test]
    fn remove_model_removes_inactive_entry() {
        let mut settings = Settings::default();
        settings.migrate();
        settings.add_model(ModelEntry {
            group: "OpenAI".into(),
            name: "extra".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://x.com/v1".into(),
            model: "x-model".into(),
            available_models: vec![],
            max_tokens: 4096,
            temperature: None,
            thinking_budget: None,
        });
        settings.set_api_key("extra", "sk-extra".into());

        assert!(settings.remove_model("extra"));
        assert_eq!(settings.models.len(), 1);
        assert!(!settings.api_keys.contains_key("extra"));
    }

    #[test]
    fn resolve_active_api_key_from_per_model_map() {
        let mut settings = Settings::default();
        settings.migrate();
        settings.set_api_key("default", "sk-per-model".into());

        let key = settings.resolve_active_api_key();
        assert_eq!(key.unwrap(), "sk-per-model");
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::permissions::PermissionMode;

/// Set restrictive (owner-only) permissions on a directory.
/// On Unix: 0o700. No-op on other platforms.
#[cfg(unix)]
pub fn set_restrictive_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub fn set_restrictive_dir_permissions(_path: &Path) {}

/// Set restrictive (owner-only) permissions on a file.
/// On Unix: 0o600. No-op on other platforms.
#[cfg(unix)]
pub fn set_restrictive_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub fn set_restrictive_file_permissions(_path: &Path) {}

/// Write content to a file atomically with restrictive permissions at creation time.
/// On Unix, uses `OpenOptions::mode(0o600)` to avoid TOCTOU between file creation
/// and permission setting. Creates parent directories via `ensure_restricted_dir`.
pub fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_restricted_dir(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(content)
            })
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(content)
            })
    }
}

/// Async version of `atomic_write` using `tokio::fs`.
pub async fn atomic_write_async(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let parent = parent.to_path_buf();
        tokio::task::spawn_blocking(move || ensure_restricted_dir(&parent)).await??;
    }
    let path = path.to_path_buf();
    let content = content.to_vec();
    tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(&content)
                })
        }
        #[cfg(not(unix))]
        {
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(&content)
                })
        }
    })
    .await?
}
/// Creates the directory and all missing parents, then sets 0o700 on each
/// newly-created directory in the chain.
pub fn ensure_restricted_dir(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        set_restrictive_dir_permissions(path);
        return Ok(());
    }
    // Walk up to find the first existing ancestor
    let mut to_create = Vec::new();
    let mut current = Some(path);
    while let Some(p) = current {
        if p.exists() {
            break;
        }
        to_create.push(p);
        current = p.parent();
    }
    // Create from ancestor down, setting permissions on each
    for p in to_create.iter().rev() {
        std::fs::create_dir(p)?;
        set_restrictive_dir_permissions(p);
    }
    Ok(())
}

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
    /// Extended reasoning (~32 768 tokens). Maximum depth for complex architectural work.
    Extended,
}

impl std::fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Minimal => write!(f, "minimal"),
            Self::Normal => write!(f, "normal"),
            Self::Deep => write!(f, "deep"),
            Self::Extended => write!(f, "extended"),
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
            Self::Extended => "E",
        }
    }
}

/// Map a thinking level to a token budget.
pub fn level_to_budget(level: ThinkingLevel) -> u32 {
    match level {
        ThinkingLevel::Minimal => 1024,
        ThinkingLevel::Normal => 4096,
        ThinkingLevel::Deep => 16384,
        ThinkingLevel::Extended => 32768,
    }
}

/// Map a raw token budget to the nearest thinking level.
pub fn budget_to_level(budget: u32) -> ThinkingLevel {
    if budget <= 2048 {
        ThinkingLevel::Minimal
    } else if budget <= 8192 {
        ThinkingLevel::Normal
    } else if budget <= 24576 {
        ThinkingLevel::Deep
    } else {
        ThinkingLevel::Extended
    }
}

/// Which LLM backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAI,
    Anthropic,
}

impl Default for ProviderType {
    fn default() -> Self {
        Self::OpenAI
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "OpenAI"),
            Self::Anthropic => write!(f, "Anthropic"),
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
    #[serde(default)]
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
    /// Extra HTTP headers to send with every request to this provider.
    /// Used by providers like Kimi that require custom headers for routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
    /// Context window size in tokens for this specific model.
    /// When set, overrides the hardcoded lookup in `model_context_window()`.
    /// Used for auto-compaction thresholds and TUI context-usage display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
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
    /// Provider type, set by presets and resolved through model registry.
    /// Used as a fallback when no model entry is active.
    #[serde(default)]
    pub provider_type: ProviderType,
    /// Context window size in tokens for the active model.
    /// When set, overrides the hardcoded lookup in `model_context_window()`.
    /// Populated from the active `ModelEntry.context_window` at startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
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
            provider_type: ProviderType::default(),
            context_window: None,
        }
    }
}

/// Return the known context-window size (in tokens) for a model name.
/// Falls back to 128k for unknown models.
fn model_context_window(model: &str) -> u32 {
    let lower = model.to_lowercase();
    if lower.contains("mimo") {
        1_000_000
    } else if lower.contains("glm") {
        1_000_000
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

/// Resolve the effective context window for a model.
/// Checks `override_value` first (e.g. from `Config.context_window` or `ModelEntry.context_window`),
/// falling back to the hardcoded lookup via `model_context_window()`.
pub fn resolve_context_window(override_value: Option<u32>, model: &str) -> u32 {
    override_value.unwrap_or_else(|| model_context_window(model))
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

    /// Return the effective context window for this config's model.
    /// Uses `self.context_window` if set, otherwise falls back to the hardcoded lookup.
    pub fn effective_context_window(&self) -> u32 {
        resolve_context_window(self.context_window, &self.model)
    }

    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".rusty")
    }

    pub fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn sessions_dir(working_dir: &Path) -> PathBuf {
        Self::config_dir()
            .join("sessions")
            .join(Self::project_hash(working_dir))
    }

    /// Generate a short deterministic hash of the canonical working directory.
    ///
    /// Uses the directory name as a human-readable prefix and a truncated
    /// SHA-256 hex suffix for uniqueness. Sessions are stored per-project
    /// under `~/.rusty/sessions/<project_hash>/` so different projects
    /// never see each other's history.
    pub fn project_hash(working_dir: &Path) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Canonicalise to ensure the same directory always produces the same hash,
        // regardless of how the user invoked rusty (e.g. via symlink or trailing /).
        let canonical = std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf());

        let dir_name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());

        let mut hasher = DefaultHasher::new();
        canonical.hash(&mut hasher);
        let hash = hasher.finish();

        // Format as <dir_name>-<12 hex chars> for readability + uniqueness.
        format!("{}-{:012x}", dir_name, hash)
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
        settings.fixup();
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
            extra_headers: None,
            context_window: None,
        };
        self.models.push(entry);
        self.active_model = "default".to_string();

        // Migrate the API key into per-model storage
        if let Some(key) = self.api_key.clone() {
            self.api_keys.insert("default".to_string(), key);
        }
    }

    /// Patch existing model entries with known-good defaults for providers
    /// that require specific settings (e.g. Kimi requires temperature=1 and
    /// X-Title header, MiMo/DeepSeek support thinking). Called after `migrate()`.
    /// Idempotent.
    pub fn fixup(&mut self) {
        for entry in &mut self.models {
            let name_lower = entry.name.to_lowercase();
            let base_lower = entry.api_base.to_lowercase();
            let is_kimi = name_lower.contains("kimi") || base_lower.contains("kimi.com");
            let is_mimo = name_lower.starts_with("xiaomi")
                || base_lower.contains("xiaomimimo.com");
            let is_deepseek = name_lower.starts_with("deepseek")
                || base_lower.contains("api.deepseek.com");

            if is_kimi {
                // Kimi requires temperature=1 and X-Title header to identify as coding agent
                entry.temperature = Some(1.0);
                let headers = entry.extra_headers.get_or_insert_with(HashMap::new);
                headers.insert("X-Title".to_string(), "Rusty".to_string());
                // Kimi supports reasoning_content for thinking
                if entry.thinking_budget.is_none() {
                    entry.thinking_budget = Some(4096);
                }
            } else if is_mimo || is_deepseek {
                // These providers support thinking — set a default budget if missing
                if entry.thinking_budget.is_none() {
                    entry.thinking_budget = Some(4096);
                }
            }
        }
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let path = Config::settings_path();
        let content = serde_json::to_string_pretty(self)?;
        atomic_write_async(&path, content.as_bytes()).await?;
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
        let path = Config::sessions_dir(std::path::Path::new("/tmp/test-project"));
        // The last two components should be "sessions" and then the project hash.
        let file_name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            file_name.contains('-'),
            "project hash should contain a dash: {file_name}"
        );
        assert_eq!(
            path.parent().unwrap().file_name().unwrap().to_str().unwrap(),
            "sessions"
        );
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
        assert_eq!(level_to_budget(ThinkingLevel::Extended), 32768);
    }

    #[test]
    fn budget_to_level_maps_correctly() {
        assert_eq!(budget_to_level(512), ThinkingLevel::Minimal);
        assert_eq!(budget_to_level(2048), ThinkingLevel::Minimal);
        assert_eq!(budget_to_level(4096), ThinkingLevel::Normal);
        assert_eq!(budget_to_level(8192), ThinkingLevel::Normal);
        assert_eq!(budget_to_level(16384), ThinkingLevel::Deep);
        assert_eq!(budget_to_level(24576), ThinkingLevel::Deep);
        assert_eq!(budget_to_level(32768), ThinkingLevel::Extended);
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
            extra_headers: None,
            context_window: None,
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
            extra_headers: None,
            context_window: None,
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
            extra_headers: None,
            context_window: None,
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
            extra_headers: None,
            context_window: None,
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

    #[test]
    fn fixup_kimi_sets_temperature_and_thinking() {
        let mut settings = Settings::default();
        settings.models.push(ModelEntry {
            group: "Kimi".into(),
            name: "kimi-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.kimi.com/coding/v1".into(),
            model: "kimi-for-coding".into(),
            available_models: vec![],
            max_tokens: 32768,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        });
        settings.fixup();
        assert_eq!(settings.models[0].temperature, Some(1.0));
        assert_eq!(settings.models[0].thinking_budget, Some(4096));
        let headers = settings.models[0].extra_headers.as_ref().unwrap();
        assert_eq!(headers.get("X-Title").unwrap(), "Rusty");
    }

    #[test]
    fn fixup_mimo_sets_thinking_budget_if_missing() {
        let mut settings = Settings::default();
        settings.models.push(ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec![],
            max_tokens: 32768,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        });
        settings.fixup();
        assert_eq!(settings.models[0].thinking_budget, Some(4096));
    }

    #[test]
    fn fixup_deepseek_sets_thinking_budget_if_missing() {
        let mut settings = Settings::default();
        settings.models.push(ModelEntry {
            group: "DeepSeek".into(),
            name: "deepseek-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            available_models: vec![],
            max_tokens: 384000,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        });
        settings.fixup();
        assert_eq!(settings.models[0].thinking_budget, Some(4096));
    }

    #[test]
    fn fixup_does_not_overwrite_existing_thinking_budget() {
        let mut settings = Settings::default();
        settings.models.push(ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec![],
            max_tokens: 32768,
            temperature: Some(0.7),
            thinking_budget: Some(16384),
            extra_headers: None,
            context_window: None,
        });
        settings.fixup();
        assert_eq!(settings.models[0].thinking_budget, Some(16384));
    }

    #[test]
    fn fixup_ignores_unknown_providers() {
        let mut settings = Settings::default();
        settings.models.push(ModelEntry {
            group: "Custom".into(),
            name: "my-model".into(),
            provider: ProviderType::OpenAI,
            api_base: "http://localhost:8080/v1".into(),
            model: "default".into(),
            available_models: vec![],
            max_tokens: 16384,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        });
        settings.fixup();
        assert_eq!(settings.models[0].temperature, Some(0.7));
        assert_eq!(settings.models[0].thinking_budget, None);
    }
}
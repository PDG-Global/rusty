// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Model registry — thin wrapper around `rusty_core::ModelEntry` for TUI use.
//!
//! Re-exports the core `ModelEntry` and adds display helpers for the settings overlay.

// Re-export the core type so the rest of the TUI crate can use
// `crate::model_registry::ModelEntry` without depending directly on rusty_core.
pub use rusty_core::ModelEntry;

use rusty_core::{ProviderType, ThinkingLevel};

/// Fallback list of models shown when no config file exists yet.
///
/// Each entry corresponds to one of the well-known presets shipped with
/// `rusty`.  The `api_key` is stored separately in the credential store,
/// so these entries carry only the structural metadata needed to populate
/// the settings overlay.
pub fn default_model_list() -> Vec<ModelEntry> {
    vec![
        // ── Kimi ─────────────────────────────────────────────────────────
        ModelEntry {
            group: "Kimi".into(),
            name: "kimi-global".into(),
            provider: ProviderType::Anthropic,
            api_base: "https://api.kimi.com/coding/v1/".into(),
            model: "kimi-k2.6".into(),
            available_models: vec!["kimi-k2.6".into(), "kimi-k2.5".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        ModelEntry {
            group: "Kimi".into(),
            name: "kimi-cn".into(),
            provider: ProviderType::Anthropic,
            api_base: "https://api.kimi.com/coding/v1/".into(),
            model: "kimi-k2.6".into(),
            available_models: vec!["kimi-k2.6".into(), "kimi-k2.5".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── Xiaomi MiMo ──────────────────────────────────────────────────
        ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec!["mimo-v2.5-pro".into(), "mimo-v2.5-flash".into()],
            max_tokens: 32_768,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan-cn.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec!["mimo-v2.5-pro".into(), "mimo-v2.5-flash".into()],
            max_tokens: 32_768,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── DeepSeek ─────────────────────────────────────────────────────
        ModelEntry {
            group: "DeepSeek".into(),
            name: "deepseek-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            available_models: vec!["deepseek-v4-pro".into(), "deepseek-v4-flash".into()],
            max_tokens: 384_000,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        ModelEntry {
            group: "DeepSeek".into(),
            name: "deepseek-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            available_models: vec!["deepseek-v4-pro".into(), "deepseek-v4-flash".into()],
            max_tokens: 384_000,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── Zhipu GLM ────────────────────────────────────────────────────
        ModelEntry {
            group: "Zhipu GLM".into(),
            name: "zhipu-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://open.bigmodel.cn/api/paas/v4/".into(),
            model: "glm-5.1".into(),
            available_models: vec!["glm-5.1".into(), "glm-5-turbo".into(), "glm-4.6".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        ModelEntry {
            group: "Zhipu GLM".into(),
            name: "zhipu-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.z.ai/api/coding/paas/v4".into(),
            model: "glm-5.1".into(),
            available_models: vec!["glm-5.1".into(), "glm-5-turbo".into(), "glm-4.6".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── MiniMax ──────────────────────────────────────────────────────
        ModelEntry {
            group: "MiniMax".into(),
            name: "minimax-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.minimax.io/v1".into(),
            model: "MiniMax-M2.7".into(),
            available_models: vec![
                "MiniMax-M2.7".into(),
                "MiniMax-M2.7-highspeed".into(),
                "MiniMax-M2.5".into(),
            ],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        ModelEntry {
            group: "MiniMax".into(),
            name: "minimax-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.minimaxi.com/v1".into(),
            model: "MiniMax-M2.7".into(),
            available_models: vec![
                "MiniMax-M2.7".into(),
                "MiniMax-M2.7-highspeed".into(),
                "MiniMax-M2.5".into(),
            ],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── OpenAI ───────────────────────────────────────────────────────
        ModelEntry {
            group: "OpenAI".into(),
            name: "openai".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.openai.com/v1".into(),
            model: "gpt-5.5".into(),
            available_models: vec![
                "gpt-5.5".into(),
                "gpt-5.4".into(),
                "gpt-5.4-nano".into(),
                "o3".into(),
                "o4-mini".into(),
            ],
            max_tokens: 16_384,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
        // ── Ollama (local) ───────────────────────────────────────────────
        ModelEntry {
            group: "Ollama".into(),
            name: "ollama".into(),
            provider: ProviderType::OpenAI,
            api_base: "http://localhost:11434/v1".into(),
            model: "llama3".into(),
            available_models: vec![],
            max_tokens: 4_096,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
        },
    ]
}

/// Tabular display widths used by the settings overlay.
pub const NAME_WIDTH: usize = 12;
pub const MODEL_WIDTH: usize = 18;
pub const BASE_URL_WIDTH: usize = 20;

/// Format a model entry as a single summary line for display.
///
/// Example: `mimo          mimo-v2.5-pro        https://api.mimo…`
pub fn format_entry_line(entry: &ModelEntry, is_active: bool) -> String {
    let marker = if is_active { "●" } else { " " };
    let name = truncate_str(&entry.name, NAME_WIDTH);
    let model = truncate_str(&entry.model, MODEL_WIDTH);
    let base = truncate_str(&entry.api_base, BASE_URL_WIDTH);
    format!(
        "{marker} {:<name_w$}  {:<model_w$}  {:<base_w$}",
        name,
        model,
        base,
        name_w = NAME_WIDTH,
        model_w = MODEL_WIDTH,
        base_w = BASE_URL_WIDTH,
    )
}

/// Cycle to the next thinking level: off → minimal → normal → deep → off.
pub fn next_thinking_level(current: Option<ThinkingLevel>) -> Option<ThinkingLevel> {
    match current {
        None => Some(ThinkingLevel::Minimal),
        Some(ThinkingLevel::Minimal) => Some(ThinkingLevel::Normal),
        Some(ThinkingLevel::Normal) => Some(ThinkingLevel::Deep),
        Some(ThinkingLevel::Deep) => None,
    }
}

/// Display label for an optional thinking level.
pub fn thinking_level_label(level: Option<ThinkingLevel>) -> &'static str {
    match level {
        None => "off",
        Some(ThinkingLevel::Minimal) => "minimal",
        Some(ThinkingLevel::Normal) => "normal",
        Some(ThinkingLevel::Deep) => "deep",
    }
}

/// Truncate a string to `max_len` characters, appending `…` if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_len.saturating_sub(1)).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        assert_eq!(truncate_str("abcdefghijklmnop", 8), "abcdefg…");
    }

    #[test]
    fn test_thinking_cycle() {
        assert_eq!(next_thinking_level(None), Some(ThinkingLevel::Minimal));
        assert_eq!(
            next_thinking_level(Some(ThinkingLevel::Minimal)),
            Some(ThinkingLevel::Normal)
        );
        assert_eq!(
            next_thinking_level(Some(ThinkingLevel::Deep)),
            None
        );
    }

    #[test]
    fn test_format_entry_line() {
        let entry = ModelEntry {
            group: "Test".into(),
            name: "test".into(),
            provider: rusty_core::ProviderType::OpenAI,
            api_base: "https://api.example.com/v1".into(),
            model: "gpt-4o".into(),
            available_models: vec![],
            max_tokens: 4096,
            temperature: None,
            thinking_budget: None,
            extra_headers: None,
        };
        let line = format_entry_line(&entry, true);
        assert!(line.starts_with('●'));
        assert!(line.contains("test"));
    }
}
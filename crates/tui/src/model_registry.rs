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
            provider: ProviderType::OpenAI,
            api_base: "https://api.kimi.com/coding/v1".into(),
            model: "kimi-for-coding".into(),
            available_models: vec!["kimi-for-coding".into(), "kimi-for-coding-highspeed".into(), "k3.1-pro".into(), "k3".into()],
            max_tokens: 32_768,
            temperature: Some(1.0),
            thinking_budget: Some(4096),
            extra_headers: Some(
                vec![("X-Title".to_string(), "Rusty".to_string())]
                    .into_iter()
                    .collect(),
            ),
            context_window: Some(1_048_576),
        },
        ModelEntry {
            group: "Kimi".into(),
            name: "kimi-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.kimi.com/coding/v1".into(),
            model: "kimi-for-coding".into(),
            available_models: vec!["kimi-for-coding".into(), "kimi-for-coding-highspeed".into(), "k3.1-pro".into(), "k3".into()],
            max_tokens: 32_768,
            temperature: Some(1.0),
            thinking_budget: Some(4096),
            extra_headers: Some(
                vec![("X-Title".to_string(), "Rusty".to_string())]
                    .into_iter()
                    .collect(),
            ),
            context_window: Some(1_048_576),
        },
        // ── Xiaomi MiMo ──────────────────────────────────────────────────
        ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan-sgp.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec!["mimo-v2.5-pro".into(), "mimo-v2.5".into()],
            max_tokens: 32_768,
            temperature: Some(0.7),
            thinking_budget: Some(4096),
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        ModelEntry {
            group: "Xiaomi MiMo".into(),
            name: "xiaomi-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://token-plan-cn.xiaomimimo.com/v1".into(),
            model: "mimo-v2.5-pro".into(),
            available_models: vec!["mimo-v2.5-pro".into(), "mimo-v2.5".into()],
            max_tokens: 32_768,
            temperature: Some(0.7),
            thinking_budget: Some(4096),
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        // ── DeepSeek ─────────────────────────────────────────────────────
        ModelEntry {
            group: "DeepSeek".into(),
            name: "deepseek-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            available_models: vec!["deepseek-v4-pro".into(), "deepseek-flash".into()],
            max_tokens: 384_000,
            temperature: Some(0.7),
            thinking_budget: Some(4096),
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        ModelEntry {
            group: "DeepSeek".into(),
            name: "deepseek-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.deepseek.com".into(),
            model: "deepseek-v4-pro".into(),
            available_models: vec!["deepseek-v4-pro".into(), "deepseek-flash".into()],
            max_tokens: 384_000,
            temperature: Some(0.7),
            thinking_budget: Some(4096),
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        // ── Zhipu GLM ────────────────────────────────────────────────────
        ModelEntry {
            group: "Zhipu GLM".into(),
            name: "zhipu-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.z.ai/api/coding/paas/v4".into(),
            model: "glm-5.3".into(),
            available_models: vec!["glm-5.3".into(), "glm-5.3-flash".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        },
        ModelEntry {
            group: "Zhipu GLM".into(),
            name: "zhipu-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://open.bigmodel.cn/api/coding/paas/v4".into(),
            model: "glm-5.3".into(),
            available_models: vec!["glm-5.3".into(), "glm-5.3-flash".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        },
        // ── MiniMax ──────────────────────────────────────────────────────
        ModelEntry {
            group: "MiniMax".into(),
            name: "minimax-global".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.minimax.io/v1".into(),
            model: "MiniMax-M3".into(),
            available_models: vec!["MiniMax-M3".into(), "MiniMax-M3.1-Flash".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        ModelEntry {
            group: "MiniMax".into(),
            name: "minimax-cn".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.minimaxi.com/v1".into(),
            model: "MiniMax-M3".into(),
            available_models: vec!["MiniMax-M3".into(), "MiniMax-M3.1-Flash".into()],
            max_tokens: 8_192,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: Some(1_048_576),
        },
        // ── OpenAI ───────────────────────────────────────────────────────
        ModelEntry {
            group: "OpenAI".into(),
            name: "openai".into(),
            provider: ProviderType::OpenAI,
            api_base: "https://api.openai.com/v1".into(),
            model: "gpt-6.1-sol".into(),
            available_models: vec!["gpt-6.1-sol".into(), "gpt-6-astra".into(), "gpt-6-luna".into()],
            max_tokens: 16_384,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        },
        // ── Anthropic ────────────────────────────────────────────────────
        ModelEntry {
            group: "Anthropic".into(),
            name: "anthropic".into(),
            provider: ProviderType::Anthropic,
            api_base: "https://api.anthropic.com/v1".into(),
            model: "claude-sonnet-5-5".into(),
            available_models: vec![
                "claude-sonnet-5-5".into(),
                "claude-opus-5-5".into(),
                "claude-haiku-5-5".into(),
                "claude-fable-5-1".into(),
            ],
            max_tokens: 16_384,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: Some(1_000_000),
        },
        // ── Ollama (local) ───────────────────────────────────────────────
        ModelEntry {
            group: "Ollama".into(),
            name: "ollama".into(),
            provider: ProviderType::OpenAI,
            api_base: "http://localhost:11434/v1".into(),
            model: "qwen3-coder:30b".into(),
            available_models: vec!["qwen3-coder:30b".into(), "gpt-oss:20b".into(), "gpt-oss:120b".into()],
            max_tokens: 4_096,
            temperature: Some(0.7),
            thinking_budget: None,
            extra_headers: None,
            context_window: None,
        },
    ]
}

/// Extract the host portion of `api_base` for compact display.
///
/// `"https://token-plan-cn.xiaomimimo.com/v1"` -> `"token-plan-cn.xiaomimimo.com"`
/// Returns the raw string unchanged if it has no scheme prefix.
pub fn host_of(api_base: &str) -> &str {
    let rest = api_base
        .strip_prefix("https://")
        .or_else(|| api_base.strip_prefix("http://"))
        .unwrap_or(api_base);
    match rest.find('/') {
        Some(i) => &rest[..i],
        None => rest,
    }
}

/// Human-readable token count: `32K`, `1M`, or `"-"` when zero.
pub fn format_tokens(count: u32) -> String {
    if count == 0 {
        return "-".to_string();
    }
    if count >= 1_000_000 {
        format!("{}M", count / 1_000_000)
    } else if count >= 1000 {
        format!("{}K", count / 1000)
    } else {
        count.to_string()
    }
}

/// Cycle to the next thinking level: off → minimal → normal → deep → extended → off.
pub fn next_thinking_level(current: Option<ThinkingLevel>) -> Option<ThinkingLevel> {
    match current {
        None => Some(ThinkingLevel::Minimal),
        Some(ThinkingLevel::Minimal) => Some(ThinkingLevel::Normal),
        Some(ThinkingLevel::Normal) => Some(ThinkingLevel::Deep),
        Some(ThinkingLevel::Deep) => Some(ThinkingLevel::Extended),
        Some(ThinkingLevel::Extended) => None,
    }
}

/// Display label for an optional thinking level.
pub fn thinking_level_label(level: Option<ThinkingLevel>) -> &'static str {
    match level {
        None => "off",
        Some(ThinkingLevel::Minimal) => "minimal",
        Some(ThinkingLevel::Normal) => "normal",
        Some(ThinkingLevel::Deep) => "deep",
        Some(ThinkingLevel::Extended) => "extended",
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_cycle() {
        assert_eq!(next_thinking_level(None), Some(ThinkingLevel::Minimal));
        assert_eq!(
            next_thinking_level(Some(ThinkingLevel::Minimal)),
            Some(ThinkingLevel::Normal)
        );
        assert_eq!(
            next_thinking_level(Some(ThinkingLevel::Normal)),
            Some(ThinkingLevel::Deep)
        );
        assert_eq!(
            next_thinking_level(Some(ThinkingLevel::Deep)),
            Some(ThinkingLevel::Extended)
        );
        assert_eq!(next_thinking_level(Some(ThinkingLevel::Extended)), None);
    }

    #[test]
    fn test_host_of() {
        assert_eq!(
            host_of("https://token-plan-cn.xiaomimimo.com/v1"),
            "token-plan-cn.xiaomimimo.com"
        );
        assert_eq!(host_of("http://localhost:11434/v1"), "localhost:11434");
        assert_eq!(host_of("https://api.deepseek.com"), "api.deepseek.com");
        assert_eq!(host_of("not-a-url"), "not-a-url");
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(0), "-");
        assert_eq!(format_tokens(512), "512");
        assert_eq!(format_tokens(4096), "4K");
        assert_eq!(format_tokens(32_768), "32K");
        assert_eq!(format_tokens(1_048_576), "1M");
        assert_eq!(format_tokens(2_000_000), "2M");
    }
}

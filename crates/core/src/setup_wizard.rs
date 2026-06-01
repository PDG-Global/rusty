// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Interactive first-run setup wizard.
//!
//! Guides users through provider selection, API key entry, and model configuration.
//! Works in plain terminal mode — no TUI dependency — so it operates over SSH,
//! in containers, and in CI environments.

use crate::credentials::CredentialManager;
use crate::{Config, CredentialStore, RustyError, Settings};
use crossterm::style::Stylize;
use std::io::{self, BufRead, Write};

/// A preconfigured API provider.
#[derive(Debug, Clone)]
pub struct ProviderPreset {
    pub name: &'static str,
    pub api_base: &'static str,
    pub default_model: &'static str,
    pub needs_key: bool,
}

impl ProviderPreset {
    /// All available provider presets.
    pub fn all() -> Vec<Self> {
        vec![
            Self {
                name: "Xiaomi (MiMo)",
                api_base: "https://token-plan-cn.xiaomimimo.com/v1",
                default_model: "mimo-v2.5-pro",
                needs_key: true,
            },
            Self {
                name: "Kimi (Moonshot)",
                api_base: "https://api.moonshot.cn/v1",
                default_model: "kimi-k2",
                needs_key: true,
            },
            Self {
                name: "OpenAI",
                api_base: "https://api.openai.com/v1",
                default_model: "gpt-4.1",
                needs_key: true,
            },
            Self {
                name: "DeepSeek",
                api_base: "https://api.deepseek.com/v1",
                default_model: "deepseek-chat",
                needs_key: true,
            },
            Self {
                name: "Ollama (local)",
                api_base: "http://localhost:11434/v1",
                default_model: "qwen3:8b",
                needs_key: false,
            },
            Self {
                name: "Custom (OpenAI-compatible)",
                api_base: "http://localhost:8080/v1",
                default_model: "default",
                needs_key: true,
            },
        ]
    }
}

/// Check if this is the first run (no settings file exists yet).
pub fn is_first_run() -> bool {
    !Config::settings_path().exists()
}

/// Run the interactive setup wizard.
///
/// Returns `Ok(true)` if configuration was saved, `Ok(false)` if the user cancelled.
pub async fn run_setup_wizard() -> Result<bool, RustyError> {
    println!();
    println!(
        "  {}",
        "╔══════════════════════════════════════╗"
            .dark_grey()
    );
    println!(
        "  {}",
        "║                                      ║"
            .dark_grey()
    );
    println!(
        "  {}{}{}",
        "║  ".dark_grey(),
        "⚙  Rusty Setup Wizard".bold().cyan(),
        "              ║".dark_grey()
    );
    println!(
        "  {}",
        "║                                      ║"
            .dark_grey()
    );
    println!(
        "  {}",
        "╚══════════════════════════════════════╝"
            .dark_grey()
    );
    println!();
    println!(
        "  {}",
        "Configure your LLM provider to get started.".dark_grey()
    );
    println!();

    // --- Provider selection ---
    let providers = ProviderPreset::all();
    println!("  {}", "Select a provider:".bold().white());
    println!();
    for (i, p) in providers.iter().enumerate() {
        let key_hint = if p.needs_key {
            ""
        } else {
            " (no API key needed)"
        };
        println!(
            "    {} {}{}",
            format!("[{}]", i + 1).cyan(),
            p.name,
            key_hint.dark_grey()
        );
    }
    println!();

    let choice = loop {
        print!("  {} ", "▶".cyan());
        io::stdout().flush().ok();
        let input = read_line_default("1")?;
        match input.trim().parse::<usize>() {
            Ok(n) if n >= 1 && n <= providers.len() => break n - 1,
            _ => {
                println!(
                    "  {}",
                    format!("Please enter a number between 1 and {}", providers.len()).red()
                );
            }
        }
    };

    let preset = &providers[choice];
    let mut api_base = preset.api_base.to_string();
    let mut api_key_value: Option<String> = None;
    let mut credential_store = CredentialStore::SettingsFile;

    // --- Custom provider: ask for API base ---
    if preset.name.starts_with("Custom") {
        println!();
        print!("  {} API base URL [{}]: ", "▶".cyan(), preset.api_base);
        io::stdout().flush().ok();
        let base = read_line_default(preset.api_base)?;
        api_base = base.trim().to_string();
    }

    // --- API key (skip for local providers) ---
    if preset.needs_key {
        // Check if env var is already set
        let env_key = std::env::var("RUSTY_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .filter(|k| !k.is_empty());

        if let Some(ref key) = env_key {
            println!();
            let masked = if key.len() > 8 {
                format!("{}...{}", &key[..4], &key[key.len() - 4..])
            } else {
                "****".to_string()
            };
            println!(
                "  {} {} {}",
                "✓".green(),
                "API key detected from environment variable:".dark_grey(),
                masked.dark_grey()
            );
            api_key_value = Some(key.clone());
        } else {
            let keyring_available = CredentialManager::is_keyring_available();

            println!();
            if keyring_available {
                println!(
                    "  {}",
                    "Where would you like to store your API key?"
                        .bold()
                        .white()
                );
                println!(
                    "    {} OS Keychain / Credential Manager {}",
                    "[1]".cyan(),
                    "(recommended)".dark_grey()
                );
                println!(
                    "    {} Settings file (~/.rusty/settings.json)",
                    "[2]".cyan()
                );
                println!();
                let store_choice = loop {
                    print!("  {} ", "▶".cyan());
                    io::stdout().flush().ok();
                    let input = read_line_default("1")?;
                    match input.trim() {
                        "1" => break CredentialStore::Keyring,
                        "2" => break CredentialStore::SettingsFile,
                        _ => println!("  {}", "Please enter 1 or 2".red()),
                    }
                };
                credential_store = store_choice;
            } else {
                println!(
                    "  {}",
                    "No OS keyring detected. API key will be stored in settings file."
                        .dark_grey()
                );
                credential_store = CredentialStore::SettingsFile;
            }

            println!();
            println!(
                "  {}",
                "Enter your API key (input is hidden):".bold().white()
            );
            print!("  {} ", "▶".cyan());
            io::stdout().flush().ok();

            let key = mask_input()?;

            if key.is_empty() {
                return Err(RustyError::Config("API key is required.".to_string()));
            }

            api_key_value = Some(key);
        }
    }

    // --- Model selection ---
    println!();
    println!(
        "  {}",
        format!("Default model: {}", preset.default_model).dark_grey()
    );
    print!("  {} Model [{}]: ", "▶".cyan(), preset.default_model);
    io::stdout().flush().ok();
    let model = read_line_default(preset.default_model)?;

    // --- Connectivity test ---
    if preset.needs_key {
        if let Some(ref key) = api_key_value {
            println!();
            print!("  {} Testing connection... ", "⏳".to_string());
            io::stdout().flush().ok();

            match test_connectivity(&api_base, key, &model).await {
                Ok(duration) => {
                    println!(
                        "{}",
                        format!("OK ({:.1}s)", duration.as_secs_f64()).green()
                    );
                }
                Err(e) => {
                    println!("{}", format!("Failed: {e}").red());
                    println!(
                        "  {}",
                        "You can still save the config and try again later.".dark_grey()
                    );
                    print!("  {} Continue anyway? [Y/n]: ", "▶".cyan());
                    io::stdout().flush().ok();
                    let answer = read_line_default("y")?;
                    if !answer.trim().eq_ignore_ascii_case("y") {
                        println!("  {}", "Setup cancelled.".yellow());
                        return Ok(false);
                    }
                }
            }
        }
    }

    // --- Store API key ---
    #[allow(unused_variables)]
    if let Some(ref _key) = api_key_value {
        match credential_store {
            CredentialStore::Keyring => {
                #[cfg(feature = "os-keyring")]
                {
                    CredentialManager::store_in_keyring(_key)?;
                    println!(
                        "  {} API key stored in OS Keychain / Credential Manager.",
                        "✓".green()
                    );
                }
                #[cfg(not(feature = "os-keyring"))]
                {
                    // Unreachable: without the os-keyring feature, is_keyring_available()
                    // always returns false, so the wizard never offers Keyring as a choice.
                    unreachable!("Keyring credential store selected without os-keyring feature");
                }
            }
            CredentialStore::SettingsFile => {
                // Will be saved into settings.json below alongside other config
                println!("  {} API key will be saved in settings file.", "✓".green());
            }
        }
    }

    // --- Save configuration ---
    let path = Config::settings_path();
    let mut settings = if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| RustyError::Config(format!("Failed to read settings: {e}")))?;
        serde_json::from_str::<Settings>(&content).unwrap_or_default()
    } else {
        Settings::default()
    };

    settings.api_base = Some(api_base.trim_end_matches('/').to_string());
    settings.default_model = Some(model.trim().to_string());
    // Only store api_key in settings file when using settings file store
    if credential_store == CredentialStore::SettingsFile {
        settings.api_key = api_key_value;
    }

    settings.credential_store = credential_store;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RustyError::Config(format!("Failed to create config dir: {e}")))?;
    }
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| RustyError::Config(format!("Failed to serialize settings: {e}")))?;
    std::fs::write(&path, json)
        .map_err(|e| RustyError::Config(format!("Failed to write settings: {e}")))?;

    println!();
    println!("  {} {}", "✓".green(), "Configuration saved!".bold().green());
    println!();
    Ok(true)
}

/// Lightweight connectivity test.
///
/// Makes a minimal chat completion request to verify the API key is valid
/// and the endpoint is reachable.
async fn test_connectivity(
    api_base: &str,
    api_key: &str,
    model: &str,
) -> Result<std::time::Duration, RustyError> {
    let url = format!("{}/chat/completions", api_base.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 1,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| RustyError::Http(format!("Failed to create HTTP client: {e}")))?;

    let start = std::time::Instant::now();

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| RustyError::Http(format!("Connection failed: {e}")))?;

    let status = resp.status();
    let elapsed = start.elapsed();

    if status.is_success() {
        Ok(elapsed)
    } else {
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            Err(RustyError::Auth(format!(
                "Authentication failed ({}): check your API key",
                status.as_u16()
            )))
        } else {
            Err(RustyError::Http(format!(
                "HTTP {} — {}",
                status.as_u16(),
                truncate(&text, 200)
            )))
        }
    }
}

/// Read a line from stdin, using `default` if the user just presses Enter.
fn read_line_default(default: &str) -> Result<String, RustyError> {
    let stdin = io::stdin();
    let mut line = String::new();
    stdin
        .lock()
        .read_line(&mut line)
        .map_err(|e| RustyError::Config(format!("Input error: {e}")))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Read input with password masking (characters displayed as `*`).
fn mask_input() -> Result<String, RustyError> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    enable_raw_mode().map_err(|e| RustyError::Config(format!("Failed to enable raw mode: {e}")))?;

    let result = (|| -> io::Result<String> {
        let mut input = String::new();
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        loop {
            if crossterm::event::poll(std::time::Duration::from_secs(300))? {
                if let crossterm::event::Event::Key(key_event) = crossterm::event::read()? {
                    match key_event.code {
                        crossterm::event::KeyCode::Enter => {
                            write!(stdout, "\r\n")?;
                            stdout.flush()?;
                            break;
                        }
                        crossterm::event::KeyCode::Char(c) => {
                            input.push(c);
                            write!(stdout, "*")?;
                            stdout.flush()?;
                        }
                        crossterm::event::KeyCode::Backspace => {
                            if input.pop().is_some() {
                                // Clear the line and reprint masked input
                                write!(stdout, "\r")?;
                                for _ in 0..100 {
                                    write!(stdout, " ")?;
                                }
                                write!(stdout, "\r  ▶ ")?;
                                for _ in 0..input.len() {
                                    write!(stdout, "*")?;
                                }
                                stdout.flush()?;
                            }
                        }
                        crossterm::event::KeyCode::Esc => {
                            input.clear();
                            write!(stdout, "\r\n")?;
                            stdout.flush()?;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(input)
    })();

    disable_raw_mode().map_err(|e| RustyError::Config(format!("Failed to disable raw mode: {e}")))?;

    result.map_err(|e| RustyError::Config(format!("Input error: {e}")))
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_presets_count() {
        assert_eq!(ProviderPreset::all().len(), 6);
    }

    #[test]
    fn provider_presets_have_names() {
        let presets = ProviderPreset::all();
        assert_eq!(presets[0].name, "Xiaomi (MiMo)");
        assert_eq!(presets[1].name, "Kimi (Moonshot)");
        assert_eq!(presets[2].name, "OpenAI");
        assert_eq!(presets[3].name, "DeepSeek");
        assert_eq!(presets[4].name, "Ollama (local)");
        assert!(presets[5].name.starts_with("Custom"));
    }

    #[test]
    fn ollama_does_not_need_key() {
        let presets = ProviderPreset::all();
        let ollama = presets.iter().find(|p| p.name.contains("Ollama")).unwrap();
        assert!(!ollama.needs_key);
    }

    #[test]
    fn all_cloud_providers_need_key() {
        let presets = ProviderPreset::all();
        for p in &presets {
            if p.name.contains("Ollama") {
                continue;
            }
            assert!(p.needs_key, "{} should need an API key", p.name);
        }
    }

    #[test]
    fn provider_api_bases_are_well_formed() {
        let presets = ProviderPreset::all();
        for p in &presets {
            assert!(
                p.api_base.starts_with("http://") || p.api_base.starts_with("https://"),
                "{} api_base should start with http(s)://: {}",
                p.name,
                p.api_base
            );
        }
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let result = truncate("hello world this is a long string", 10);
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate("hello", 5), "hello");
    }
}
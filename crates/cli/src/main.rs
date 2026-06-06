// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use crossterm::event::{EnableBracketedPaste, DisableBracketedPaste};
use ratatui::{backend::CrosstermBackend, Terminal};
use rusty_agent::{Agent, AgentCallbacks};
use rusty_core::CancelToken;
use rusty_core::plan::Plan;
use rusty_core::permissions::{PermissionDecision, PermissionRequest};
use rusty_core::ContentBlock;
use rusty_core::{Config, ConversationSession, CredentialManager, PermissionMode, Settings};
use rusty_core::setup_wizard::{run_setup_wizard, is_first_run};
use rusty_core::config::{ensure_restricted_dir, set_restrictive_file_permissions};
use rusty_provider::ProviderConfig;
use rusty_tools::{all_tools, Tool};
use rusty_keymap as keymap_lib;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "rusty", version, about = "Lightweight AI coding agent")]
struct Args {
    /// Initial prompt (non-interactive mode)
    #[arg(short = 'p', long)]
    prompt: Option<String>,

    /// Model to use
    #[arg(short, long)]
    model: Option<String>,

    /// API base URL
    #[arg(long)]
    api_base: Option<String>,

    /// API provider preset (xiaomi, kimi, openai, ollama, deepseek)
    #[arg(long)]
    preset: Option<String>,

    /// API key
    #[arg(long, env = "OPENAI_API_KEY")]
    api_key: Option<String>,

    /// Working directory
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Permission mode
    #[arg(long, value_enum, default_value = "default")]
    permissions: PermissionModeArg,

    /// Plan mode with task tracking (implies --permissions plan, instructs model to use todowrite)
    #[arg(long)]
    plan_with_tasks: bool,

    /// Resume session by ID
    #[arg(long)]
    resume: Option<String>,

    /// List saved sessions
    #[arg(long)]
    list_sessions: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Max turns before stopping
    #[arg(long)]
    max_turns: Option<u32>,

    /// Max tokens per response
    #[arg(long)]
    max_tokens: Option<u32>,

    /// Temperature
    #[arg(long)]
    temperature: Option<f32>,

    /// Thinking/reasoning token budget
    #[arg(long)]
    thinking_budget: Option<u32>,

    /// Thinking level (minimal, normal, deep)
    #[arg(long, value_enum)]
    thinking_level: Option<ThinkingLevelArg>,

    /// No TUI, just print responses
    #[arg(long)]
    headless: bool,

    /// Run the interactive first-run setup wizard
    #[arg(long)]
    setup: bool,

    /// Path to a JSON keymap file for custom key bindings
    #[arg(long)]
    keymap: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum PermissionModeArg {
    Default,
    AcceptEdits,
    Bypass,
    Plan,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ThinkingLevelArg {
    Minimal,
    Normal,
    Deep,
}

impl From<ThinkingLevelArg> for rusty_core::ThinkingLevel {
    fn from(arg: ThinkingLevelArg) -> Self {
        match arg {
            ThinkingLevelArg::Minimal => rusty_core::ThinkingLevel::Minimal,
            ThinkingLevelArg::Normal => rusty_core::ThinkingLevel::Normal,
            ThinkingLevelArg::Deep => rusty_core::ThinkingLevel::Deep,
        }
    }
}

impl From<PermissionModeArg> for PermissionMode {
    fn from(arg: PermissionModeArg) -> Self {
        match arg {
            PermissionModeArg::Default => PermissionMode::Default,
            PermissionModeArg::AcceptEdits => PermissionMode::AcceptEdits,
            PermissionModeArg::Bypass => PermissionMode::BypassPermissions,
            PermissionModeArg::Plan => PermissionMode::Plan,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Determine log directory (RUSTY_LOG_DIR env, or platform-appropriate cache dir)
    let log_dir = std::env::var("RUSTY_LOG_DIR").unwrap_or_else(|_| {
        dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("rusty")
            .to_string_lossy()
            .into_owned()
    });
    // Best-effort directory creation — fall back to /tmp if it fails
    let _ = ensure_restricted_dir(std::path::Path::new(&log_dir));
    let log_path = std::path::Path::new(&log_dir).join("debug.log");

    // Open log file in append mode; fall back to stderr if file can't be opened
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    // Set restrictive permissions on the log file
    if log_file.is_some() {
        set_restrictive_file_permissions(&log_path);
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    match log_file {
        Some(_file) => {
            let log_path_clone = log_path.clone();
            tracing_subscriber::fmt()
                .with_writer(move || {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path_clone)
                        .unwrap()
                })
                .with_env_filter(filter)
                .init();
        }
        None => {
            // Last resort: stderr (headless mode may tolerate this)
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(filter)
                .init();
        }
    }

    let args = Args::parse();

    // Add a startup log line so we know where logs are written
    info!("Log file: {}", log_path.display());

    // Handle --setup or auto-detect first run
    let needs_setup = args.setup || is_first_run();
    if needs_setup {
        if !args.setup {
            eprintln!("No configuration found. Starting setup wizard...");
            eprintln!();
        }
        let completed = run_setup_wizard().await?;
        if !completed {
            // User cancelled the wizard
            eprintln!("Setup cancelled.");
            std::process::exit(1);
        }
        // Re-load settings after wizard saved them
        // If wizard completed successfully, we have credentials; continue to normal flow
        // unless --setup was explicit (then exit after wizard)
        if args.setup {
            // Also auto-detect first run: if no settings file existed, the wizard
            // already created it. If it was explicitly --setup, just exit.
            eprintln!("Setup complete! Run `rusty` to start using the agent.");
            return Ok(());
        }
    }

    let working_dir = args
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let sessions_dir = rusty_core::Config::sessions_dir(&working_dir);

    // Handle --list-sessions early exit
    if args.list_sessions {
        // Run session cleanup in background before listing
        let _ = rusty_core::ConversationSession::cleanup(&sessions_dir).await;
        let sessions = rusty_core::ConversationSession::list(&sessions_dir).await?;
        if sessions.is_empty() {
            println!("No saved sessions.");
        } else {
            println!("Saved sessions:");
            for s in &sessions {
                let msg_count = s.messages.len();
                let preview = s
                    .messages
                    .last()
                    .map(|m| {
                        let text = m.get_all_text();
                        if text.len() > 60 {
                            let safe = text.floor_char_boundary(60);
                            format!("{}...", &text[..safe])
                        } else {
                            text
                        }
                    })
                    .unwrap_or_default();
                println!(
                    "  {} | {} msgs | {} | {}",
                    &s.id[..8],
                    msg_count,
                    s.model,
                    s.updated_at.format("%Y-%m-%d %H:%M")
                );
                if !preview.is_empty() {
                    println!("    {preview}");
                }
            }
        }
        return Ok(());
    }

    // Load config
    let settings = Settings::load().await.unwrap_or_default();
    let mut config = Config::default();

    // Fire-and-forget session cleanup: remove sessions older than 30 days
    let cleanup_dir = sessions_dir.clone();
    tokio::spawn(async move {
        let _ = rusty_core::ConversationSession::cleanup(&cleanup_dir).await;
    });

    // Apply preset first (can be overridden by explicit flags)
    if let Some(preset) = &args.preset {
        match preset.as_str() {
            "xiaomi" | "mimo" => {
                config.api_base = Some("https://token-plan-cn.xiaomimimo.com/v1".to_string());
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "mimo-v2.5-pro".to_string();
                }
            }
            "kimi" | "moonshot" => {
                config.api_base = Some("https://api.kimi.com/coding/v1/".to_string());
                config.provider_type = rusty_core::ProviderType::Anthropic;
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "kimi-k2.6".to_string();
                }
            }
            "openai" => {
                config.api_base = Some("https://api.openai.com/v1".to_string());
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "gpt-4o".to_string();
                }
            }
            "deepseek" => {
                config.api_base = Some("https://api.deepseek.com".to_string());
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "deepseek-v4-pro".to_string();
                }
            }
            "ollama" => {
                config.api_base = Some("http://localhost:11434/v1".to_string());
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "llama3".to_string();
                }
            }
            _ => {
                eprintln!("Unknown preset: {preset}");
                eprintln!("Available presets: xiaomi, kimi, openai, deepseek, ollama");
                std::process::exit(1);
            }
        }
    }

    // Apply settings/registry defaults (model registry takes priority over legacy fields)
    let active_entry = settings.active_model_entry().cloned();
    let permanent_allowlist = settings.allowed_tools_set();

    if let Some(ref entry) = active_entry {
        // Use model registry as the primary source of defaults
        if config.api_base.is_none() {
            config.api_base = Some(entry.api_base.clone());
        }
        config.model = entry.model.clone();
        if args.max_tokens.is_none() {
            config.max_tokens = entry.max_tokens;
        }
        if args.temperature.is_none() {
            config.temperature = entry.temperature;
        }
        if args.thinking_budget.is_none() {
            config.thinking_budget = entry.thinking_budget;
        }
        if config.context_window.is_none() {
            config.context_window = entry.context_window;
        }
    } else {
        // Legacy flat settings — only fill gaps not set by preset/CLI
        if config.api_base.is_none() {
            if let Some(base) = args.api_base.clone().or(settings.api_base.clone()) {
                config.api_base = Some(base);
            }
        }
        if let Some(model) = settings.default_model.clone() {
            config.model = model;
        }
    }

    // CLI overrides (always win over presets, registry, and legacy settings)
    if let Some(model) = args.model {
        config.model = model;
    }
    if let Some(max_tokens) = args.max_tokens {
        config.max_tokens = max_tokens;
    }
    if let Some(max_turns) = args.max_turns {
        config.max_turns = max_turns;
    }
    if let Some(temp) = args.temperature {
        config.temperature = Some(temp);
    }
    if let Some(budget) = args.thinking_budget {
        config.thinking_budget = Some(budget);
    }
    if let Some(level) = args.thinking_level {
        config.thinking_level = Some(level.into());
    }
    config.verbose = args.verbose;
    config.permission_mode = args.permissions.into();
    if args.plan_with_tasks {
        config.permission_mode = PermissionMode::Plan;
        config.plan_with_tasks = true;
    }

    // Build provider — resolve API key in priority order:
    //   1. --api-key CLI flag (highest priority)
    //   2. Per-model key from registry (model_registry[name].api_keys)
    //   3. CredentialManager (env var → keyring → settings.api_key)
    //   4. Auto-launch setup wizard if nothing found
    let api_key = args.api_key
        .or_else(|| if active_entry.is_some() { settings.resolve_active_api_key() } else { None })
        .or_else(|| CredentialManager::resolve_api_key(&settings));
    let api_base = config.resolve_api_base();

    let api_key = match api_key {
        Some(key) => key,
        None => {
            eprintln!("No API key configured.");
            eprintln!();
            eprintln!("Let's run the setup wizard to configure your provider.");
            eprintln!();

            // Auto-launch setup wizard when no key is found
            let completed = run_setup_wizard().await?;
            if !completed {
                std::process::exit(1);
            }

            // Reload settings after wizard and re-resolve
            let new_settings = Settings::load().await.unwrap_or_default();
            let resolved = CredentialManager::resolve_api_key(&new_settings);
            match resolved {
                Some(key) => key,
                None => {
                    eprintln!("Error: Still no API key after setup. Please check your configuration.");
                    std::process::exit(1);
                }
            }
        }
    };

    // Create provider via factory — routes to the right implementation
    // based on the model entry's ProviderType.
    // We use `config` (not the raw entry) so that CLI overrides (--model, --api-base)
    // are respected even when a registry entry is active.
    let provider: Arc<dyn rusty_provider::LlmProvider> = if let Some(ref entry) = active_entry {
        rusty_provider::create_provider(
            entry.provider,
            ProviderConfig {
                api_key,
                api_base: config.api_base.clone().unwrap_or_else(|| entry.api_base.clone()),
                model: config.model.clone(),
                max_tokens: config.max_tokens,
                temperature: config.temperature,
                thinking_budget: config.thinking_budget,
                extra_headers: entry.extra_headers.clone(),
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        // Legacy path: no registry entry, build config from flat fields
        rusty_provider::create_provider(
            config.provider_type,
            ProviderConfig {
                api_key,
                api_base,
                model: config.model.clone(),
                max_tokens: config.max_tokens,
                temperature: config.temperature,
                thinking_budget: config.thinking_budget,
                extra_headers: None,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?
    };

    // Build system prompt (must happen before make_agent_tool so sub-agents
    // receive the full context including AGENTS.md/CLAUDE.md, platform info, etc.)
    // Load stored memories for injection into system prompt
    let project_memory = rusty_core::memory::ProjectMemory::load_for_project(&working_dir)
        .await
        .unwrap_or_else(|_| rusty_core::memory::ProjectMemory::new(working_dir.to_string_lossy().to_string()));
    let memory_context = project_memory.format_for_context();
    let memory_context_opt = if memory_context.is_empty() {
        None
    } else {
        Some(memory_context.clone())
    };

    // Create plan for task management (shared between TodoWriteTool and Agent system prompt)
    let plan = Arc::new(tokio::sync::Mutex::new(Plan::new(working_dir.to_string_lossy().to_string())));

    let system_prompt = rusty_agent::build_system_prompt(
        &config,
        &working_dir,
        memory_context_opt.as_deref(),
        None, // plan state is injected dynamically by refresh_system_prompt before each LLM call
    )
    .await;

    // Build tools (including agent tool, memory tool, and plan tool)
    let mut tools: Vec<Box<dyn Tool>> = all_tools();
    let memory_tool = rusty_tools::memory::MemoryTool::new(project_memory);
    tools.push(Box::new(memory_tool));

    let plan_tool = rusty_tools::todowrite::TodoWriteTool::new(plan.clone());
    tools.push(Box::new(plan_tool));

    // Add agent tool with spawn function — uses the full system prompt so
    // sub-agents inherit the same context (AGENTS.md, CLAUDE.md, git info, etc.)
    let agent_tool = rusty_agent::make_agent_tool(
        provider.clone(),
        system_prompt.clone(),
        config.clone(),
    );
    tools.push(Box::new(agent_tool));

    info!("Model: {}", config.model);
    info!("Working directory: {}", working_dir.display());

    // Load or create session
    let mut agent = if let Some(session_id) = &args.resume {
        if let Some(session) = rusty_core::ConversationSession::load(&sessions_dir, session_id).await? {
            info!("Resumed session: {}", session.id);
            let mut a = Agent::new(
                provider.clone(),
                tools,
                config.clone(),
                working_dir.clone(),
                system_prompt.clone(),
            );
            // Restore messages
            for msg in session.messages {
                a.messages_mut().push(msg);
            }
            a
        } else {
            info!("Session not found, starting new");
            Agent::new(
                provider.clone(),
                tools,
                config.clone(),
                working_dir.clone(),
                system_prompt,
            )
        }
    } else {
        Agent::new(
            provider.clone(),
            tools,
            config.clone(),
            working_dir.clone(),
            system_prompt,
        )
    };
    agent.set_permission_mode(config.permission_mode);
    agent.set_plan(plan.clone());

    // Run mode
    if let Some(prompt) = args.prompt {
        // Non-interactive mode
        run_headless(&mut agent, &prompt).await?;
    } else if args.headless {
        // Headless mode with stdin
        run_headless_stdin(&mut agent, &config.model, &sessions_dir).await?;
    } else {
        // Interactive TUI mode — moves agent into spawned task, handles session save internally
        // Load keymap if --keymap was specified
        let keymap = if let Some(ref keymap_path) = args.keymap {
            load_keymap(keymap_path)
        } else {
            None
        };
        run_tui(agent, &config.model, permanent_allowlist, &config, &working_dir, &log_path, settings, &sessions_dir, keymap, plan.clone()).await?;
        return Ok(());
    }

    // Save session (headless modes only — TUI saves internally)
    let session = rusty_core::ConversationSession {
        id: uuid::Uuid::new_v4().to_string(),
        name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        messages: agent.messages().to_vec(),
        model: config.model,
        working_dir: working_dir.display().to_string(),
    };
    session.save(&sessions_dir).await?;
    info!("Session saved: {}", session.id);

    Ok(())
}

async fn run_headless(agent: &mut Agent, prompt: &str) -> Result<()> {
    let text_cb: rusty_agent::r#loop::TextCallback = Box::new(|text| {
        print!("{text}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    });

    let result = agent
        .run(
            vec![ContentBlock::Text { text: prompt.to_string() }],
            AgentCallbacks {
                on_text: Some(&text_cb),
                ..Default::default()
            },
        )
        .await?;
    if !result.ends_with('\n') {
        println!();
    }
    Ok(())
}

async fn run_headless_stdin(agent: &mut Agent, model: &str, sessions_dir: &Path) -> Result<()> {
    use std::io::{self, BufRead};
    let stdin = io::stdin();
    let mut _session_name: Option<String> = None;

    println!("rusty (headless mode). Type 'exit' or Ctrl-D to quit.");
    println!("Slash commands: /help, /init, /resume, /sessions, /compact, /clear, /copy, /model, /rename, /quit");

    loop {
        print!("> ");
        use std::io::Write;
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line == "exit" || line == "quit" {
            break;
        }
        if line.is_empty() {
            continue;
        }

        // Handle slash commands in headless mode
        if line.starts_with('/') {
            match rusty_tui::app::SlashCommand::parse(line) {
                Some(rusty_tui::app::SlashCommand::Help) => {
                    println!("Available commands:");
                    for (cmd, desc) in rusty_tui::app::SlashCommand::all_descriptions() {
                        println!("  {:12} {}", cmd, desc);
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Quit) => break,
                Some(rusty_tui::app::SlashCommand::Sessions) => {
                    let sessions = ConversationSession::list(sessions_dir).await?;
                    if sessions.is_empty() {
                        println!("No saved sessions.");
                    } else {
                        for s in &sessions {
                            println!(
                                "  {} | {} msgs | {} | {}",
                                &s.id[..8],
                                s.messages.len(),
                                s.model,
                                s.updated_at.format("%Y-%m-%d %H:%M")
                            );
                        }
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Init) => {
                    let init_prompt = build_init_prompt();
                    // Fall through to send as regular prompt
                    let text_cb: rusty_agent::r#loop::TextCallback = Box::new(|text| {
                        print!("{text}");
                        let _ = io::stdout().flush();
                    });
                    let result = agent
                        .run(
                            vec![ContentBlock::Text { text: init_prompt }],
                            AgentCallbacks {
                                on_text: Some(&text_cb),
                                ..Default::default()
                            },
                        )
                        .await?;
                    if !result.ends_with('\n') {
                        println!();
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Compact) => {
                    match agent.compact().await {
                        Ok(true) => println!("Conversation compacted."),
                        Ok(false) => println!("Not enough messages to compact (need at least 4)."),
                        Err(e) => println!("Compaction failed: {e}"),
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Clear) => {
                    agent.messages_mut().clear();
                    println!("Conversation cleared.");
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Copy) => {
                    // Find last assistant message
                    let last = agent.messages().iter().rev().find(|m| m.role == rusty_core::Role::Assistant);
                    match last {
                        Some(msg) => {
                            let text = msg.get_all_text();
                            match arboard::Clipboard::new() {
                                Ok(mut clipboard) => {
                                    match clipboard.set_text(&text) {
                                        Ok(_) => println!("Copied last response to clipboard."),
                                        Err(e) => println!("Failed to copy: {e}"),
                                    }
                                }
                                Err(e) => println!("Clipboard unavailable: {e}"),
                            }
                        }
                        None => println!("No assistant response to copy."),
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Model) => {
                    println!("Current model: {model}");
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Rename) => {
                    let new_name = line.strip_prefix("/rename").unwrap_or("").trim();
                    if new_name.is_empty() {
                        println!("Usage: /rename <new session name>");
                    } else {
                        _session_name = Some(new_name.to_string());
                        println!("Session renamed to: {new_name}");
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Permissions) => {
                    let settings = rusty_core::Settings::load().await.unwrap_or_default();
                    let args = line.strip_prefix("/permissions").or_else(|| line.strip_prefix("/perms")).unwrap_or("").trim();
                    if args.starts_with("remove ") {
                        let tool_key = args.strip_prefix("remove ").unwrap().trim();
                        if tool_key.is_empty() {
                            println!("Usage: /permissions remove <tool_key>");
                        } else {
                            match rusty_core::remove_permanent_permission(tool_key).await {
                                Ok(true) => println!("Removed '{tool_key}' from always-approve list."),
                                Ok(false) => println!("'{tool_key}' was not in the always-approve list."),
                                Err(e) => println!("Error: {e}"),
                            }
                        }
                    } else if settings.allowed_tools.is_empty() {
                        println!("No tools in always-approve list.");
                    } else {
                        println!("Always-approved tools:");
                        for tool in &settings.allowed_tools {
                            println!("  • {tool}");
                        }
                        println!("\nUse /permissions remove <tool_key> to revoke.");
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Resume) => {
                    let sessions = ConversationSession::list(sessions_dir).await?;
                    if sessions.is_empty() {
                        println!("No saved sessions to resume.");
                    } else {
                        println!("Available sessions:");
                        for (i, s) in sessions.iter().enumerate() {
                            println!(
                                "  [{}] {} | {} msgs | {}",
                                i,
                                &s.id[..8],
                                s.messages.len(),
                                s.updated_at.format("%Y-%m-%d %H:%M")
                            );
                        }
                        print!("Enter session number (or empty to cancel): ");
                        use std::io::Write;
                        io::stdout().flush()?;
                        let mut choice = String::new();
                        stdin.lock().read_line(&mut choice)?;
                        if let Ok(idx) = choice.trim().parse::<usize>() {
                            if idx < sessions.len() {
                                agent.messages_mut().clear();
                                for msg in &sessions[idx].messages {
                                    agent.messages_mut().push(msg.clone());
                                }
                                println!("Resumed session {}.", &sessions[idx].id[..8]);
                            }
                        }
                    }
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Settings) => {
                    println!("Settings panel is not available in headless mode. Edit ~/.rusty/settings.json directly.");
                    continue;
                }
                Some(rusty_tui::app::SlashCommand::Version) => {
                    let current = rusty_core::update::current_version();
                    println!("rusty v{current}");
                    match rusty_core::update::check_for_update().await {
                        Ok(Some(update)) => {
                            println!("Update available: v{}", update.latest_version);
                            println!("https://github.com/PDG-Global/rusty/releases");
                        }
                        Ok(None) => {
                            println!("You are running the latest version.");
                        }
                        Err(e) => {
                            println!("Could not check for updates: {e}");
                        }
                    }
                    continue;
                }
                None => {
                    println!("Unknown command: {line}. Type /help for available commands.");
                    continue;
                }
            }
        }

        let text_cb: rusty_agent::r#loop::TextCallback = Box::new(|text| {
            print!("{text}");
            let _ = io::stdout().flush();
        });

        let result = agent
            .run(
                vec![ContentBlock::Text { text: line.to_string() }],
                AgentCallbacks {
                    on_text: Some(&text_cb),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

/// Build the prompt for /init — instructs the agent to analyze the codebase and write AGENTS.md
fn build_init_prompt() -> String {
    r#"Write an AGENTS.md file at the project root that describes this repository.

CRITICAL RULES:
- First, explore the repository. Use glob, file_read, grep, and bash to discover every file and understand what they contain.
- Describe ONLY what actually exists. Do not invent, assume, or hallucinate features, modules, architectures, or tools that are not present in the files.
- If the repository contains a single SQLite database and nothing else, say exactly that. Do not describe an application that doesn't exist.
- If there is no build system, no tests, no modules, no dependencies file — do not create sections for them.
- Every claim in AGENTS.md must be traceable to an actual file or directory you inspected.

APPROACH:
1. List all files in the repository (glob **/* and check hidden files).
2. Read each file to understand its purpose and contents.
3. Write AGENTS.md based strictly on what you found.

The file should help any AI agent or developer understand this repository. Use whatever sections are appropriate for what's actually here — do not force a template. Write in plain, direct language. No emojis. No filler. Factual and concise."#.to_string()
}

/// A permission request bundled with a oneshot sender for the response.
struct PermissionPromptMsg {
    request: PermissionRequest,
    respond: oneshot::Sender<PermissionDecision>,
}

/// Events from the agent task to the TUI.
enum AgentTaskEvent {
    /// Regular agent event (text, tool, etc.)
    Event(rusty_tui::app::AgentEvent),
    /// Permission request — TUI should show prompt and send response on the oneshot.
    PermissionRequest(PermissionPromptMsg),
    /// Agent has finished processing a message and is ready for more input
    ReadyForInput,
}

/// Load a keymap from a JSON file path. Returns None on failure with a warning to stderr.
fn load_keymap(path: &std::path::Path) -> Option<keymap_lib::KeyMap> {
    match std::fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<keymap_lib::BindingConfig>(&json) {
            Ok(config) => Some(keymap_lib::KeyMap::from_config(&config)),
            Err(e) => {
                eprintln!("Warning: failed to parse keymap {}: {e}", path.display());
                None
            }
        },
        Err(e) => {
            eprintln!("Warning: failed to read keymap {}: {e}", path.display());
            None
        }
    }
}

#[allow(unused_variables)]
async fn run_tui(
    agent: Agent,
    model: &str,
    permanent_allowlist: HashSet<String>,
    config: &Config,
    working_dir: &PathBuf,
    log_path: &std::path::Path,
    mut settings: Settings,
    sessions_dir: &Path,
    keymap: Option<keymap_lib::KeyMap>,
    plan: Arc<tokio::sync::Mutex<Plan>>,
) -> Result<()> {
    use rusty_core::Message;

    // ── OS-level stderr redirect (Unix only) ──────────────────────────────
    // Save the real stderr fd so we can restore it on exit.  Then point fd 2
    // at the log file.  This captures *all* writes to stderr — from tracing,
    // log, eprintln!(), or raw fd 2 writes from any dependency — and sends
    // them to the log file instead of corrupting the TUI.
    #[cfg(unix)]
    let saved_stderr_fd: i32 = unsafe { libc::dup(libc::STDERR_FILENO) };
    #[cfg(not(unix))]
    let _saved_stderr_fd: i32 = -1;
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        if let Ok(path_cstr) = CString::new(log_path.as_os_str().as_bytes()) {
            let log_fd = unsafe {
                libc::open(
                    path_cstr.as_ptr(),
                    libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                    0o644,
                )
            };
            if log_fd >= 0 {
                unsafe { libc::dup2(log_fd, libc::STDERR_FILENO) };
                unsafe { libc::close(log_fd) };
            }
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    // Install panic hook to restore terminal + stderr on crash.
    // The flag prevents the hook from yanking the terminal out of raw/alternate
    // mode while the TUI render loop is still running on another thread.
    let tui_active = Arc::new(AtomicBool::new(false));
    let tui_active_hook = tui_active.clone();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Restore real stderr so the panic message is visible
        #[cfg(unix)]
        if saved_stderr_fd >= 0 {
            unsafe { libc::dup2(saved_stderr_fd, libc::STDERR_FILENO) };
            unsafe { libc::close(saved_stderr_fd) };
        }

        // Only restore terminal if the TUI render loop is NOT active.
        // When the TUI is running, the main loop will detect the crashed agent
        // task and handle cleanup gracefully.
        if !tui_active_hook.load(Ordering::SeqCst) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste);
        }
        original_hook(info);
    }));
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel for TUI commands → agent task
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<rusty_tui::app::TuiCommand>();
    // Channel for agent → TUI events (including permission requests)
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentTaskEvent>();
    // Channel for agent task to return its message history for session save
    let (msg_return_tx, msg_return_rx) = oneshot::channel::<Vec<Message>>();

    // Spawn the agent task
    let perm_mode = config.permission_mode;
    let agent = Arc::new(tokio::sync::Mutex::new(agent));
    let update_event_tx = event_tx.clone(); // clone before move into agent task
    let agent_handle = tokio::spawn({
        let agent_arc = agent.clone();
        async move {
            let mut agent = agent_arc.lock().await;
            agent.set_permission_mode(perm_mode);
            agent.set_permanent_allowlist(permanent_allowlist);

            // Set up permission callback
            let event_tx_cb = event_tx.clone();
            let perm_cb: rusty_agent::PermissionCallback = Arc::new(move |request| {
                let tx = event_tx_cb.clone();
                Box::pin(async move {
                    let (resp_tx, resp_rx) = oneshot::channel();
                    let _ = tx.send(AgentTaskEvent::PermissionRequest(PermissionPromptMsg {
                        request,
                        respond: resp_tx,
                    }));
                    resp_rx.await.unwrap_or(PermissionDecision::Deny("Channel closed".into()))
                })
            });
            agent.set_permission_callback(perm_cb);
            drop(agent);

            let agent_arc = agent_arc;

            // Helper to start an agent.run() in a spawned task
            fn start_run(
                agent: Arc<tokio::sync::Mutex<Agent>>,
                event_tx: mpsc::UnboundedSender<AgentTaskEvent>,
                input: Vec<ContentBlock>,
                cancel: CancelToken,
            ) -> tokio::task::JoinHandle<()> {
                tokio::spawn(async move {
                    let mut agent = agent.lock().await;

                    let tx_text = event_tx.clone();
                    let text_cb: rusty_agent::r#loop::TextCallback = Box::new(move |text: &str| {
                        let _ = tx_text.send(AgentTaskEvent::Event(
                            rusty_tui::app::AgentEvent::TextDelta(text.to_string()),
                        ));
                    });
                    let tx_think = event_tx.clone();
                    let thinking_cb: rusty_agent::r#loop::ThinkingCallback = Box::new(move |text: &str| {
                        let _ = tx_think.send(AgentTaskEvent::Event(
                            rusty_tui::app::AgentEvent::ThinkingDelta(text.to_string()),
                        ));
                    });
                    let tx_tool = event_tx.clone();
                    let tool_cb: rusty_agent::r#loop::ToolCallback = Box::new(move |name: &str, status: rusty_agent::ToolStatus| {
                        let event = match status {
                            rusty_agent::ToolStatus::Running { arguments } => {
                                rusty_tui::app::AgentEvent::ToolStart {
                                    name: name.to_string(),
                                    arguments,
                                }
                            }
                            rusty_agent::ToolStatus::Done { output } => {
                                rusty_tui::app::AgentEvent::ToolDone {
                                    name: name.to_string(),
                                    is_error: false,
                                    output,
                                }
                            }
                            rusty_agent::ToolStatus::Error { output } => {
                                rusty_tui::app::AgentEvent::ToolDone {
                                    name: name.to_string(),
                                    is_error: true,
                                    output,
                                }
                            }
                        };
                        let _ = tx_tool.send(AgentTaskEvent::Event(event));
                    });
                    let tx_usage = event_tx.clone();
                    let usage_cb: rusty_agent::r#loop::UsageCallback = Box::new(move |input_tokens, output_tokens, current_context_tokens, cached_input_tokens| {
                        let _ = tx_usage.send(AgentTaskEvent::Event(
                            rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens, cached_input_tokens, current_context_tokens },
                        ));
                    });
                    let tx_thinking_level = event_tx.clone();
                    let thinking_level_cb: rusty_agent::r#loop::ThinkingLevelCallback = Box::new(move |level| {
                        let _ = tx_thinking_level.send(AgentTaskEvent::Event(
                            rusty_tui::app::AgentEvent::ThinkingLevel(level),
                        ));
                    });

                    let cancel_ref = cancel.clone();
                    let result = agent
                        .run(
                            input,
                            AgentCallbacks {
                                on_text: Some(&text_cb),
                                on_thinking: Some(&thinking_cb),
                                on_tool: Some(&tool_cb),
                                on_usage: Some(&usage_cb),
                                on_thinking_level: Some(&thinking_level_cb),
                                cancel: Some(&cancel_ref),
                            },
                        )
                        .await;

                    match result {
                        Ok(_) => {
                            let _ = event_tx.send(AgentTaskEvent::Event(
                                rusty_tui::app::AgentEvent::ResponseComplete(String::new()),
                            ));
                        }
                        Err(e) => {
                            let _ = event_tx.send(AgentTaskEvent::Event(
                                rusty_tui::app::AgentEvent::Error(e.to_string()),
                            ));
                        }
                    }
                })
            }

            let mut current_run: Option<(tokio::task::JoinHandle<()>, CancelToken)> = None;
            let mut queued_chat: Option<Vec<ContentBlock>> = None;

            loop {
                tokio::select! {
                    biased;

                    // Current run finished
                    _result = async {
                        match &mut current_run {
                            Some((handle, _)) => handle.await,
                            None => std::future::pending().await,
                        }
                    }, if current_run.is_some() => {
                        if let Err(e) = _result {
                            if e.is_panic() {
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::Error(
                                        "Agent task panicked — check logs for details.".to_string()
                                    ),
                                ));
                            } else {
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::Error(
                                        format!("Agent task failed: {e}")
                                    ),
                                ));
                            }
                        }
                        current_run = None;
                        let _ = event_tx.send(AgentTaskEvent::ReadyForInput);

                        // Auto-start queued chat immediately
                        if let Some(input) = queued_chat.take() {
                            let cancel = CancelToken::new();
                            current_run = Some((start_run(agent_arc.clone(), event_tx.clone(), input, cancel.clone()), cancel));
                        }
                    }

                    // Receive commands — behavior depends on whether a run is active
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(rusty_tui::app::TuiCommand::Cancel) if current_run.is_some() => {
                                if let Some((_, cancel)) = &current_run {
                                    cancel.cancel();
                                }
                            }
                            Some(rusty_tui::app::TuiCommand::Chat(input)) if current_run.is_some() => {
                                queued_chat = Some(input);
                            }
                            Some(rusty_tui::app::TuiCommand::Compact) if current_run.is_some() => {
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::Error("Cannot compact while agent is running.".to_string()),
                                ));
                            }
                            Some(rusty_tui::app::TuiCommand::Clear) if current_run.is_some() => {
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::Error("Cannot clear while agent is running.".to_string()),
                                ));
                            }
                            Some(rusty_tui::app::TuiCommand::Chat(input)) => {
                                let cancel = CancelToken::new();
                                current_run = Some((start_run(agent_arc.clone(), event_tx.clone(), input, cancel.clone()), cancel));
                            }
                            Some(rusty_tui::app::TuiCommand::Compact) => {
                                let mut agent = agent_arc.lock().await;
                                match agent.compact().await {
                                    Ok(true) => {
                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                            rusty_tui::app::AgentEvent::ResponseComplete("Conversation compacted.".to_string()),
                                        ));
                                    }
                                    Ok(false) => {
                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                            rusty_tui::app::AgentEvent::ResponseComplete("Not enough messages to compact (need at least 4).".to_string()),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                            rusty_tui::app::AgentEvent::Error(format!("Compaction failed: {e}")),
                                        ));
                                    }
                                }
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::Clear) => {
                                let mut agent = agent_arc.lock().await;
                                agent.messages_mut().clear();
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(String::new()),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::ResumeSession(_id, messages)) => {
                                let mut agent = agent_arc.lock().await;
                                agent.messages_mut().clear();
                                for msg in messages {
                                    agent.messages_mut().push(msg);
                                }
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(String::new()),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::SessionRename(_name)) => {}
                            Some(rusty_tui::app::TuiCommand::SwitchModel(model_key)) => {
                                let mut settings = Settings::load().await.unwrap_or_default();
                                match settings.models.iter().find(|m| m.name == model_key).cloned() {
                                    Some(entry) => {
                                        let api_key = settings.resolve_api_key_for(&entry.name)
                                            .or_else(|| CredentialManager::resolve_api_key(&settings))
                                            .or_else(|| settings.api_key.clone());
                                        match api_key {
                                            Some(key) => {
                                                let provider_config = ProviderConfig {
                                                    api_key: key,
                                                    api_base: entry.api_base.clone(),
                                                    model: entry.model.clone(),
                                                    max_tokens: entry.max_tokens,
                                                    temperature: entry.temperature,
                                                    thinking_budget: entry.thinking_budget,
                                                    extra_headers: entry.extra_headers.clone(),
                                                };
                                                match rusty_provider::create_provider(entry.provider, provider_config) {
                                                    Ok(new_provider) => {
                                                        let mut agent = agent_arc.lock().await;
                                                        agent.set_provider(new_provider);
                                                        agent.config_mut().model = entry.model.clone();
                                                        agent.config_mut().api_base = Some(entry.api_base.clone());
                                                        agent.config_mut().max_tokens = entry.max_tokens;
                                                        agent.config_mut().temperature = entry.temperature;
                                                        agent.config_mut().thinking_budget = entry.thinking_budget;
                                                        let new_context = rusty_core::resolve_context_window(entry.context_window, &entry.model);
                                                        agent.config_mut().context_window = entry.context_window;
                                                        drop(agent);
                                                        let _ = settings.switch_active_model(&model_key);
                                                        let _ = settings.save().await;
                                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                                            rusty_tui::app::AgentEvent::ModelChanged(entry.model.clone(), new_context),
                                                        ));
                                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                                            rusty_tui::app::AgentEvent::ResponseComplete(
                                                                format!("Switched to model: {} ({})", entry.name, entry.model)
                                                            ),
                                                        ));
                                                    }
                                                    Err(e) => {
                                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                                            rusty_tui::app::AgentEvent::Error(format!("Failed to create provider: {e}")),
                                                        ));
                                                    }
                                                }
                                            }
                                            None => {
                                                let _ = event_tx.send(AgentTaskEvent::Event(
                                                    rusty_tui::app::AgentEvent::Error("No API key found for selected model.".to_string()),
                                                ));
                                            }
                                        }
                                    }
                                    None => {
                                        let _ = event_tx.send(AgentTaskEvent::Event(
                                            rusty_tui::app::AgentEvent::Error(format!("Model '{}' not found in registry.", model_key)),
                                        ));
                                    }
                                }
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::SetThinkingLevel(level)) => {
                                let mut agent = agent_arc.lock().await;
                                agent.config_mut().thinking_level = level;
                                drop(agent);
                                settings.thinking_level = level;
                                let _ = settings.save().await;
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ThinkingLevel(level),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(
                                        format!("Thinking level set to: {}", level.map(|l| l.to_string()).unwrap_or_else(|| "default".to_string()))
                                    ),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::SetPermissionMode(mode)) => {
                                let mut agent = agent_arc.lock().await;
                                agent.set_permission_mode(mode);
                                drop(agent);
                                settings.permission_mode = Some(mode);
                                let _ = settings.save().await;
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(
                                        format!("Permission mode set to: {mode:?}")
                                    ),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::AddModel(entry)) => {
                                let mut s = Settings::load().await.unwrap_or_default();
                                if s.models.iter().any(|m| m.name == entry.name) {
                                    let _ = event_tx.send(AgentTaskEvent::Event(
                                        rusty_tui::app::AgentEvent::Error(format!("Model '{}' already exists.", entry.name)),
                                    ));
                                } else {
                                    s.models.push(entry.clone());
                                    let _ = s.save().await;
                                    let _ = event_tx.send(AgentTaskEvent::Event(
                                        rusty_tui::app::AgentEvent::ResponseComplete(format!("Model '{}' added.", entry.name)),
                                    ));
                                }
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::UpdateModel(old_name, entry)) => {
                                let mut s = Settings::load().await.unwrap_or_default();
                                if let Some(idx) = s.models.iter().position(|m| m.name == old_name) {
                                    s.models[idx] = entry.clone();
                                    let _ = s.save().await;
                                    let _ = event_tx.send(AgentTaskEvent::Event(
                                        rusty_tui::app::AgentEvent::ResponseComplete(format!("Model '{}' updated.", entry.name)),
                                    ));
                                } else {
                                    let _ = event_tx.send(AgentTaskEvent::Event(
                                        rusty_tui::app::AgentEvent::Error(format!("Model '{}' not found.", old_name)),
                                    ));
                                }
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::DeleteModel(name)) => {
                                let mut s = Settings::load().await.unwrap_or_default();
                                s.models.retain(|m| m.name != name);
                                let _ = s.save().await;
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(format!("Model '{}' deleted.", name)),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::SetModelApiKey(name, key)) => {
                                let mut s = Settings::load().await.unwrap_or_default();
                                s.set_model_api_key(&name, &key);
                                let _ = s.save().await;
                                let _ = event_tx.send(AgentTaskEvent::Event(
                                    rusty_tui::app::AgentEvent::ResponseComplete(format!("API key set for '{}'.", name)),
                                ));
                                let _ = event_tx.send(AgentTaskEvent::ReadyForInput);
                            }
                            Some(rusty_tui::app::TuiCommand::Cancel) => {}
                            None => {
                                if let Some((handle, _)) = current_run.take() {
                                    handle.abort();
                                }
                                break;
                            }
                        }
                    }
                }
            }

            // Return messages for session saving
            let agent = agent_arc.lock().await;
            let _ = msg_return_tx.send(agent.messages().to_vec());
        }
    });

    let mut tui_app = rusty_tui::app::AppState::default();
    tui_app.status.model = model.to_string();
    tui_app.status.context_window = config.effective_context_window();

    // Spawn background update check (non-blocking)
    {
        let update_tx = update_event_tx;
        tokio::spawn(async move {
            match rusty_core::update::check_for_update().await {
                Ok(Some(result)) => {
                    let _ = update_tx.send(AgentTaskEvent::Event(rusty_tui::app::AgentEvent::UpdateAvailable(result)));
                }
                Ok(None) => {}
                Err(_) => {} // Silently ignore check failures
            }
        });
    }

    // Spawn dedicated crossterm event reading thread to avoid blocking the tokio runtime.
    // crossterm's event::poll/read are synchronous and can block on macOS, starving
    // the async runtime of CPU time and causing input drops under load.
    let (crossterm_tx, crossterm_rx) = mpsc::unbounded_channel::<Event>();
    std::thread::Builder::new()
        .name("crossterm-events".into())
        .spawn(move || {
            loop {
                match event::poll(Duration::from_millis(200)) {
                    Ok(true) => match event::read() {
                        Ok(evt) => {
                            if crossterm_tx.send(evt).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    },
                    Ok(false) => continue,
                    Err(_) => break,
                }
            }
        })
        .expect("failed to spawn crossterm event thread");

    let tui_result = tui_main_loop(
        &mut terminal,
        &mut tui_app,
        &cmd_tx,
        &mut event_rx,
        crossterm_rx,
        agent_handle,
        working_dir,
        tui_active.clone(),
        sessions_dir,
        keymap,
    )
    .await;

    // Drop cmd_tx so the agent task sees channel close and finishes
    drop(cmd_tx);

    // Restore real stderr before tearing down the TUI
    #[cfg(unix)]
    if saved_stderr_fd >= 0 {
        unsafe { libc::dup2(saved_stderr_fd, libc::STDERR_FILENO) };
        unsafe { libc::close(saved_stderr_fd) };
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    // Wait for agent task to return messages for session save
    let messages = msg_return_rx.await.unwrap_or_default();

    let session = rusty_core::ConversationSession {
        id: uuid::Uuid::new_v4().to_string(),
        name: tui_app.session_name.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        messages,
        model: config.model.clone(),
        working_dir: working_dir.display().to_string(),
    };
    session.save(sessions_dir).await?;
    info!("Session saved: {}", session.id);

    tui_result
}

/// Convert a crossterm KeyEvent to a rusty_keymap KeyEvent.
fn crossterm_to_keymap_key(key: crossterm::event::KeyEvent) -> keymap_lib::KeyEvent {
    use crossterm::event::{KeyCode, KeyModifiers};

    let key_str = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::F(n) => format!("f{n}"),
        _ => return keymap_lib::KeyEvent::plain("__unmapped__"),
    };

    let mut mods = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        mods.push(keymap_lib::Modifier::Ctrl);
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        mods.push(keymap_lib::Modifier::Alt);
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        mods.push(keymap_lib::Modifier::Shift);
    }

    keymap_lib::KeyEvent::new(key_str, mods)
}

/// Dispatch a keymap action name to the appropriate AppState mutation or command.
/// Returns Ok(true) if the action was handled, Ok(false) if not recognised.
fn dispatch_keymap_action(
    action: &str,
    app: &mut rusty_tui::app::AppState,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
) -> bool {
    match action {
        "quit" | "exit" => {
            app.slash_command = Some("quit".into());
            true
        }
        "copy" => {
            app.slash_command = Some("copy".into());
            true
        }
        "compact" => {
            app.slash_command = Some("compact".into());
            true
        }
        "paste" => {
            // Paste from system clipboard
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                if let Ok(text) = ctx.get_text() {
                    app.handle_bracketed_paste(text);
                }
            }
            true
        }
        "scroll-up" => {
            app.scroll_up(3);
            true
        }
        "scroll-down" => {
            app.scroll_down(3);
            true
        }
        "page-up" => {
            app.scroll_up(app.viewport_height.max(3));
            true
        }
        "page-down" => {
            app.scroll_down(app.viewport_height.max(3));
            true
        }
        "top" => {
            app.scroll_top();
            true
        }
        "bottom" => {
            app.scroll_bottom();
            true
        }
        "new-line" => {
            app.input.insert(app.cursor_pos, '\n');
            app.cursor_pos += 1;
            app.needs_redraw = true;
            true
        }
        _ if action.starts_with('/') => {
            if let Some(cmd) = rusty_tui::app::SlashCommand::parse(action) {
                app.execute_slash_command(cmd, cmd_tx);
            }
            true
        }
        _ => false,
    }
}

async fn tui_main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut rusty_tui::app::AppState,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    event_rx: &mut mpsc::UnboundedReceiver<AgentTaskEvent>,
    mut crossterm_rx: mpsc::UnboundedReceiver<Event>,
    agent_handle: tokio::task::JoinHandle<()>,
    working_dir: &PathBuf,
    tui_active: Arc<AtomicBool>,
    sessions_dir: &Path,
    keymap: Option<keymap_lib::KeyMap>,
) -> Result<()> {
    let mut agent_handle = Some(agent_handle);
    let mut prefix_active = false;
    let mut last_draw = std::time::Instant::now();
    const MIN_DRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

    // Signal to the panic hook that the TUI render loop is active.
    // This prevents the hook from restoring the terminal mid-render.
    tui_active.store(true, Ordering::SeqCst);
    let tui_active_for_guard = tui_active.clone();
    struct TuiActiveGuard {
        flag: Arc<AtomicBool>,
    }
    impl Drop for TuiActiveGuard {
        fn drop(&mut self) {
            self.flag.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _tui_guard = TuiActiveGuard { flag: tui_active_for_guard };

    loop {
        // Draw if needed, throttled to ~30 FPS
        if app.needs_redraw && last_draw.elapsed() >= MIN_DRAW_INTERVAL {
            terminal.draw(|frame| {
                rusty_tui::ui::draw(app, frame.area(), frame.buffer_mut());
            })?;
            app.needs_redraw = false;
            last_draw = std::time::Instant::now();
        }

        // Check if the agent task died (panic or unexpected exit)
        if let Some(ref handle) = agent_handle {
            if handle.is_finished() {
                let handle = agent_handle.take().unwrap();
                match handle.await {
                    Ok(()) => {
                        if app.is_streaming {
                            app.finish_streaming();
                        }
                    }
                    Err(e) => {
                        let msg = if e.is_panic() {
                            "Agent task panicked".to_string()
                        } else {
                            format!("Agent task aborted: {e}")
                        };
                        app.push_error(&msg);
                    }
                }
            }
        }

        if app.should_quit {
            return Ok(());
        }

        // Calculate wait until next draw deadline
        let wait_duration = if app.needs_redraw {
            MIN_DRAW_INTERVAL
                .saturating_sub(last_draw.elapsed())
                .max(Duration::from_millis(1))
        } else {
            Duration::from_millis(100)
        };

        tokio::select! {
            biased;

            // Terminal event from dedicated reading thread
            evt = crossterm_rx.recv() => {
                match evt {
                    Some(Event::Key(key)) => {
                        // Handle session picker Enter specially
                        if key.code == KeyCode::Enter && app.session_picker.is_some() {
                            handle_session_picker_select(app, cmd_tx, working_dir, sessions_dir).await;
                            continue;
                        }

                        // Keymap lookup: only in normal input mode (no modal overlays)
                        if let Some(ref km) = keymap {
                            if app.permission_prompt.is_none()
                                && app.session_picker.is_none()
                                && app.file_picker.is_none()
                                && !app.is_renaming
                            {
                                let km_key = crossterm_to_keymap_key(key);

                                // Check prefix key first
                                if km.has_prefix() && km.is_prefix(&km_key) {
                                    prefix_active = true;
                                    continue;
                                }

                                let lookup = km.lookup(&km_key, prefix_active);
                                // Reset prefix mode after any non-prefix keypress
                                if prefix_active {
                                    prefix_active = false;
                                }

                                if let Some((action, _consumed_prefix)) = lookup {
                                    if dispatch_keymap_action(&action, app, cmd_tx) {
                                        app.needs_redraw = true;
                                        continue;
                                    }
                                }
                                // No binding matched: fall through to default handling
                            }
                        }

                        // Handle Enter — send message or queue while streaming
                        if key.code == KeyCode::Enter
                            && !app.input.is_empty()
                            && app.permission_prompt.is_none()
                            && app.session_picker.is_none()
                            && app.file_picker.is_none()
                            && !app.paste_mode
                        {
                            let input = app.input.clone();

                            if app.is_streaming {
                                if input.starts_with('/') {
                                    app.push_system("Commands cannot be used while the agent is responding.");
                                    app.input.clear();
                                    app.cursor_pos = 0;
                                    app.needs_redraw = true;
                                } else {
                                    app.queue_current_input();
                                }
                            } else if let Some(slash) = rusty_tui::app::SlashCommand::parse(&input) {
                                app.input.clear();
                                app.cursor_pos = 0;
                                app.history.push(input.clone());
                                app.history_idx = None;
                                handle_slash_command(app, slash, cmd_tx, working_dir, sessions_dir).await;
                                app.needs_redraw = true;
                            } else if input.starts_with('/') {
                                app.push_system(&format!(
                                    "Unknown command: {input}. Type /help for available commands."
                                ));
                                app.input.clear();
                                app.cursor_pos = 0;
                                app.needs_redraw = true;
                            } else {
                                // Regular chat message
                                app.messages.push(rusty_tui::app::ChatMessage {
                                    role: rusty_tui::app::MessageRole::User,
                                    content: input.clone(),
                                });
                                app.history.push(input.clone());
                                app.history_idx = None;
                                let blocks = app.build_content_blocks();
                                app.input.clear();
                                app.cursor_pos = 0;
                                app.clear_pasted_content();
                                app.is_streaming = true;
                                app.streaming_text.clear();
                                app.streaming_text = "...".to_string();
                                app.scroll_offset = 0;
                                app.needs_redraw = true;
                                let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(blocks));
                            }
                        } else {
                            // Handle key (including permission prompt responses)
                            let had_perm = app.permission_prompt.is_some();
                            app.handle_key(key);
                            if app.take_cancel_requested() {
                                let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Cancel);
                            }
                            if let Some(model_key) = app.model_switch_requested.take() {
                                let _ = cmd_tx.send(rusty_tui::app::TuiCommand::SwitchModel(model_key));
                            }
                            if let Some(level) = app.thinking_level_change_requested.take() {
                                let _ = cmd_tx.send(rusty_tui::app::TuiCommand::SetThinkingLevel(level));
                            }
                            if let Some(mode) = app.permission_mode_change_requested.take() {
                                let _ = cmd_tx.send(rusty_tui::app::TuiCommand::SetPermissionMode(mode));
                            }
                            if had_perm {
                                app.needs_redraw = true;
                            }
                        }
                    }
                    Some(Event::Resize(_, _)) => {
                        app.needs_redraw = true;
                    }
                    Some(Event::Paste(text))
                        if app.permission_prompt.is_none()
                            && app.session_picker.is_none()
                            && app.file_picker.is_none()
                            && !app.is_renaming =>
                    {
                        app.handle_bracketed_paste(text);
                    }
                    Some(_) => {}
                    None => break, // crossterm thread exited
                }
            }
            // Agent event
            evt = event_rx.recv() => {
                match evt {
                    Some(AgentTaskEvent::Event(event)) => match event {
                        rusty_tui::app::AgentEvent::TextDelta(text) => {
                            app.push_streaming_text(&text);
                        }
                        rusty_tui::app::AgentEvent::ThinkingDelta(text) => {
                            app.push_thinking_text(&text);
                        }
                        rusty_tui::app::AgentEvent::ResponseComplete(msg) => {
                            app.finish_streaming();
                            if !msg.is_empty() && app.messages.last().map_or(true, |m| m.role != rusty_tui::app::MessageRole::Assistant) {
                                app.push_system(&msg);
                            }
                        }
                        rusty_tui::app::AgentEvent::Error(msg) => {
                            app.push_error(&msg);
                        }
                        rusty_tui::app::AgentEvent::ToolStart { name, arguments } => {
                            app.tool_started(&name, &arguments);
                        }
                        rusty_tui::app::AgentEvent::ToolDone { name, is_error, output } => {
                            app.tool_finished(&name, is_error, &output);
                        }
                        rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens, cached_input_tokens, current_context_tokens } => {
                            app.status.input_tokens = input_tokens;
                            app.status.output_tokens = output_tokens;
                            app.status.cached_input_tokens = cached_input_tokens;
                            app.status.current_context_tokens = current_context_tokens;
                            app.needs_redraw = true;
                        }
                        rusty_tui::app::AgentEvent::ThinkingLevel(level) => {
                            app.status.thinking_level = level;
                            app.needs_redraw = true;
                        }
                        rusty_tui::app::AgentEvent::ModelChanged(model, context_window) => {
                            app.status.model = model;
                            app.status.context_window = context_window;
                            app.needs_redraw = true;
                        }
                        rusty_tui::app::AgentEvent::UpdateAvailable(result) => {
                            app.update_available = Some(result.latest_version);
                            app.needs_redraw = true;
                        }
                    },
                    Some(AgentTaskEvent::PermissionRequest(msg)) => {
                        app.permission_prompt = Some(rusty_tui::app::PermissionPromptState {
                            request: msg.request,
                            respond: Some(msg.respond),
                        });
                        app.needs_redraw = true;
                    }
                    Some(AgentTaskEvent::ReadyForInput) => {
                        if let Some(input) = app.take_queued_message() {
                            let blocks = app.take_queued_blocks().unwrap_or_else(|| {
                                vec![ContentBlock::Text { text: input.clone() }]
                            });
                            app.messages.push(rusty_tui::app::ChatMessage {
                                role: rusty_tui::app::MessageRole::User,
                                content: input.clone(),
                            });
                            app.history.push(input.clone());
                            app.history_idx = None;
                            app.is_streaming = true;
                            app.streaming_text.clear();
                            app.streaming_text = "...".to_string();
                            app.scroll_offset = 0;
                            app.needs_redraw = true;
                            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(blocks));
                        }
                    }
                    None => break, // agent task channel closed
                }
            }
            // Wake up to draw if needed
            _ = tokio::time::sleep(wait_duration) => {}
        }
    }

    Ok(())
}

/// Handle a slash command from the TUI
async fn handle_slash_command(
    app: &mut rusty_tui::app::AppState,
    cmd: rusty_tui::app::SlashCommand,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    _working_dir: &PathBuf,
    sessions_dir: &Path,
) {
    match cmd {
        rusty_tui::app::SlashCommand::Help => {
            let mut help = String::from("Available commands:\n");
            for (cmd, desc) in rusty_tui::app::SlashCommand::all_descriptions() {
                help.push_str(&format!("  {:12} {}\n", cmd, desc));
            }
            help.push_str("\nTab completes partial slash commands.");
            app.push_system(&help);
        }
        rusty_tui::app::SlashCommand::Init => {
            app.messages.push(rusty_tui::app::ChatMessage {
                role: rusty_tui::app::MessageRole::User,
                content: "/init (generate AGENTS.md)".to_string(),
            });
            app.is_streaming = true;
            app.streaming_text.clear();
            app.streaming_text = "...".to_string();
            app.scroll_offset = 0;
            app.needs_redraw = true;
            let init_prompt = build_init_prompt();
            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(vec![ContentBlock::Text { text: init_prompt }]));
        }
        rusty_tui::app::SlashCommand::Resume => {
            // Load sessions and show the picker
            match ConversationSession::list(sessions_dir).await {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        app.push_system("No saved sessions to resume.");
                    } else {
                        app.session_picker =
                            Some(rusty_tui::app::SessionPickerState::from_sessions(sessions));
                    }
                }
                Err(e) => {
                    app.push_system(&format!("Failed to load sessions: {e}"));
                }
            }
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Sessions => {
            match ConversationSession::list(sessions_dir).await {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        app.push_system("No saved sessions.");
                    } else {
                        let mut msg = String::from("Saved sessions:\n");
                        for s in &sessions {
                            let preview = s
                                .messages
                                .last()
                                .map(|m| {
                                    let text = m.get_all_text();
                                    let first_line = text.lines().next().unwrap_or(&text);
                                    if first_line.len() > 50 {
                                        let safe = first_line.floor_char_boundary(50);
                                        format!("{}...", &first_line[..safe])
                                    } else {
                                        first_line.to_string()
                                    }
                                })
                                .unwrap_or_default();
                            msg.push_str(&format!(
                                "  {} | {} msgs | {} | {}\n",
                                &s.id[..8],
                                s.messages.len(),
                                s.model,
                                s.updated_at.format("%Y-%m-%d %H:%M")
                            ));
                            if !preview.is_empty() {
                                msg.push_str(&format!("    {}\n", preview));
                            }
                        }
                        msg.push_str("\nUse /resume to open the session picker.");
                        app.push_system(&msg);
                    }
                }
                Err(e) => {
                    app.push_system(&format!("Failed to load sessions: {e}"));
                }
            }
        }
        rusty_tui::app::SlashCommand::Compact => {
            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Compact);
            app.push_system("Compacting conversation...");
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Clear => {
            app.messages.clear();
            app.streaming_text.clear();
            app.thinking_text.clear();
            app.saved_thinking.clear();
            app.thinking_line_count = 0;
            app.thinking_expanded = false;
            app.pending_tools.clear();
            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Clear);
            app.push_system("Conversation cleared.");
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Copy => {
            // Find the last assistant message
            let last_response = app.messages.iter().rev().find(|m| m.role == rusty_tui::app::MessageRole::Assistant);
            match last_response {
                Some(msg) => {
                    match arboard::Clipboard::new() {
                        Ok(mut clipboard) => {
                            match clipboard.set_text(&msg.content) {
                                Ok(_) => app.push_system("Copied last response to clipboard."),
                                Err(e) => app.push_system(&format!("Failed to copy: {e}")),
                            }
                        }
                        Err(e) => app.push_system(&format!("Clipboard unavailable: {e}")),
                    }
                }
                None => app.push_system("No assistant response to copy."),
            }
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Model => {
            app.push_system(&format!("Current model: {}", app.status.model));
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Rename => {
            let input = app.input.clone();
            let new_name = input.strip_prefix("/rename").unwrap_or("").trim();
            if new_name.is_empty() {
                app.push_system("Usage: /rename <new session name>");
            } else {
                app.session_name = Some(new_name.to_string());
                app.push_system(&format!("Session renamed to: {new_name}"));
            }
            app.input.clear();
            app.cursor_pos = 0;
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Permissions => {
            let settings = rusty_core::Settings::load().await.unwrap_or_default();
            let input = app.input.clone();
            let args = input
                .strip_prefix("/permissions")
                .or_else(|| input.strip_prefix("/perms"))
                .unwrap_or("")
                .trim();

            if args.starts_with("remove ") {
                let tool_key = args.strip_prefix("remove ").unwrap().trim();
                if tool_key.is_empty() {
                    app.push_system("Usage: /permissions remove <tool_key>");
                } else {
                    match rusty_core::remove_permanent_permission(tool_key).await {
                        Ok(true) => {
                            app.push_system(&format!("Removed '{tool_key}' from always-approve list."))
                        }
                        Ok(false) => {
                            app.push_system(&format!(
                                "'{tool_key}' was not in the always-approve list."
                            ))
                        }
                        Err(e) => app.push_system(&format!("Error: {e}")),
                    }
                }
            } else if settings.allowed_tools.is_empty() {
                app.push_system("No tools in always-approve list.");
            } else {
                let mut msg = String::from("Always-approved tools:\n");
                for tool in &settings.allowed_tools {
                    msg.push_str(&format!("  • {tool}\n"));
                }
                msg.push_str("\nUse /permissions remove <tool_key> to revoke.");
                app.push_system(&msg);
            }

            app.input.clear();
            app.cursor_pos = 0;
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Quit => {
            app.should_quit = true;
        }
        rusty_tui::app::SlashCommand::Settings => {
            // Toggle the settings panel visibility
            if app.settings_overlay.is_some() {
                app.settings_overlay = None;
            } else {
                // Build the model list from the user's saved registry
                let settings = rusty_core::Settings::load().await.unwrap_or_default();
                let models = if settings.models.is_empty() {
                    rusty_tui::model_registry::default_model_list()
                } else {
                    settings.models.clone()
                };
                let active_name = settings.active_model.clone();
                app.settings_overlay = Some(rusty_tui::app::SettingsState::new(
                    models,
                    active_name,
                    settings.thinking_level,
                    settings.permission_mode.unwrap_or(rusty_core::PermissionMode::Default),
                ));
            }
            app.input.clear();
            app.cursor_pos = 0;
            app.needs_redraw = true;
        }
        rusty_tui::app::SlashCommand::Version => {
            let current = rusty_core::update::current_version();
            let mut msg = format!("rusty v{current}");
            match app.update_available.as_ref() {
                Some(update) => {
                    msg.push_str(&format!("\nUpdate available: v{}", update));
                    msg.push_str("\nhttps://github.com/PDG-Global/rusty/releases");
                }
                None => {
                    msg.push_str("\nYou are running the latest version.");
                }
            }
            app.push_system(&msg);
            app.needs_redraw = true;
        }
    }
}

/// Handle session picker selection
async fn handle_session_picker_select(
    app: &mut rusty_tui::app::AppState,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    _working_dir: &PathBuf,
    sessions_dir: &Path,
) {
    let picker = match app.session_picker.take() {
        Some(p) => p,
        None => return,
    };

    if picker.sessions.is_empty() {
        return;
    }

    let selected_idx = picker.selected;
    let session_id = match picker.sessions.get(selected_idx) {
        Some(entry) => entry.id.clone(),
        None => return,
    };

    // Load the full session from disk
    match ConversationSession::load(sessions_dir, &session_id).await {
        Ok(Some(session)) => {
            let msg_count = session.messages.len();
            app.messages.clear();
            app.streaming_text.clear();
            app.thinking_text.clear();
            app.saved_thinking.clear();
            app.thinking_line_count = 0;
            app.thinking_expanded = false;
            app.pending_tools.clear();

            // Show loaded messages in the TUI
            for msg in &session.messages {
                let role = match msg.role {
                    rusty_core::Role::User => rusty_tui::app::MessageRole::User,
                    rusty_core::Role::Assistant => rusty_tui::app::MessageRole::Assistant,
                };
                app.messages.push(rusty_tui::app::ChatMessage {
                    role,
                    content: msg.get_all_text(),
                });
            }

            app.push_system(&format!(
                "Resumed session {} ({} messages).",
                &session_id[..8],
                msg_count
            ));

            // Send the messages to the agent task
            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::ResumeSession(
                session_id,
                session.messages,
            ));
        }
        Ok(None) => {
            app.push_system(&format!("Session {} not found.", &session_id[..8]));
        }
        Err(e) => {
            app.push_system(&format!("Failed to load session: {e}"));
        }
    }

    app.needs_redraw = true;
}

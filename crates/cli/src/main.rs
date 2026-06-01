// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use rusty_agent::{Agent, AgentCallbacks};
use rusty_core::permissions::{PermissionDecision, PermissionRequest};
use rusty_core::{Config, ConversationSession, CredentialManager, PermissionMode, Settings};
use rusty_core::setup_wizard::{run_setup_wizard, is_first_run};
use rusty_provider::{OpenAiProvider, ProviderConfig};
use rusty_tools::{all_tools, Tool};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();

    // Handle --setup or auto-detect first run
    let needs_setup = args.setup || is_first_run();
    if needs_setup {
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

    // Handle --list-sessions early exit
    if args.list_sessions {
        let sessions = rusty_core::ConversationSession::list().await?;
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
                            format!("{}...", &text[..60])
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

    let working_dir = args
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load config
    let settings = Settings::load().await.unwrap_or_default();
    let mut config = Config::default();

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
                config.api_base = Some("https://api.moonshot.cn/v1".to_string());
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
                config.api_base = Some("https://api.deepseek.com/v1".to_string());
                if args.model.is_none() && settings.default_model.is_none() {
                    config.model = "deepseek-chat".to_string();
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

    // Capture allowlist before settings are consumed
    let permanent_allowlist = settings.allowed_tools_set();

    // Apply settings (explicit flags override preset)
    if let Some(key) = settings.api_key.clone().or(args.api_key) {
        config.api_key = Some(key);
    }
    if let Some(base) = settings.api_base.clone().or(args.api_base) {
        config.api_base = Some(base);
    }
    if let Some(model) = args.model.or(settings.default_model.clone()) {
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

    // Build provider — resolve API key using CredentialManager (handles
    // env vars → keyring → settings file, matching wizard's storage order)
    let api_key = CredentialManager::resolve_api_key(&settings);
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

    let provider_config = ProviderConfig {
        api_key,
        api_base,
        model: config.model.clone(),
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        thinking_budget: config.thinking_budget,
    };

    let provider = OpenAiProvider::new(provider_config)?;
    let provider: Arc<dyn rusty_provider::LlmProvider> = Arc::new(provider);

    // Build system prompt (must happen before make_agent_tool so sub-agents
    // receive the full context including AGENTS.md/CLAUDE.md, platform info, etc.)
    let system_prompt = rusty_agent::build_system_prompt(&config, &working_dir).await;

    // Build tools (including agent tool)
    let mut tools: Vec<Box<dyn Tool>> = all_tools();

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
        if let Some(session) = rusty_core::ConversationSession::load(session_id).await? {
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

    // Run mode
    if let Some(prompt) = args.prompt {
        // Non-interactive mode
        run_headless(&mut agent, &prompt).await?;
    } else if args.headless {
        // Headless mode with stdin
        run_headless_stdin(&mut agent, &config.model).await?;
    } else {
        // Interactive TUI mode — moves agent into spawned task, handles session save internally
        run_tui(agent, &config.model, permanent_allowlist, &config, &working_dir).await?;
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
    session.save().await?;
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
            prompt,
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

async fn run_headless_stdin(agent: &mut Agent, model: &str) -> Result<()> {
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
                    let sessions = ConversationSession::list().await?;
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
                            &init_prompt,
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
                Some(rusty_tui::app::SlashCommand::Resume) => {
                    let sessions = ConversationSession::list().await?;
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
                line,
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
    r#"Analyze this codebase thoroughly and write an AGENTS.md file at the project root.

The file should serve as a comprehensive guide for any AI agent (or developer) working in this repository. Write it in plain, direct language. No emojis. No filler. Structure it for both human readability and machine parseability.

Cover all of the following sections:

## Overview
What this project is, what language/framework it uses, what problem it solves, and the high-level architecture. Include the runtime/async model if relevant.

## Workspace Structure
A tree view of the directory layout with one-line descriptions of each directory and key file. Show the dependency graph between modules/packages.

## Key Modules and Files
For each major module or crate, list the files with their purpose. Include the main types, traits, and functions exported. Note the data flow between modules.

## Configuration
How the project is configured: config files, environment variables, CLI flags, presets. Include the search/resolution order for overlapping config sources.

## Building and Running
Exact commands to build, run, test, lint, and format. Include both debug and release builds. Show example invocations with common flags.

## Architecture Patterns
The key patterns used in the codebase: streaming, callbacks, error handling strategy, permission model, plugin/extension points. Be specific — name the types and traits involved.

## Data Flow
Trace the main request/response path from user input through to output. Include tool execution, permission checks, and any async/streaming pipeline.

## Testing
Where tests live, how to run them, what's covered. Note any test utilities or fixtures.

## Adding New Functionality
Step-by-step guides for common extension tasks: adding a new tool, adding a new provider, adding a new command. List the files to touch and the interfaces to implement.

## Error Handling
The error types used, how errors propagate, retry logic, and how to add new error variants.

## Dependencies
Key external crates/libraries and what they're used for. Note any version constraints or compatibility concerns.

## Troubleshooting
Common issues and their fixes. Include debug logging setup.

Write the file as AGENTS.md in the project root. Use markdown headers, tables where they help, and code blocks for commands and file paths. Keep it under 500 lines — dense and useful, not verbose."#.to_string()
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

async fn run_tui(
    agent: Agent,
    model: &str,
    permanent_allowlist: HashSet<String>,
    config: &Config,
    working_dir: &PathBuf,
) -> Result<()> {
    use rusty_core::Message;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
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
                input: String,
                cancel: rusty_agent::CancelToken,
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
                    let usage_cb: rusty_agent::r#loop::UsageCallback = Box::new(move |input_tokens, output_tokens| {
                        let _ = tx_usage.send(AgentTaskEvent::Event(
                            rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens },
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
                            &input,
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

            let mut current_run: Option<(tokio::task::JoinHandle<()>, rusty_agent::CancelToken)> = None;
            let mut queued_chat: Option<String> = None;

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
                        current_run = None;
                        let _ = event_tx.send(AgentTaskEvent::ReadyForInput);

                        // Auto-start queued chat immediately
                        if let Some(input) = queued_chat.take() {
                            let cancel = rusty_agent::CancelToken::new();
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
                                let cancel = rusty_agent::CancelToken::new();
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

    let tui_result = tui_main_loop(
        &mut terminal,
        &mut tui_app,
        &cmd_tx,
        &mut event_rx,
        agent_handle,
        working_dir,
    )
    .await;

    // Drop cmd_tx so the agent task sees channel close and finishes
    drop(cmd_tx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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
    session.save().await?;
    info!("Session saved: {}", session.id);

    tui_result
}

async fn tui_main_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut rusty_tui::app::AppState,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    event_rx: &mut mpsc::UnboundedReceiver<AgentTaskEvent>,
    agent_handle: tokio::task::JoinHandle<()>,
    working_dir: &PathBuf,
) -> Result<()> {
    let mut agent_handle = Some(agent_handle);
    let mut last_draw = std::time::Instant::now();
    const MIN_DRAW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

    loop {
        // Draw if needed, throttled to ~30 FPS
        if app.needs_redraw && last_draw.elapsed() >= MIN_DRAW_INTERVAL {
            terminal.draw(|frame| {
                rusty_tui::ui::draw(app, frame.area(), frame.buffer_mut());
            })?;
            app.needs_redraw = false;
            last_draw = std::time::Instant::now();
        }

        // Poll terminal events (always, even during agent processing)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    // Handle session picker Enter specially
                    if key.code == KeyCode::Enter && app.session_picker.is_some() {
                        handle_session_picker_select(app, cmd_tx, working_dir).await;
                        continue;
                    }

                    // Handle Enter — send message or queue while streaming
                    if key.code == KeyCode::Enter
                        && !app.input.is_empty()
                        && app.permission_prompt.is_none()
                        && app.session_picker.is_none()
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
                            handle_slash_command(app, slash, cmd_tx, working_dir).await;
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
                            app.input.clear();
                            app.cursor_pos = 0;
                            app.is_streaming = true;
                            app.streaming_text.clear();
                            app.streaming_text = "...".to_string();
                            app.needs_redraw = true;
                            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(input));
                        }
                    } else {
                        // Handle key (including permission prompt responses)
                        let had_perm = app.permission_prompt.is_some();
                        app.handle_key(key);
                        if app.take_cancel_requested() {
                            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Cancel);
                        }
                        if had_perm {
                            app.needs_redraw = true;
                        }
                    }
                }
                Event::Resize(_, _) => {
                    app.needs_redraw = true;
                }
                Event::Paste(text)
                    if app.permission_prompt.is_none()
                        && app.session_picker.is_none()
                        && !app.is_renaming =>
                {
                    // Insert pasted text at cursor position, replacing newlines with spaces
                    let sanitized: String = text
                        .chars()
                        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                        .collect();
                    let byte_pos = app.cursor_pos;
                    if byte_pos <= app.input.len() {
                        app.input.insert_str(byte_pos, &sanitized);
                        app.cursor_pos += sanitized.len();
                        app.needs_redraw = true;
                    }
                }
                _ => {}
            }
        }

        // Drain agent events (non-blocking)
        loop {
            match event_rx.try_recv() {
                Ok(AgentTaskEvent::Event(event)) => match event {
                    rusty_tui::app::AgentEvent::TextDelta(text) => {
                        app.push_streaming_text(&text);
                    }
                    rusty_tui::app::AgentEvent::ThinkingDelta(text) => {
                        app.push_thinking_text(&text);
                    }
                    rusty_tui::app::AgentEvent::ResponseComplete(_) => {
                        app.finish_streaming();
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
                    rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens } => {
                        app.status.input_tokens = input_tokens;
                        app.status.output_tokens = output_tokens;
                        app.needs_redraw = true;
                    }
                    rusty_tui::app::AgentEvent::ThinkingLevel(level) => {
                        app.status.thinking_level = level;
                        app.needs_redraw = true;
                    }
                },
                Ok(AgentTaskEvent::PermissionRequest(msg)) => {
                    // Show the permission prompt in the TUI
                    app.permission_prompt = Some(rusty_tui::app::PermissionPromptState {
                        request: msg.request,
                        respond: Some(msg.respond),
                    });
                    app.needs_redraw = true;
                }
                Ok(AgentTaskEvent::ReadyForInput) => {
                    // Agent is ready — auto-send any queued message
                    if let Some(input) = app.take_queued_message() {
                        app.messages.push(rusty_tui::app::ChatMessage {
                            role: rusty_tui::app::MessageRole::User,
                            content: input.clone(),
                        });
                        app.history.push(input.clone());
                        app.history_idx = None;
                        app.is_streaming = true;
                        app.streaming_text.clear();
                        app.streaming_text = "...".to_string();
                        app.needs_redraw = true;
                        let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(input));
                    }
                }
                Err(_) => break,
            }
        }

        // Check if the agent task died (panic or unexpected exit)
        if let Some(ref handle) = agent_handle {
            if handle.is_finished() {
                let handle = agent_handle.take().unwrap();
                match handle.await {
                    Ok(()) => {
                        // Task completed normally — cmd_tx was dropped
                        // Ensure streaming state is cleared
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

        // Small yield to prevent busy-waiting
        if app.needs_redraw {
            let elapsed = last_draw.elapsed();
            if elapsed < MIN_DRAW_INTERVAL {
                tokio::time::sleep(MIN_DRAW_INTERVAL - elapsed).await;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    }
}

/// Handle a slash command from the TUI
async fn handle_slash_command(
    app: &mut rusty_tui::app::AppState,
    cmd: rusty_tui::app::SlashCommand,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    _working_dir: &PathBuf,
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
            app.needs_redraw = true;
            let init_prompt = build_init_prompt();
            let _ = cmd_tx.send(rusty_tui::app::TuiCommand::Chat(init_prompt));
        }
        rusty_tui::app::SlashCommand::Resume => {
            // Load sessions and show the picker
            match ConversationSession::list().await {
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
            match ConversationSession::list().await {
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
                                        format!("{}...", &first_line[..50])
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
        rusty_tui::app::SlashCommand::Quit => {
            app.should_quit = true;
        }
    }
}

/// Handle session picker selection
async fn handle_session_picker_select(
    app: &mut rusty_tui::app::AppState,
    cmd_tx: &mpsc::UnboundedSender<rusty_tui::app::TuiCommand>,
    _working_dir: &PathBuf,
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
    match ConversationSession::load(&session_id).await {
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

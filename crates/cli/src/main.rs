use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use rusty_agent::Agent;
use rusty_core::permissions::{PermissionDecision, PermissionRequest};
use rusty_core::{Config, PermissionMode, Settings};
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

    /// No TUI, just print responses
    #[arg(long)]
    headless: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum PermissionModeArg {
    Default,
    AcceptEdits,
    Bypass,
    Plan,
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
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();

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
    if let Some(key) = settings.api_key.or(args.api_key) {
        config.api_key = Some(key);
    }
    if let Some(base) = settings.api_base.or(args.api_base) {
        config.api_base = Some(base);
    }
    if let Some(model) = args.model.or(settings.default_model) {
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
    config.verbose = args.verbose;
    config.permission_mode = args.permissions.into();

    // Build provider
    let api_key = config.resolve_api_key();
    let api_base = config.resolve_api_base();

    let api_key = match api_key {
        Some(key) => key,
        None => {
            eprintln!("Error: No API key found.");
            eprintln!();
            eprintln!("Set one of:");
            eprintln!("  --api-key <KEY>");
            eprintln!("  OPENAI_API_KEY=<KEY>");
            eprintln!("  RUSTY_API_KEY=<KEY>");
            eprintln!("  ~/.rusty/settings.json {{ \"api_key\": \"<KEY>\" }}");
            std::process::exit(1);
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

    // Build tools (including agent tool)
    let mut tools: Vec<Box<dyn Tool>> = all_tools();

    // Add agent tool with spawn function
    let agent_tool = rusty_agent::make_agent_tool(
        provider.clone(),
        config
            .system_prompt
            .clone()
            .unwrap_or_else(|| "You are a helpful AI coding assistant.".to_string()),
        config.clone(),
    );
    tools.push(Box::new(agent_tool));

    // Build system prompt
    let system_prompt = rusty_agent::build_system_prompt(&config, &working_dir).await;

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
        run_headless_stdin(&mut agent).await?;
    } else {
        // Interactive TUI mode — moves agent into spawned task, handles session save internally
        run_tui(agent, &config.model, permanent_allowlist, &config, &working_dir).await?;
        return Ok(());
    }

    // Save session (headless modes only — TUI saves internally)
    let session = rusty_core::ConversationSession {
        id: uuid::Uuid::new_v4().to_string(),
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

    let result = agent.run(prompt, Some(&text_cb), None, None, None).await?;
    if !result.ends_with('\n') {
        println!();
    }
    Ok(())
}

async fn run_headless_stdin(agent: &mut Agent) -> Result<()> {
    use std::io::{self, BufRead};
    let stdin = io::stdin();

    println!("rusty (headless mode). Type 'exit' or Ctrl-D to quit.");

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

        let text_cb: rusty_agent::r#loop::TextCallback = Box::new(|text| {
            print!("{text}");
            let _ = io::stdout().flush();
        });

        let result = agent.run(line, Some(&text_cb), None, None, None).await?;
        if !result.ends_with('\n') {
            println!();
        }
    }
    Ok(())
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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel for user input → agent
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<String>();
    // Channel for agent → TUI events (including permission requests)
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentTaskEvent>();
    // Channel for agent task to return its message history for session save
    let (msg_return_tx, msg_return_rx) = oneshot::channel::<Vec<Message>>();

    // Spawn the agent task
    let perm_mode = config.permission_mode;
    let agent_handle = tokio::spawn(async move {
        let mut agent = agent;
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

        // Agent event callbacks
        let tx_text = event_tx.clone();
        let text_cb: rusty_agent::r#loop::TextCallback = Box::new(move |text| {
            let _ = tx_text.send(AgentTaskEvent::Event(
                rusty_tui::app::AgentEvent::TextDelta(text.to_string()),
            ));
        });
        let tx_think = event_tx.clone();
        let thinking_cb: rusty_agent::r#loop::ThinkingCallback = Box::new(move |text| {
            let _ = tx_think.send(AgentTaskEvent::Event(
                rusty_tui::app::AgentEvent::ThinkingDelta(text.to_string()),
            ));
        });
        let tx_tool = event_tx.clone();
        let tool_cb: rusty_agent::r#loop::ToolCallback = Box::new(move |name, status| {
            let event = match status {
                "running" => rusty_tui::app::AgentEvent::ToolStart(name.to_string()),
                _ => rusty_tui::app::AgentEvent::ToolDone(name.to_string(), status == "error"),
            };
            let _ = tx_tool.send(AgentTaskEvent::Event(event));
        });

        let tx_usage = event_tx.clone();
        let usage_cb: rusty_agent::r#loop::UsageCallback = Box::new(move |input_tokens, output_tokens| {
            let _ = tx_usage.send(AgentTaskEvent::Event(
                rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens },
            ));
        });

        // Process user messages one at a time
        while let Some(input) = input_rx.recv().await {
            let result = agent
                .run(&input, Some(&text_cb), Some(&thinking_cb), Some(&tool_cb), Some(&usage_cb))
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
        }

        // Return messages for session saving
        let _ = msg_return_tx.send(agent.messages().to_vec());
    });

    let mut tui_app = rusty_tui::app::AppState::default();
    tui_app.status.model = model.to_string();

    let tui_result = tui_main_loop(
        &mut terminal,
        &mut tui_app,
        &input_tx,
        &mut event_rx,
        agent_handle,
    )
    .await;

    // Drop input_tx so the agent task sees channel close and finishes
    drop(input_tx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Wait for agent task to return messages for session save
    let messages = msg_return_rx.await.unwrap_or_default();

    let session = rusty_core::ConversationSession {
        id: uuid::Uuid::new_v4().to_string(),
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
    input_tx: &mpsc::UnboundedSender<String>,
    event_rx: &mut mpsc::UnboundedReceiver<AgentTaskEvent>,
    agent_handle: tokio::task::JoinHandle<()>,
) -> Result<()> {
    let mut agent_handle = Some(agent_handle);
    loop {
        // Draw if needed
        if app.needs_redraw {
            terminal.draw(|frame| {
                rusty_tui::ui::draw(app, frame.area(), frame.buffer_mut());
            })?;
            app.needs_redraw = false;
        }

        // Poll terminal events (always, even during agent processing)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Enter
                        && !app.input.is_empty()
                        && !app.is_streaming
                        && app.permission_prompt.is_none()
                    {
                        // Send user input to agent task
                        let input = app.input.clone();
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
                        let _ = input_tx.send(input);
                    } else {
                        // Handle key (including permission prompt responses)
                        let had_perm = app.permission_prompt.is_some();
                        app.handle_key(key);
                        // If a permission prompt was just resolved, nothing else to do
                        // (the oneshot was already sent in handle_key)
                        if had_perm {
                            app.needs_redraw = true;
                        }
                    }
                }
                Event::Resize(_, _) => {
                    app.needs_redraw = true;
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
                    rusty_tui::app::AgentEvent::ToolStart(name) => {
                        app.tool_started(&name);
                    }
                    rusty_tui::app::AgentEvent::ToolDone(name, is_error) => {
                        app.tool_finished(&name, is_error);
                    }
                    rusty_tui::app::AgentEvent::Usage { input_tokens, output_tokens } => {
                        app.status.input_tokens = input_tokens;
                        app.status.output_tokens = output_tokens;
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
                Err(_) => break,
            }
        }

        // Check if the agent task died (panic or unexpected exit)
        if let Some(ref handle) = agent_handle {
            if handle.is_finished() {
                let handle = agent_handle.take().unwrap();
                match handle.await {
                    Ok(()) => {
                        // Task completed normally — input_tx was dropped
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
        if !app.needs_redraw {
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod app;
 pub mod model_registry;
pub mod ui;

use app::{AgentEvent, AppState, MessageRole};
use crossterm::{
    event::{self, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;

/// Callback invoked when the user submits input. Returns a receiver for agent events.
pub type InputHandler = Box<
    dyn Fn(String) -> mpsc::UnboundedReceiver<AgentEvent> + Send,
>;

/// Run the TUI. Stays alive until user quits (Ctrl-C / Ctrl-D).
/// `on_input` is called with each user message; it must return a receiver
/// that streams agent events back for that turn.
pub async fn run(
    model: &str,
    context_window: u32,
    on_input: InputHandler,
) -> Result<(), anyhow::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState::default();
    app.status.model = model.to_string();
    app.status.context_window = context_window;
    app.working_dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from));

    // Start with a dummy channel that's already closed
    let (_dummy_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

    let result = run_loop(&mut terminal, &mut app, &mut agent_rx, &on_input).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableBracketedPaste,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
    agent_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    on_input: &InputHandler,
) -> Result<(), anyhow::Error> {
    loop {
        // Draw if needed
        if app.needs_redraw {
            terminal.draw(|frame| {
                ui::draw(app, frame.area(), frame.buffer_mut());
            })?;
            app.needs_redraw = false;
        }

        // Check for terminal events (non-blocking)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Enter
                        && key.modifiers.is_empty()
                        && !app.input.is_empty()
                        && !app.is_streaming
                        && !app.paste_mode
                    {
                        let input = app.input.clone();
                        app.messages.push(app::ChatMessage {
                            role: MessageRole::User,
                            content: input.clone(),
                        });
                        app.history.push(input.clone());
                        app.history_idx = None;
                        app.input.clear();
                        app.cursor_pos = 0;
                        app.is_streaming = true;
                        app.streaming_text.clear();
                        app.scroll_offset = 0;
                        app.needs_redraw = true;

                        // Send input to handler, get new receiver
                        *agent_rx = on_input(input);
                    } else {
                        app.handle_key(key);
                    }
                }
                Event::Paste(text)
                    if !app.is_streaming => {
                        app.handle_bracketed_paste(text);
                    }
                Event::Mouse(mouse_event) => {
                    match mouse_event.kind {
                        MouseEventKind::ScrollUp => {
                            app.scroll_up(3);
                        }
                        MouseEventKind::ScrollDown => {
                            app.scroll_down(3);
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    app.needs_redraw = true;
                }
                _ => {}
            }
        }

        // Check for agent events (non-blocking)
        loop {
            match agent_rx.try_recv() {
                Ok(event) => match event {
                    AgentEvent::TextDelta(text) => {
                        app.push_streaming_text(&text);
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        app.push_thinking_text(&text);
                    }
                    AgentEvent::ResponseComplete(_) => {
                        app.finish_streaming();
                    }
                    AgentEvent::Error(msg) => {
                        app.push_error(&msg);
                    }
                    AgentEvent::ToolStart { name, arguments } => {
                        app.tool_started(&name, &arguments);
                    }
                    AgentEvent::ToolDone { name, is_error, output } => {
                        app.tool_finished(&name, is_error, &output);
                    }
                    AgentEvent::Usage { input_tokens, output_tokens, current_context_tokens, .. } => {
                        app.status.input_tokens = input_tokens;
                        app.status.output_tokens = output_tokens;
                        app.status.current_context_tokens = current_context_tokens;
                        app.needs_redraw = true;
                    }
                    AgentEvent::ThinkingLevel(level) => {
                        app.status.thinking_level = level;
                        app.needs_redraw = true;
                    }
                    AgentEvent::ModelChanged(model, context_window) => {
                        app.status.model = model;
                        app.status.context_window = context_window;
                        app.needs_redraw = true;
                    }
                    AgentEvent::UpdateAvailable(result) => {
                        app.update_available = Some(result.latest_version);
                        app.needs_redraw = true;
                    }
                    AgentEvent::PlanMode(mode) => {
                        app.plan_mode = mode;
                        app.needs_redraw = true;
                    }
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
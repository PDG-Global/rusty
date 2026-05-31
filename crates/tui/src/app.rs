// Copyright (C) 2025 Jeremy Moseley
// SPDX-License-Identifier: AGPL-3.0-or-later

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusty_core::{PermissionDecision, PermissionRequest};
use tokio::sync::oneshot;

/// Events from the terminal input
pub enum InputEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
}

/// Messages from the agent to display
pub enum AgentEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart(String),
    ToolDone(String, bool),
    ResponseComplete(String),
    Error(String),
    Usage { input_tokens: u32, output_tokens: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny,
}

pub struct PermissionPromptState {
    pub request: PermissionRequest,
    pub respond: Option<oneshot::Sender<PermissionDecision>>,
}

pub struct AppState {
    pub input: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub messages: Vec<ChatMessage>,
    pub status: StatusInfo,
    pub streaming_text: String,
    pub thinking_text: String,
    pub is_streaming: bool,
    pub is_thinking: bool,
    pub needs_redraw: bool,
    pub should_quit: bool,
    pub permission_prompt: Option<PermissionPromptState>,
    /// Tools currently executing — name -> start index in streaming_text
    pub pending_tools: Vec<PendingTool>,
}

pub struct PendingTool {
    pub name: String,
    /// Index in streaming_text where this tool's line starts
    pub line_start: usize,
}

pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

pub enum MessageRole {
    User,
    Assistant,
    System,
}

pub struct StatusInfo {
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub is_processing: bool,
}

impl Default for StatusInfo {
    fn default() -> Self {
        Self {
            model: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            is_processing: false,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_idx: None,
            messages: Vec::new(),
            status: StatusInfo::default(),
            streaming_text: String::new(),
            thinking_text: String::new(),
            is_streaming: false,
            is_thinking: false,
            needs_redraw: true,
            should_quit: false,
            permission_prompt: None,
            pending_tools: Vec::new(),
        }
    }
}

impl AppState {
    pub fn handle_key(&mut self, key: KeyEvent) {
        // If permission prompt is active, handle it exclusively
        if let Some(ref mut prompt) = self.permission_prompt {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(respond) = prompt.respond.take() {
                        let _ = respond.send(PermissionDecision::AllowOnce);
                    }
                    self.permission_prompt = None;
                    self.needs_redraw = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(respond) = prompt.respond.take() {
                        let _ = respond.send(PermissionDecision::Deny("User denied".into()));
                    }
                    self.permission_prompt = None;
                    self.needs_redraw = true;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    if let Some(respond) = prompt.respond.take() {
                        let _ = respond.send(PermissionDecision::AllowSession);
                    }
                    self.permission_prompt = None;
                    self.needs_redraw = true;
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    if let Some(respond) = prompt.respond.take() {
                        let _ = respond.send(PermissionDecision::AllowAlways);
                    }
                    self.permission_prompt = None;
                    self.needs_redraw = true;
                }
                KeyCode::Esc => {
                    if let Some(respond) = prompt.respond.take() {
                        let _ = respond.send(PermissionDecision::Deny("User cancelled".into()));
                    }
                    self.permission_prompt = None;
                    self.needs_redraw = true;
                }
                _ => {} // Ignore other keys during prompt
            }
            return;
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.input.is_empty() {
                    self.should_quit = true;
                }
            }
            KeyCode::Esc if self.is_streaming => {
                // Cancel streaming — reset to ready state
                self.is_streaming = false;
                self.is_thinking = false;
                if !self.streaming_text.is_empty() {
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: std::mem::take(&mut self.streaming_text),
                    });
                }
                self.thinking_text.clear();
                self.needs_redraw = true;
            }
            KeyCode::Enter if !self.input.is_empty() && !self.is_streaming => {
                let input = self.input.clone();
                self.messages.push(ChatMessage {
                    role: MessageRole::User,
                    content: input.clone(),
                });
                self.history.push(input);
                self.history_idx = None;
                self.input.clear();
                self.cursor_pos = 0;
                self.is_streaming = true;
                self.streaming_text.clear();
                self.needs_redraw = true;
                // The actual sending happens in the main loop
            }
            KeyCode::Char(c) if !self.is_streaming => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.needs_redraw = true;
            }
            KeyCode::Backspace if !self.is_streaming => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                    self.needs_redraw = true;
                }
            }
            KeyCode::Delete if !self.is_streaming => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                    self.needs_redraw = true;
                }
            }
            KeyCode::Left if !self.is_streaming => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.needs_redraw = true;
                }
            }
            KeyCode::Right if !self.is_streaming => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                    self.needs_redraw = true;
                }
            }
            KeyCode::Home if !self.is_streaming => {
                self.cursor_pos = 0;
                self.needs_redraw = true;
            }
            KeyCode::End if !self.is_streaming => {
                self.cursor_pos = self.input.len();
                self.needs_redraw = true;
            }
            KeyCode::Up if !self.is_streaming => {
                if !self.history.is_empty() {
                    let idx = match self.history_idx {
                        Some(i) => i.saturating_add(1),
                        None => 0,
                    };
                    if idx < self.history.len() {
                        self.history_idx = Some(idx);
                        let hist_idx = self.history.len() - 1 - idx;
                        self.input = self.history[hist_idx].clone();
                        self.cursor_pos = self.input.len();
                        self.needs_redraw = true;
                    }
                }
            }
            KeyCode::Down if !self.is_streaming => {
                match self.history_idx {
                    Some(0) => {
                        self.history_idx = None;
                        self.input.clear();
                        self.cursor_pos = 0;
                        self.needs_redraw = true;
                    }
                    Some(i) => {
                        let new_idx = i - 1;
                        self.history_idx = Some(new_idx);
                        let hist_idx = self.history.len() - 1 - new_idx;
                        self.input = self.history[hist_idx].clone();
                        self.cursor_pos = self.input.len();
                        self.needs_redraw = true;
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Returns true if Enter was pressed and we have input to send
    pub fn take_pending_input(&mut self) -> Option<String> {
        // Check if we just set is_streaming and have a pending message
        if self.is_streaming && self.messages.last().map_or(false, |m| matches!(m.role, MessageRole::User)) {
            None // Already handled in handle_key via the Enter branch
        } else {
            None
        }
    }

    pub fn push_streaming_text(&mut self, text: &str) {
        // If we were thinking, mark thinking as done
        if self.is_thinking {
            self.is_thinking = false;
        }
        self.streaming_text.push_str(text);
        self.needs_redraw = true;
    }

    pub fn push_thinking_text(&mut self, text: &str) {
        // Clear the placeholder "..." when real thinking starts
        if !self.is_thinking && self.streaming_text == "..." {
            self.streaming_text.clear();
        }
        self.is_thinking = true;
        self.thinking_text.push_str(text);
        self.needs_redraw = true;
    }

    pub fn finish_streaming(&mut self) {
        if !self.streaming_text.is_empty() {
            self.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: self.streaming_text.clone(),
            });
        }
        self.streaming_text.clear();
        self.thinking_text.clear();
        self.is_streaming = false;
        self.is_thinking = false;
        self.needs_redraw = true;
    }

    pub fn push_error(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Error: {msg}"),
        });
        self.is_streaming = false;
        self.needs_redraw = true;
    }

    pub fn tool_started(&mut self, name: &str) {
        let label = friendly_tool_name(name);
        let line_start = self.streaming_text.len();
        // Add a newline before if the streaming text doesn't end with one
        if !self.streaming_text.ends_with('\n') && !self.streaming_text.is_empty() {
            self.streaming_text.push('\n');
        }
        self.streaming_text.push_str(&format!("  \u{23F3} {label}..."));
        self.pending_tools.push(PendingTool {
            name: name.to_string(),
            line_start,
        });
        self.needs_redraw = true;
    }

    pub fn tool_finished(&mut self, name: &str, is_error: bool) {
        let label = friendly_tool_name(name);
        let symbol = if is_error { "\u{2717}" } else { "\u{2713}" };

        // Find and remove the pending tool entry
        if let Some(pos) = self.pending_tools.iter().position(|t| t.name == name) {
            let _ = self.pending_tools.remove(pos);
            // Replace the running line in-place with the done line
            let running = format!("  \u{23F3} {label}...");
            let done = format!("  {symbol} {label}");
            if let Some(idx) = self.streaming_text.find(&running) {
                self.streaming_text.replace_range(idx..idx + running.len(), &done);
            } else {
                // Fallback: just append
                self.streaming_text.push_str(&format!("\n{done}"));
            }
        } else {
            // No pending tool found — just append
            let done = format!("  {symbol} {label}");
            if !self.streaming_text.ends_with('\n') && !self.streaming_text.is_empty() {
                self.streaming_text.push('\n');
            }
            self.streaming_text.push_str(&done);
        }
        self.needs_redraw = true;
    }
}

pub fn friendly_tool_name(name: &str) -> &'static str {
    match name {
        "bash" => "Shell",
        "file_read" => "Reading",
        "file_write" => "Writing",
        "file_edit" => "Editing",
        "glob" => "Searching",
        "grep" => "Searching",
        "agent" => "Sub-agent",
        _ => "Tool",
    }
}

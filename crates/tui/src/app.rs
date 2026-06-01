// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusty_core::{ConversationSession, PermissionDecision, PermissionRequest};
use std::collections::BTreeMap;
use tokio::sync::oneshot;

/// Maximum allowed paste length (100KB of text). Prevents OOM from huge pastes.
const MAX_PASTE_LENGTH: usize = 100 * 1024;

/// Sanitize pasted text by removing control characters and potential terminal injection.
///
/// This strips:
/// - All C0 control characters except `\n` and `\t`
/// - All C1 control characters (0x80-0x9F)
/// - ANSI escape sequences (`\x1b[...`)
/// - Unicode bidi override characters (U+202A-U+202E, U+2066-U+2069)
/// - Zero-width characters that could cause cursor desync
///
/// The text is also truncated to `MAX_PASTE_LENGTH` bytes.
pub fn sanitize_paste_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len().min(MAX_PASTE_LENGTH));
    let mut chars = text.chars().peekable();
    let mut chars_skipped = 0u32;

    while let Some(ch) = chars.next() {
        match ch {
            // Allow newline and tab
            '\n' | '\t' => result.push(ch),

            // Strip ANSI escape sequences: ESC [ ... final_byte
            '\x1b' => {
                // Skip the entire escape sequence
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c >= '@' && c <= '~' {
                            break; // final byte of CSI sequence
                        }
                    }
                    chars_skipped += 1;
                }
                // Also strip ESC followed by other chars (OSC, etc.)
                else if let Some(&c) = chars.peek() {
                    if c == ']' || c == '(' || c == ')' || c == '#' || c == 'P' {
                        // OSC/SCS/DCS sequences - skip until ST (ESC \) or BEL
                        let prefix = c;
                        chars.next();
                        if prefix == ']' {
                            // OSC: terminated by BEL (0x07) or ST (ESC \)
                            while let Some(&c2) = chars.peek() {
                                chars.next();
                                if c2 == '\x07' {
                                    break;
                                }
                                if c2 == '\x1b' && chars.peek() == Some(&'\\') {
                                    chars.next();
                                    break;
                                }
                            }
                        }
                    }
                    chars_skipped += 1;
                }
            }

            // Strip C0 control characters (0x00-0x1F) except \n and \t (handled above)
            c if c.is_control() => {
                chars_skipped += 1;
                continue;
            }

            // Strip Unicode bidi override characters (security risk)
            '\u{202A}' | '\u{202B}' | '\u{202C}' | '\u{202D}' | '\u{202E}' |
            '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{2069}' |
            '\u{200E}' | '\u{200F}' => {
                chars_skipped += 1;
                continue;
            }

            // Strip zero-width characters that can cause cursor desync
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}' => {
                chars_skipped += 1;
                continue;
            }

            // All other printable characters pass through
            c => result.push(c),
        }
    }

    if chars_skipped > 0 {
        tracing::debug!("Sanitized paste: stripped {chars_skipped} control/escape chars");
    }

    // Truncate if needed
    if result.len() > MAX_PASTE_LENGTH {
        tracing::warn!(
            "Paste truncated from {} to {} bytes",
            result.len(),
            MAX_PASTE_LENGTH
        );
        result.truncate(MAX_PASTE_LENGTH);
    }

    result
}

/// Sanitize text for single-line insertion (used when pasting into a single-line input).
/// Strips control chars and converts newlines to spaces.
pub fn sanitize_single_line(text: &str) -> String {
    sanitize_paste_text(text)
        .replace('\n', " ")
        .replace('\t', "    ")
}

/// Content type for pasted data
#[derive(Debug, Clone)]
pub enum PastedContentType {
    /// Multi-line text content
    Text(String),
    /// Image data (raw bytes from clipboard)
    Image {
        /// Raw image bytes (e.g., PNG, JPEG, or RGBA)
        data: Vec<u8>,
        /// Image format hint (e.g., "png", "jpeg", "rgba")
        format: String,
        /// Width if known (for raw RGBA data)
        width: Option<u32>,
        /// Height if known
        height: Option<u32>,
    },
}

/// A single piece of pasted content stored for later reconstruction
#[derive(Debug, Clone)]
pub struct PastedContent {
    pub content_type: PastedContentType,
    pub order: usize,
}

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
    ToolStart { name: String, arguments: String },
    ToolDone { name: String, is_error: bool, output: String },
    ResponseComplete(String),
    Error(String),
    Usage { input_tokens: u32, output_tokens: u32 },
}

/// Messages from the TUI to the agent task
pub enum TuiCommand {
    /// Regular chat message
    Chat(String),
    /// Cancel the currently running agent turn
    Cancel,
    /// Force compaction of conversation history
    Compact,
    /// Clear conversation and start fresh
    Clear,
    /// Resume a session (session id, messages)
    ResumeSession(String, Vec<rusty_core::Message>),
    /// Rename the current session
    SessionRename(String),
}

/// Slash commands the user can invoke
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// /help — show available commands
    Help,
    /// /init — generate AGENTS.md for the current codebase
    Init,
    /// /resume — show session picker
    Resume,
    /// /sessions — list saved sessions inline
    Sessions,
    /// /compact — force message compaction
    Compact,
    /// /clear — clear current conversation
    Clear,
    /// /quit — exit
    Quit,
    /// /copy — copy last assistant response to clipboard
    Copy,
    /// /model — show current model name
    Model,
    /// /rename — rename the current session
    Rename,
}

impl SlashCommand {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        match trimmed {
            "/help" | "/h" | "/?" => Some(SlashCommand::Help),
            "/init" => Some(SlashCommand::Init),
            "/resume" | "/r" => Some(SlashCommand::Resume),
            "/sessions" | "/s" => Some(SlashCommand::Sessions),
            "/compact" => Some(SlashCommand::Compact),
            "/clear" => Some(SlashCommand::Clear),
            "/quit" | "/exit" | "/q" => Some(SlashCommand::Quit),
            "/copy" | "/c" => Some(SlashCommand::Copy),
            "/model" | "/m" => Some(SlashCommand::Model),
            "/rename" => Some(SlashCommand::Rename),
            _ if trimmed.starts_with("/rename ") => Some(SlashCommand::Rename),
            _ => None,
        }
    }

    pub fn all_descriptions() -> Vec<(&'static str, &'static str)> {
        vec![
            ("/help", "Show this help message"),
            ("/init", "Generate AGENTS.md for the current codebase"),
            ("/resume", "Open session picker to resume a previous session"),
            ("/sessions", "List saved sessions"),
            ("/compact", "Force compaction of conversation history"),
            ("/clear", "Clear current conversation"),
            ("/copy", "Copy last assistant response to clipboard"),
            ("/model", "Show current model name"),
            ("/rename", "Rename the current session"),
            ("/quit", "Exit rusty"),
        ]
    }
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

pub struct SessionPickerState {
    pub sessions: Vec<SessionEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
}

pub struct SessionEntry {
    pub id: String,
    pub preview: String,
    pub message_count: usize,
    pub model: String,
    pub updated_at: String,
}

impl SessionPickerState {
    pub fn from_sessions(sessions: Vec<ConversationSession>) -> Self {
        let entries: Vec<SessionEntry> = sessions
            .iter()
            .map(|s| {
                let preview = s
                    .messages
                    .last()
                    .map(|m| {
                        let text = m.get_all_text();
                        // Take first line, truncate to 60 chars
                        let first_line = text.lines().next().unwrap_or(&text);
                        if first_line.len() > 60 {
                            format!("{}...", &first_line[..60])
                        } else {
                            first_line.to_string()
                        }
                    })
                    .unwrap_or_default();
                SessionEntry {
                    id: s.id.clone(),
                    preview,
                    message_count: s.messages.len(),
                    model: s.model.clone(),
                    updated_at: s.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                }
            })
            .collect();
        Self {
            sessions: entries,
            selected: 0,
            scroll_offset: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.sessions.len() {
            self.selected += 1;
            let visible_rows = 10; // matches the overlay height
            if self.selected >= self.scroll_offset + visible_rows {
                self.scroll_offset = self.selected - visible_rows + 1;
            }
        }
    }
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
    /// Flag to request cancellation of the current agent turn
    pub cancel_requested: bool,
    /// Message queued while streaming is active
    pub queued_message: Option<String>,
    pub needs_redraw: bool,
    pub should_quit: bool,
    pub permission_prompt: Option<PermissionPromptState>,
    pub session_picker: Option<SessionPickerState>,
    /// Tools currently executing — name -> start index in streaming_text
    pub pending_tools: Vec<PendingTool>,
    /// Whether we're waiting for clear confirmation
    pub clear_pending: bool,
    /// Whether we're in rename mode (waiting for new name)
    pub is_renaming: bool,
    /// Current session name (if renamed)
    pub session_name: Option<String>,
    /// Whether thinking text should be shown collapsed (just line count)
    pub thinking_collapsed: bool,
    /// Saved thinking text after thinking phase ends
    pub saved_thinking: String,
    /// Number of thinking lines (for collapsed display)
    pub thinking_line_count: usize,
    /// Whether thinking is expanded (full text visible)
    pub thinking_expanded: bool,
    /// Stored pasted content keyed by placeholder ID
    pub pasted_content: BTreeMap<String, PastedContent>,
    /// Counter for generating unique paste IDs
    pub paste_counter: usize,
}

pub struct PendingTool {
    pub name: String,
    pub arguments: String,
    /// Index in streaming_text where this tool's line starts
    pub line_start: usize,
    pub output: Option<String>,
    pub is_error: bool,
}

pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Default)]
pub struct StatusInfo {
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub is_processing: bool,
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
            session_picker: None,
            pending_tools: Vec::new(),
            clear_pending: false,
            is_renaming: false,
            session_name: None,
            thinking_collapsed: true,
            saved_thinking: String::new(),
            thinking_line_count: 0,
            thinking_expanded: false,
            pasted_content: BTreeMap::new(),
            paste_counter: 0,
        }
    }
}

impl AppState {
    pub fn handle_key(&mut self, key: KeyEvent) {
        // If session picker is active, handle it exclusively
        if let Some(ref mut picker) = self.session_picker {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.move_up();
                    self.needs_redraw = true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    picker.move_down();
                    self.needs_redraw = true;
                }
                KeyCode::Enter => {
                    // Selection is handled in the main loop (needs access to sessions data)
                    self.needs_redraw = true;
                }
                KeyCode::Esc => {
                    self.session_picker = None;
                    self.needs_redraw = true;
                }
                _ => {}
            }
            return;
        }

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

        // If clear confirmation is pending, handle y/n
        if self.clear_pending {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.clear_pending = false;
                    self.messages.clear();
                    self.streaming_text.clear();
                    self.thinking_text.clear();
                    self.saved_thinking.clear();
                    self.thinking_line_count = 0;
                    self.thinking_expanded = false;
                    self.status = StatusInfo::default();
                    self.needs_redraw = true;
                    // Signal clear to main loop via special marker
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "__CLEAR__".to_string(),
                    });
                }
                _ => {
                    self.clear_pending = false;
                    self.needs_redraw = true;
                }
            }
            return;
        }

        // If rename mode is active, handle text input for new name
        if self.is_renaming {
            match key.code {
                KeyCode::Enter => {
                    let name = self.input.trim().to_string();
                    if !name.is_empty() {
                        self.session_name = Some(name.clone());
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("Session renamed to: {name}"),
                        });
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("__RENAME__{name}"),
                        });
                    }
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.is_renaming = false;
                    self.needs_redraw = true;
                }
                KeyCode::Esc => {
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.is_renaming = false;
                    self.needs_redraw = true;
                }
                KeyCode::Char(c) => {
                    self.input.insert(self.cursor_pos, c);
                    self.cursor_pos += 1;
                    self.needs_redraw = true;
                }
                KeyCode::Backspace
                    if self.cursor_pos > 0 => {
                        self.cursor_pos -= 1;
                        self.input.remove(self.cursor_pos);
                        self.needs_redraw = true;
                    }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL)
                && !self.is_streaming => {
                // Paste from clipboard
                self.paste_from_clipboard();
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL)
                // Toggle thinking expand/collapse
                && (self.thinking_line_count > 0 || self.is_thinking) => {
                    self.thinking_expanded = !self.thinking_expanded;
                    self.needs_redraw = true;
                }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL)
                && self.input.is_empty() => {
                    self.should_quit = true;
                }
            KeyCode::Esc if self.is_streaming => {
                // Cancel streaming — reset to ready state
                self.is_streaming = false;
                self.is_thinking = false;
                self.pending_tools.clear();
                if !self.streaming_text.is_empty() {
                    self.messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: std::mem::take(&mut self.streaming_text),
                    });
                }
                self.thinking_text.clear();
                self.saved_thinking.clear();
                self.thinking_line_count = 0;
                self.thinking_expanded = false;
                self.needs_redraw = true;
            }
            KeyCode::Char(c) if !self.is_streaming => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
                self.needs_redraw = true;
            }
            KeyCode::Backspace if !self.is_streaming
                && self.cursor_pos > 0 => {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                    self.needs_redraw = true;
                }
            KeyCode::Delete if !self.is_streaming
                && self.cursor_pos < self.input.len() => {
                    self.input.remove(self.cursor_pos);
                    self.needs_redraw = true;
                }
            KeyCode::Left if !self.is_streaming
                && self.cursor_pos > 0 => {
                    self.cursor_pos -= 1;
                    self.needs_redraw = true;
                }
            KeyCode::Right if !self.is_streaming
                && self.cursor_pos < self.input.len() => {
                    self.cursor_pos += 1;
                    self.needs_redraw = true;
                }
            KeyCode::Home if !self.is_streaming => {
                self.cursor_pos = 0;
                self.needs_redraw = true;
            }
            KeyCode::End if !self.is_streaming => {
                self.cursor_pos = self.input.len();
                self.needs_redraw = true;
            }
            KeyCode::Up if !self.is_streaming
                && !self.history.is_empty() => {
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
            KeyCode::Tab if !self.is_streaming
                // Tab-complete slash commands
                && self.input.starts_with('/') => {
                    self.autocomplete_slash();
                }
            _ => {}
        }
    }

    /// Tab-complete slash commands from partial input
    fn autocomplete_slash(&mut self) {
        let partial = self.input.trim().to_lowercase();
        let commands = [
            "/help", "/init", "/resume", "/sessions", "/compact", "/clear", "/copy", "/model", "/rename", "/quit",
        ];
        let matches: Vec<&str> = commands
            .iter()
            .filter(|c| c.starts_with(&partial))
            .copied()
            .collect();

        if matches.len() == 1 {
            self.input = matches[0].to_string();
            self.cursor_pos = self.input.len();
            self.needs_redraw = true;
        } else if matches.len() > 1 {
            // Find longest common prefix
            let mut prefix = matches[0].to_string();
            for m in &matches[1..] {
                while !m.starts_with(&prefix) {
                    prefix.pop();
                }
            }
            if prefix.len() > partial.len() {
                self.input = prefix;
                self.cursor_pos = self.input.len();
                self.needs_redraw = true;
            }
        }
    }

    /// Read clipboard via arboard and handle the paste
    fn paste_from_clipboard(&mut self) {
        use arboard::Clipboard;

        let clipboard_text = match Clipboard::new() {
            Ok(mut ctx) => match ctx.get_text() {
                Ok(text) => text,
                Err(_) => return, // clipboard empty or unsupported
            },
            Err(_) => return, // can't access clipboard
        };

        // Also try to get an image from the clipboard
        let clipboard_image = match Clipboard::new() {
            Ok(mut ctx) => match ctx.get_image() {
                Ok(img) => Some(img),
                Err(_) => None,
            },
            Err(_) => None,
        };

        self.handle_paste_text(&clipboard_text, clipboard_image);
    }

    /// Process pasted text: sanitize it, then decide whether to use a placeholder
    /// (multi-line) or insert directly (single-line). Also handles image data.
    pub fn handle_paste_text(
        &mut self,
        raw_text: &str,
        image_data: Option<arboard::ImageData>,
    ) {
        // Always sanitize first
        let text = sanitize_paste_text(raw_text);

        if text.is_empty() && image_data.is_none() {
            return;
        }

        let has_newlines = text.contains('\n');
        let is_long = text.len() > 500; // threshold for "too long for inline"
        let has_image = image_data.is_some();

        if has_image {
            // Store image as placeholder
            let img = image_data.unwrap();
            let (w, h) = (img.width as u32, img.height as u32);
            // Convert RGBA to PNG for storage/transmission
            let png_data = rgba_to_png(&img.bytes, w, h);
            let format = if png_data.is_some() { "png" } else { "rgba" };
            let data = png_data.unwrap_or_else(|| img.bytes.to_vec());
            let placeholder = self.add_pasted_image(
                data,
                format.to_string(),
                Some(w),
                Some(h),
            );
            self.input.insert_str(self.cursor_pos, &placeholder);
            self.cursor_pos += placeholder.len();
        } else if has_newlines || is_long {
            // Multi-line or long text → store as placeholder
            let placeholder = self.add_pasted_text(text);
            self.input.insert_str(self.cursor_pos, &placeholder);
            self.cursor_pos += placeholder.len();
        } else {
            // Short single-line → insert directly (already sanitized)
            self.input.insert_str(self.cursor_pos, &text);
            self.cursor_pos += text.len();
        }
        self.needs_redraw = true;
    }

    /// Handle terminal bracketed paste event (from crossterm Event::Paste).
    /// This is called when the terminal sends paste events via bracketed paste mode.
    pub fn handle_bracketed_paste(&mut self, text: String) {
        self.handle_paste_text(&text, None);
    }

    /// Check if the current input is a slash command (starts with /)
    pub fn is_slash_input(&self) -> bool {
        self.input.starts_with('/')
    }

    /// Returns true if Enter was pressed and we have input to send
    pub fn take_pending_input(&mut self) -> Option<String> {
        None
    }

    pub fn push_streaming_text(&mut self, text: &str) {
        // If we were thinking, mark thinking as done and save it
        if self.is_thinking {
            self.is_thinking = false;
            self.saved_thinking = self.thinking_text.clone();
            self.thinking_line_count = self.thinking_text.lines().count();
            self.thinking_text.clear();
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
        // Save any remaining thinking text
        if self.is_thinking && !self.thinking_text.is_empty() {
            self.saved_thinking = self.thinking_text.clone();
            self.thinking_line_count = self.thinking_text.lines().count();
        }
        self.streaming_text.clear();
        self.thinking_text.clear();
        self.pending_tools.clear();
        self.is_streaming = false;
        self.is_thinking = false;
        self.thinking_expanded = false;
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

    pub fn push_system(&mut self, msg: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: msg.to_string(),
        });
        self.needs_redraw = true;
    }

    pub fn tool_started(&mut self, name: &str, arguments: &str) {
        let label = format_tool_label(name, arguments);
        let line_start = self.streaming_text.len();
        if !self.streaming_text.ends_with('\n') && !self.streaming_text.is_empty() {
            self.streaming_text.push('\n');
        }
        // Trailing newline ensures subsequent text deltas start on their own line
        self.streaming_text.push_str(&format!("  \u{25B6} {label}\n"));
        self.pending_tools.push(PendingTool {
            name: name.to_string(),
            arguments: arguments.to_string(),
            line_start,
            output: None,
            is_error: false,
        });
        self.needs_redraw = true;
    }

    pub fn tool_finished(&mut self, name: &str, is_error: bool, output: &str) {
        let label = format_tool_label(name, &self.pending_tools.iter().find(|t| t.name == name).map(|t| t.arguments.clone()).unwrap_or_default());
        let symbol = if is_error { "\u{2717}" } else { "\u{2713}" };
        let summary = tool_output_summary(name, output);
        let done_header = if summary.is_empty() {
            format!("  {symbol} {label}\n")
        } else {
            format!("  {symbol} {label} \u{2014} {summary}\n")
        };

        // Find and remove the pending tool entry
        if let Some(pos) = self.pending_tools.iter().position(|t| t.name == name) {
            self.pending_tools.remove(pos);
            // Must match the indicator used in tool_started (including trailing newline)
            let running = format!("  \u{25B6} {label}\n");

            if let Some(idx) = self.streaming_text.find(&running) {
                self.streaming_text.replace_range(idx..idx + running.len(), &done_header);
            } else {
                if !self.streaming_text.ends_with('\n') {
                    self.streaming_text.push('\n');
                }
                self.streaming_text.push_str(&done_header);
            }
        } else {
            // No pending tool found — just append
            if !self.streaming_text.ends_with('\n') && !self.streaming_text.is_empty() {
                self.streaming_text.push('\n');
            }
            self.streaming_text.push_str(&done_header);
        }
        self.needs_redraw = true;
    }

    /// Generate a unique paste placeholder ID
    fn next_paste_id(&mut self) -> String {
        self.paste_counter += 1;
        format!("PASTE{}", self.paste_counter)
    }

    /// Add pasted text content and return the placeholder string
    pub fn add_pasted_text(&mut self, text: String) -> String {
        let id = self.next_paste_id();
        let line_count = text.lines().count();
        let placeholder = format!(" ⟦Pasted Content: {} lines, id={}⟧ ", line_count, id);
        let order = self.pasted_content.len();
        self.pasted_content.insert(
            id.clone(),
            PastedContent {
                content_type: PastedContentType::Text(text),
                order,
            },
        );
        placeholder
    }

    /// Add pasted image content and return the placeholder string
    pub fn add_pasted_image(&mut self, data: Vec<u8>, format: String, width: Option<u32>, height: Option<u32>) -> String {
        let id = self.next_paste_id();
        let size_str = if data.len() > 1024 * 1024 {
            format!("{:.1}MB", data.len() as f64 / (1024.0 * 1024.0))
        } else {
            format!("{}KB", data.len() / 1024)
        };
        let placeholder = format!(" ⟦Image {}, id={}⟧ ", size_str, id);
        let order = self.pasted_content.len();
        self.pasted_content.insert(
            id.clone(),
            PastedContent {
                content_type: PastedContentType::Image { data, format, width, height },
                order,
            },
        );
        placeholder
    }

    /// Reconstruct full input text with pasted content inlined
    pub fn reconstruct_input(&self) -> String {
        let mut result = self.input.clone();
        for (id, content) in &self.pasted_content {
            let text_content = match &content.content_type {
                PastedContentType::Text(text) => text.clone(),
                PastedContentType::Image { data, format, width, height } => {
                    use base64::Engine;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                    let dims = match (width, height) {
                        (Some(w), Some(h)) => format!(" ({w}x{h})"),
                        _ => String::new(),
                    };
                    format!("[Image: {} {}, {}{}]", format, encoded, data.len(), dims)
                }
            };
            let _placeholder = format!(" ⟦Pasted Content: lines, id={}⟧ ", id);
            // Search for any placeholder containing this id
            let pattern_start = result.find(&format!("⟦Pasted Content:"));
            if pattern_start.is_none() {
                // Try image pattern
                let img_pattern = format!("⟦Image ");
                if let Some(start) = result.find(&img_pattern) {
                    if let Some(end) = result[start..].find("⟧") {
                        let full_end = start + end + "⟧".len();
                        result.replace_range(start..full_end, &text_content);
                    }
                }
            } else if let Some(start) = pattern_start {
                if let Some(end) = result[start..].find("⟧") {
                    let full_end = start + end + "⟧".len();
                    result.replace_range(start..full_end, &text_content);
                }
            }
        }
        result
    }

    /// Clear all stored pasted content
    pub fn clear_pasted_content(&mut self) {
        self.pasted_content.clear();
        self.paste_counter = 0;
    }

    /// Check if input contains paste placeholders
    pub fn has_paste_placeholders(&self) -> bool {
        !self.pasted_content.is_empty()
    }

    /// Parse input text to find paste placeholder boundaries for rendering
    pub fn find_paste_placeholders(input: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut pos = 0;
        while pos < input.len() {
            if let Some(start) = input[pos..].find("⟦") {
                let abs_start = pos + start;
                if let Some(end) = input[abs_start..].find("⟧") {
                    let abs_end = abs_start + end + "⟧".len();
                    ranges.push((abs_start, abs_end));
                    pos = abs_end;
                    continue;
                }
            }
            break;
        }
        ranges
    }
}

pub fn format_tool_label(name: &str, arguments: &str) -> String {
    let detail = extract_tool_detail(name, arguments);
    match name {
        "bash" => format!("Bash({detail})"),
        "file_read" => format!("Read({detail})"),
        "file_write" => format!("Write({detail})"),
        "file_edit" => format!("Edit({detail})"),
        "glob" => format!("Glob({detail})"),
        "grep" => format!("Grep({detail})"),
        "agent" => format!("Sub-agent({detail})"),
        _ => format!("{name}({detail})"),
    }
}

fn extract_tool_detail(name: &str, arguments: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(arguments) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    match name {
        "bash" => {
            let cmd = v["command"].as_str().unwrap_or("");
            if cmd.len() > 60 { format!("{}...", &cmd[..60]) } else { cmd.to_string() }
        }
        "file_read" | "file_write" | "file_edit" => {
            let path = v["file_path"].as_str().or_else(|| v["path"].as_str()).unwrap_or("");
            // Show just the filename, not the full path
            std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string())
        }
        "glob" => v["pattern"].as_str().unwrap_or("").to_string(),
        "grep" => {
            let pattern = v["pattern"].as_str().unwrap_or("");
            let include = v["include"].as_str().unwrap_or("");
            if include.is_empty() {
                pattern.to_string()
            } else {
                format!("{pattern} ({include})")
            }
        }
        "agent" => {
            let task = v["task"].as_str().unwrap_or("");
            if task.len() > 60 { format!("{}...", &task[..60]) } else { task.to_string() }
        }
        _ => String::new(),
    }
}

/// Generate a clean one-line summary for tool output, like Claude Code does.
fn tool_output_summary(name: &str, output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let line_count = trimmed.lines().count();

    match name {
        "file_read" => {
            format!("{line_count} lines")
        }
        "file_write" => {
            format!("{line_count} lines written")
        }
        "file_edit" => {
            // Count added/removed lines from diff output
            let added = trimmed.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
            let removed = trimmed.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
            if added > 0 || removed > 0 {
                format!("+{added} -{removed} lines")
            } else {
                format!("{line_count} lines")
            }
        }
        "bash" => {
            // Show first non-empty line, truncated
            let first = trimmed.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let clean = first.trim();
            if clean.len() > 60 {
                format!("{}...", &clean[..60])
            } else if clean.is_empty() {
                format!("{line_count} lines")
            } else {
                clean.to_string()
            }
        }
        "glob" => {
            // "Found N files"
            if let Some(first) = trimmed.lines().next() {
                if first.starts_with("Found") {
                    return first.to_string();
                }
            }
            format!("{line_count} lines")
        }
        "grep" => {
            // "N matches"
            if let Some(first) = trimmed.lines().next() {
                if first.contains("match") {
                    return first.to_string();
                }
            }
            format!("{line_count} matches")
        }
        "agent" => {
            format!("{line_count} lines")
        }
        "web_fetch" => {
            format!("{line_count} lines")
        }
        _ => {
            format!("{line_count} lines")
        }
    }
}

/// Convert raw RGBA bytes to PNG format for clipboard image storage.
/// Returns None if the encoding fails (e.g., invalid dimensions).
fn rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }

    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = match encoder.write_header() {
            Ok(w) => w,
            Err(_) => return None,
        };
        if writer.write_image_data(rgba).is_err() {
            return None;
        }
    }
    Some(png_data)
}

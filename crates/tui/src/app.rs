// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rusty_core::{ConversationSession, PermissionDecision, PermissionRequest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

/// Threshold for detecting paste events from timing.
/// Events arriving faster than this (in nanoseconds) are considered part of a paste.
/// Human typing is typically 50-200ms between keys; paste events arrive in <5ms.
const PASTE_DETECT_THRESHOLD_NS: u128 = 5_000_000; // 5ms in nanoseconds

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
    Usage { input_tokens: u32, output_tokens: u32, cached_input_tokens: u32, current_context_tokens: u32 },
    ThinkingLevel(Option<rusty_core::ThinkingLevel>),
    ModelChanged(String, u32),
    UpdateAvailable(rusty_core::update::UpdateCheckResult),
    PlanMode(bool),
}

/// Messages from the TUI to the agent task
pub enum TuiCommand {
    /// Regular chat message
    Chat(Vec<rusty_core::ContentBlock>),
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
    /// Switch to a different model by alias or provider/model
    SwitchModel(String),
    /// Set thinking level
    SetThinkingLevel(Option<rusty_core::ThinkingLevel>),
    /// Set permission mode
    SetPermissionMode(rusty_core::PermissionMode),
    /// Add a new model entry to settings
    AddModel(rusty_core::ModelEntry),
    /// Update an existing model entry (old_name, new_entry)
    UpdateModel(String, rusty_core::ModelEntry),
    /// Delete a model entry by name
    DeleteModel(String),
    /// Set API key for a model (name, key)
    SetModelApiKey(String, String),
}

/// Mode for the model form popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelFormMode {
    Add,
    Edit(String), // Edit stores the original name for update
}

/// Editable fields in the model form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFormField {
    Name,
    ModelId,
    Provider,
    ApiBase,
    ApiKey,
    MaxTokens,
    Temperature,
    ThinkingBudget,
}

impl ModelFormField {
    pub const ALL: [ModelFormField; 8] = [
        ModelFormField::Name,
        ModelFormField::ModelId,
        ModelFormField::Provider,
        ModelFormField::ApiBase,
        ModelFormField::ApiKey,
        ModelFormField::MaxTokens,
        ModelFormField::Temperature,
        ModelFormField::ThinkingBudget,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::ModelId => "Model ID",
            Self::Provider => "Provider",
            Self::ApiBase => "API Base URL",
            Self::ApiKey => "API Key",
            Self::MaxTokens => "Max Tokens",
            Self::Temperature => "Temperature",
            Self::ThinkingBudget => "Thinking Budget",
        }
    }
}

/// State for the model add/edit form popup.
#[derive(Debug, Clone)]
pub struct ModelFormState {
    pub mode: ModelFormMode,
    pub current_field: usize,
    pub field_buffers: [String; 8],
    pub field_cursors: [usize; 8],
    pub error: Option<String>,
    pub confirm_delete: bool,
}

impl ModelFormState {
    /// Create a blank form for adding a new model.
    pub fn new_add() -> Self {
        Self {
            mode: ModelFormMode::Add,
            current_field: 0,
            field_buffers: [
                String::new(),       // Name
                String::new(),       // Model ID
                "OpenAI".to_string(), // Provider (default)
                String::new(),       // API Base
                String::new(),       // API Key
                "16384".to_string(), // Max Tokens
                String::new(),       // Temperature
                String::new(),       // Thinking Budget
            ],
            field_cursors: [0, 0, 7, 0, 0, 5, 0, 0], // cursor after "OpenAI" and "16384"
            error: None,
            confirm_delete: false,
        }
    }

    /// Create a form pre-filled from an existing model entry for editing.
    pub fn new_edit(entry: &crate::model_registry::ModelEntry) -> Self {
        Self {
            mode: ModelFormMode::Edit(entry.name.clone()),
            current_field: 0,
            field_buffers: [
                entry.name.clone(),
                entry.model.clone(),
                entry.provider.to_string(),
                entry.api_base.clone(),
                String::new(), // API key not shown for security
                entry.max_tokens.to_string(),
                entry.temperature.map(|t| format!("{t}")).unwrap_or_default(),
                entry.thinking_budget.map(|t| t.to_string()).unwrap_or_default(),
            ],
            field_cursors: [
                entry.name.len(),
                entry.model.len(),
                entry.provider.to_string().len(),
                entry.api_base.len(),
                0,
                entry.max_tokens.to_string().len(),
                entry.temperature.map(|t| format!("{t}").len()).unwrap_or(0),
                entry.thinking_budget.map(|t| t.to_string().len()).unwrap_or(0),
            ],
            error: None,
            confirm_delete: false,
        }
    }

    /// Validate the form and return a ModelEntry if valid.
    pub fn build_entry(&self) -> Result<rusty_core::ModelEntry, String> {
        let name = self.field_buffers[0].trim().to_string();
        let model_id = self.field_buffers[1].trim().to_string();
        let api_base = self.field_buffers[3].trim().to_string();
        let max_tokens_str = self.field_buffers[5].trim().to_string();
        let temp_str = self.field_buffers[6].trim().to_string();
        let budget_str = self.field_buffers[7].trim().to_string();

        if name.is_empty() {
            return Err("Name is required".into());
        }
        if model_id.is_empty() {
            return Err("Model ID is required".into());
        }
        if api_base.is_empty() {
            return Err("API Base URL is required".into());
        }

        let max_tokens: u32 = if max_tokens_str.is_empty() {
            16384
        } else {
            max_tokens_str.parse().map_err(|_| "Max Tokens must be a number")?
        };

        let temperature: Option<f32> = if temp_str.is_empty() {
            None
        } else {
            Some(temp_str.parse().map_err(|_| "Temperature must be a number")?)
        };

        let thinking_budget: Option<u32> = if budget_str.is_empty() {
            None
        } else {
            Some(budget_str.parse().map_err(|_| "Thinking Budget must be a number")?)
        };

        Ok(rusty_core::ModelEntry {
            group: String::new(),
            name,
            provider: rusty_core::ProviderType::OpenAI,
            api_base,
            model: model_id,
            available_models: Vec::new(),
            max_tokens,
            temperature,
            thinking_budget,
            extra_headers: None,
            context_window: None,
        })
    }
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
    /// /permissions — manage allowed tools (always-approve list)
    Permissions,
    /// /settings — open the settings/model registry TUI
    Settings,
    /// /version — show current version and check for updates
    Version,
    /// /plan — enter explicit plan mode (read-only planning)
    Plan,
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
            "/permissions" | "/perms" => Some(SlashCommand::Permissions),
            _ if trimmed.starts_with("/permissions ") || trimmed.starts_with("/perms ") => Some(SlashCommand::Permissions),
            "/settings" => Some(SlashCommand::Settings),
            "/version" | "/v" => Some(SlashCommand::Version),
            "/plan" => Some(SlashCommand::Plan),
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
            ("/permissions", "Manage always-approved tools list"),
            ("/settings", "Open settings and model registry"),
            ("/version", "Show current version and update status"),
            ("/plan", "Enter explicit plan mode for planning only"),
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
                        if first_line.chars().count() > 60 {
                            format!("{}...", first_line.chars().take(60).collect::<String>())
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

/// Which tab is active in the settings overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsTab {
    /// Browse the model registry and switch the active model.
    Models,
    /// View/edit general settings (thinking level, permissions, etc.).
    General,
}

/// A single row in the settings panel's model list.
/// Group headers are not selectable — only model entries are.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// A group header (e.g. "Kimi", "DeepSeek"). Not selectable.
    GroupHeader { name: String, count: usize },
    /// A model entry. The `usize` indexes into `SettingsState.models`.
    ModelEntry(usize),
}

/// Rows displayed in the General settings tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralRow {
    ThinkingLevel,
    PermissionMode,
}

impl GeneralRow {
    pub const ALL: &'static [GeneralRow] = &[GeneralRow::ThinkingLevel, GeneralRow::PermissionMode];
}

/// Cycle to the next permission mode.
pub fn next_permission_mode(current: rusty_core::PermissionMode) -> rusty_core::PermissionMode {
    match current {
        rusty_core::PermissionMode::Default => rusty_core::PermissionMode::AcceptEdits,
        rusty_core::PermissionMode::AcceptEdits => rusty_core::PermissionMode::BypassPermissions,
        rusty_core::PermissionMode::BypassPermissions => rusty_core::PermissionMode::Plan,
        rusty_core::PermissionMode::Plan => rusty_core::PermissionMode::Default,
    }
}

/// Display label for a permission mode.
pub fn permission_mode_label(mode: rusty_core::PermissionMode) -> &'static str {
    match mode {
        rusty_core::PermissionMode::Default => "default",
        rusty_core::PermissionMode::AcceptEdits => "accept-edits",
        rusty_core::PermissionMode::BypassPermissions => "bypass",
        rusty_core::PermissionMode::Plan => "plan",
    }
}

/// Tracks the state of the `/settings` TUI overlay.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Currently active tab.
    pub active_tab: SettingsTab,
    /// Index into `models` Vec of the currently highlighted model (Models tab).
    pub selected: usize,
    /// Scroll offset so long lists don't overflow the viewport.
    pub scroll: usize,
    /// Cached list of models from the registry.
    pub models: Vec<crate::model_registry::ModelEntry>,
    /// The `name` of the model that is currently active.
    pub active_model_name: String,
    /// Which model entry is expanded to show `available_models` (if any).
    pub expanded: Option<usize>,
    /// Whether the user is in "edit mode" for a specific field (future extensibility).
    pub edit_buffer: Option<String>,
    /// Currently highlighted row in the General tab.
    pub general_selected: usize,
    /// Current thinking level value (for General tab display and cycling).
    pub general_thinking_level: Option<rusty_core::ThinkingLevel>,
    /// Current permission mode value (for General tab display and cycling).
    pub general_permission_mode: rusty_core::PermissionMode,
}

impl SettingsState {
    /// Create a new settings state seeded from the current runtime config.
    pub fn new(
        models: Vec<crate::model_registry::ModelEntry>,
        active_model_name: String,
        thinking_level: Option<rusty_core::ThinkingLevel>,
        permission_mode: rusty_core::PermissionMode,
    ) -> Self {
        let selected = models
            .iter()
            .position(|m| m.name == active_model_name)
            .unwrap_or(0);
        Self {
            active_tab: SettingsTab::Models,
            selected,
            scroll: 0,
            expanded: None,
            models,
            active_model_name,
            edit_buffer: None,
            general_selected: 0,
            general_thinking_level: thinking_level,
            general_permission_mode: permission_mode,
        }
    }

    /// Move cursor up (skipping group headers in Models tab).
    pub fn select_previous(&mut self) {
        match self.active_tab {
            SettingsTab::General => {
                if self.general_selected > 0 {
                    self.general_selected -= 1;
                }
            }
            SettingsTab::Models => {
                let rows = self.display_rows();
                let current_display_idx = rows
                    .iter()
                    .position(|r| matches!(r, DisplayRow::ModelEntry(i) if *i == self.selected));
                if let Some(pos) = current_display_idx {
                    for row in rows[..pos].iter().rev() {
                        if let DisplayRow::ModelEntry(idx) = row {
                            self.selected = *idx;
                            self.expanded = None;
                            // Adjust scroll
                            if self.selected < self.scroll {
                                self.scroll = self.selected;
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Move cursor down (skipping group headers in Models tab).
    pub fn select_next(&mut self) {
        match self.active_tab {
            SettingsTab::General => {
                if self.general_selected + 1 < GeneralRow::ALL.len() {
                    self.general_selected += 1;
                }
            }
            SettingsTab::Models => {
                let rows = self.display_rows();
                let current_display_idx = rows
                    .iter()
                    .position(|r| matches!(r, DisplayRow::ModelEntry(i) if *i == self.selected));
                if let Some(pos) = current_display_idx {
                    for row in rows[pos + 1..].iter() {
                        if let DisplayRow::ModelEntry(idx) = row {
                            self.selected = *idx;
                            self.expanded = None;
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Cycle tabs right.
    pub fn next_tab(&mut self) {
        self.active_tab = match self.active_tab {
            SettingsTab::Models => SettingsTab::General,
            SettingsTab::General => SettingsTab::Models,
        };
    }

    /// Cycle tabs left (same as right with 2 tabs).
    pub fn prev_tab(&mut self) {
        self.next_tab();
    }

    /// Returns the model entry under the cursor, if any.
    pub fn selected_model(&self) -> Option<&crate::model_registry::ModelEntry> {
        self.models.get(self.selected)
    }

    /// Toggle expansion of the currently selected entry to show `available_models`.
    pub fn toggle_expand(&mut self) {
        if self.expanded == Some(self.selected) {
            self.expanded = None;
        } else {
            self.expanded = Some(self.selected);
        }
    }

    /// Build the display rows: group headers interspersed with model entries.
    /// Groups are ordered by first appearance in `models`.
    pub fn display_rows(&self) -> Vec<DisplayRow> {
        let mut rows = Vec::new();
        let mut seen_groups: Vec<String> = Vec::new();

        for (i, entry) in self.models.iter().enumerate() {
            let group_name = entry.group.clone();
            if !seen_groups.contains(&group_name) {
                let count = self.models.iter().filter(|e| e.group == group_name).count();
                rows.push(DisplayRow::GroupHeader {
                    name: group_name.clone(),
                    count,
                });
                seen_groups.push(group_name);
            }
            rows.push(DisplayRow::ModelEntry(i));
        }
        rows
    }
}

/// Entry in the file picker
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Display path (relative to working_dir)
    pub display: String,
    /// Full absolute path
    pub full_path: String,
    /// File size in bytes (for display)
    pub size: Option<u64>,
    /// Whether this is a directory
    pub is_dir: bool,
}

/// State for the file picker overlay (triggered by @)
pub struct FilePickerState {
    /// Current search pattern (text after @)
    pub query: String,
    /// Matching file paths from glob search
    pub matches: Vec<FileEntry>,
    /// Currently selected index
    pub selected: usize,
    /// Scroll offset for long lists
    pub scroll_offset: usize,
    /// Working directory for resolving paths
    pub working_dir: PathBuf,
    /// Cursor position where @ was typed (position after @)
    pub at_position: usize,
}

impl FilePickerState {
    pub fn new(working_dir: String, at_position: usize) -> Self {
        let mut picker = Self {
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            working_dir: PathBuf::from(working_dir),
            at_position,
        };
        picker.update_matches();
        picker
    }

    pub fn update_matches(&mut self) {
        use glob::Pattern;
        use walkdir::WalkDir;

        let pattern_str = if self.query.is_empty() {
            "**/*".to_string()
        } else {
            format!("**/*{}*", self.query)
        };

        let matcher = match Pattern::new(&pattern_str) {
            Ok(m) => m,
            Err(_) => return,
        };

        // Directories to skip (build artifacts, dependencies, etc.)
        const SKIP_DIRS: &[&str] = &[
            "node_modules", "target", "__pycache__", ".git", ".svn", ".hg",
            "dist", "build", ".next", ".nuxt", ".cache", "vendor", "venv",
            ".venv", "env", ".tox", ".mypy_cache", ".pytest_cache",
            "coverage", ".turbo", ".parcel-cache",
        ];

        let mut matches = Vec::new();
        let max_results = 100;

        let walker = WalkDir::new(&self.working_dir)
            .follow_links(false)
            .into_iter();

        for entry in walker.filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            // Skip hidden directories
            if name.starts_with('.') {
                return false;
            }
            // Skip known large directories
            if e.file_type().is_dir() && SKIP_DIRS.contains(&name) {
                return false;
            }
            true
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let relative = path
                .strip_prefix(&self.working_dir)
                .unwrap_or(path)
                .to_string_lossy();

            if matcher.matches(&relative) {
                let display = if relative.is_empty() {
                    ".".to_string()
                } else {
                    relative.to_string()
                };

                matches.push(FileEntry {
                    display,
                    full_path: path.display().to_string(),
                    size: entry.metadata().ok().map(|m| m.len()),
                    is_dir: entry.file_type().is_dir(),
                });

                if matches.len() >= max_results {
                    break;
                }
            }
        }

        // Sort: directories first, then by name
        matches.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| a.display.cmp(&b.display))
        });

        self.matches = matches;
        self.selected = 0;
        self.scroll_offset = 0;
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
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
            let visible_rows = 15; // matches the overlay height
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
    /// Working directory to display in the status bar.
    pub working_dir: Option<String>,
    pub streaming_text: String,
    pub thinking_text: String,
    pub is_streaming: bool,
    pub is_thinking: bool,
    /// Flag to request cancellation of the current agent turn
    pub cancel_requested: bool,
    /// Message queued while streaming is active
    pub queued_message: Option<String>,
    /// Pre-built content blocks for the queued message (preserves image blocks)
    pub queued_blocks: Option<Vec<rusty_core::ContentBlock>>,
    pub needs_redraw: bool,
    pub should_quit: bool,
    pub permission_prompt: Option<PermissionPromptState>,
    pub session_picker: Option<SessionPickerState>,
    /// Settings overlay state (for /settings)
    pub settings_overlay: Option<SettingsState>,
    /// Tools currently executing — name -> start index in streaming_text
    pub pending_tools: Vec<PendingTool>,
    /// Whether we're waiting for clear confirmation
    pub clear_pending: bool,
    /// Whether we're in rename mode (waiting for new name)
    pub is_renaming: bool,
    /// Current session name (if renamed)
    pub session_name: Option<String>,
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
    /// Pinned todo list text shown at bottom of chat area
    pub pinned_todos: Option<String>,
    /// File picker state for @ file references
    pub file_picker: Option<FilePickerState>,
    /// Timestamp of the last key event, used for paste detection
    last_key_time: Option<Instant>,
    /// Whether we're currently in paste mode (rapid input detected)
    pub paste_mode: bool,
    /// Number of lines the user has scrolled up from the bottom (0 = at bottom)
    pub scroll_offset: usize,
    /// Set by the Enter handler in the Models tab — the TUI main loop
    /// picks this up and sends `TuiCommand::SwitchModel` to the agent task.
    pub model_switch_requested: Option<String>,
    /// Model form popup state (add/edit model).
    pub model_form: Option<ModelFormState>,
    /// Set by the Enter handler in the General tab — the TUI main loop
    /// picks this up and sends `TuiCommand::SetThinkingLevel` to the agent task.
    pub thinking_level_change_requested: Option<Option<rusty_core::ThinkingLevel>>,
    /// Set by the Enter handler in the General tab — the TUI main loop
    /// picks this up and sends `TuiCommand::SetPermissionMode` to the agent task.
    pub permission_mode_change_requested: Option<rusty_core::PermissionMode>,
    /// Whether the agent is currently in explicit plan mode.
    pub plan_mode: bool,
    /// Queue of commands dispatched by key handlers that the main event loop
    /// picks up and sends to the agent task (needed because key handlers
    /// borrow `self` mutably so we can't send directly).
    pub pending_commands: std::collections::VecDeque<TuiCommand>,
    /// Last known viewport (chat area) height in rows, set by the draw loop.
    pub viewport_height: usize,
    /// Slash command queued for execution by the main loop (set by keymap dispatch).
    pub slash_command: Option<String>,
    /// New version available from GitHub, if detected by the background update check.
    pub update_available: Option<String>,
    /// Autocomplete popup state for slash commands.
    pub autocomplete: Option<AutocompleteState>,
}

pub struct PendingTool {
    pub name: String,
    pub arguments: String,
    /// Index in streaming_text where this tool's line starts
    pub line_start: usize,
    pub output: Option<String>,
    pub is_error: bool,
}

/// All available slash commands with their descriptions
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show available commands"),
    ("/init", "Generate AGENTS.md for the codebase"),
    ("/resume", "Resume a saved session"),
    ("/sessions", "List saved sessions"),
    ("/compact", "Force conversation compaction"),
    ("/clear", "Clear current conversation"),
    ("/copy", "Copy last assistant response"),
    ("/model", "Show current model"),
    ("/rename", "Rename current session"),
    ("/permissions", "Manage tool permissions"),
    ("/settings", "Open settings overlay"),
    ("/version", "Show version and check for updates"),
    ("/plan", "Enter explicit plan mode for planning only"),
    ("/quit", "Exit"),
];

/// State for the slash command autocomplete popup.
#[derive(Debug, Clone)]
pub struct AutocompleteState {
    /// Matching commands (command string, description)
    pub matches: Vec<(String, String)>,
    /// Currently selected index in the matches list
    pub selected: usize,
}

impl AutocompleteState {
    /// Compute matches for the given input text. Returns None if input does not start
    /// with `/` or there are no matching commands.
    pub fn compute(input: &str) -> Option<Self> {
        if !input.starts_with('/') {
            return None;
        }
        let partial = input.trim().to_lowercase();
        let matches: Vec<(String, String)> = SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd.starts_with(&partial))
            .map(|(cmd, desc)| (cmd.to_string(), desc.to_string()))
            .collect();

        if matches.is_empty() {
            None
        } else {
            Some(Self { matches, selected: 0 })
        }
    }

    /// Get the currently selected command text.
    pub fn selected_command(&self) -> &str {
        &self.matches[self.selected].0
    }
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
    pub context_window: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_input_tokens: u32,
    pub current_context_tokens: u32,
    pub is_processing: bool,
    pub thinking_level: Option<rusty_core::ThinkingLevel>,
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
            cancel_requested: false,
            queued_message: None,
            queued_blocks: None,
            needs_redraw: true,
            should_quit: false,
            permission_prompt: None,
            session_picker: None,
            settings_overlay: None,
            pending_tools: Vec::new(),
            clear_pending: false,
            is_renaming: false,
            session_name: None,
            saved_thinking: String::new(),
            thinking_line_count: 0,
            thinking_expanded: false,
            pasted_content: BTreeMap::new(),
            paste_counter: 0,
            pinned_todos: None,
            working_dir: None,
            file_picker: None,
            last_key_time: None,
            paste_mode: false,
            scroll_offset: 0,
            model_switch_requested: None,
            model_form: None,
            thinking_level_change_requested: None,
            permission_mode_change_requested: None,
            plan_mode: false,
            pending_commands: std::collections::VecDeque::new(),
            viewport_height: 20,
            slash_command: None,
            update_available: None,
            autocomplete: None,
        }
    }
}

impl AppState {
    pub fn handle_key(&mut self, key: KeyEvent) {
        // Track timing for paste detection.
        // If events arrive faster than PASTE_DETECT_THRESHOLD_NS, we're in a paste.
        let now = Instant::now();
        if let Some(prev) = self.last_key_time {
            let elapsed_ns = now.duration_since(prev).as_nanos();
            if elapsed_ns < PASTE_DETECT_THRESHOLD_NS {
                self.paste_mode = true;
            } else {
                // Gap too large — normal typing or new interaction
                self.paste_mode = false;
            }
        }
        self.last_key_time = Some(now);

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

        // If file picker is active, handle it exclusively
        if let Some(ref mut picker) = self.file_picker {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.move_up();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    picker.move_down();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Enter | KeyCode::Tab => {
                    // Extract what we need to avoid borrow checker issues
                    let selected_entry = picker.matches.get(picker.selected).cloned();
                    let at_pos = picker.at_position;
                    if let Some(entry) = selected_entry {
                        // Replace @query with @selected_path in input
                        if at_pos <= self.input.len() {
                            let replace_end = self.cursor_pos;
                            if at_pos <= replace_end {
                                let reference = entry.display.clone();
                                self.input.replace_range(at_pos..replace_end, &reference);
                                self.cursor_pos = at_pos + reference.len();
                            }
                        }
                    }
                    self.file_picker = None;
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Esc => {
                    self.file_picker = None;
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Char(c) => {
                    picker.query.push(c);
                    picker.update_matches();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Backspace => {
                    picker.query.pop();
                    if picker.query.is_empty() && picker.at_position == self.cursor_pos {
                        // Backspaced all query text, close picker
                        self.file_picker = None;
                    } else {
                        picker.update_matches();
                    }
                    self.needs_redraw = true;
                    return;
                }
                _ => {}
            }
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

        // If model form is active, handle it exclusively
        if let Some(ref mut form) = self.model_form {
            // If in delete confirmation mode
            if form.confirm_delete {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let ModelFormMode::Edit(ref name) = form.mode {
                            let model_name = name.clone();
                            self.model_form = None;
                            self.pending_commands.push_back(TuiCommand::DeleteModel(model_name));
                        }
                        self.needs_redraw = true;
                    }
                    _ => {
                        form.confirm_delete = false;
                        self.needs_redraw = true;
                    }
                }
                return;
            }

            match key.code {
                KeyCode::Esc => {
                    self.model_form = None;
                    self.needs_redraw = true;
                }
                KeyCode::Tab | KeyCode::Down => {
                    form.current_field = (form.current_field + 1) % ModelFormField::ALL.len();
                    form.error = None;
                    self.needs_redraw = true;
                }
                KeyCode::BackTab | KeyCode::Up => {
                    form.current_field = if form.current_field == 0 {
                        ModelFormField::ALL.len() - 1
                    } else {
                        form.current_field - 1
                    };
                    form.error = None;
                    self.needs_redraw = true;
                }
                KeyCode::Left => {
                    let cursor = &mut form.field_cursors[form.current_field];
                    *cursor = cursor.saturating_sub(1);
                    self.needs_redraw = true;
                }
                KeyCode::Right => {
                    let buf_len = form.field_buffers[form.current_field].len();
                    let cursor = &mut form.field_cursors[form.current_field];
                    if *cursor < buf_len {
                        *cursor += 1;
                    }
                    self.needs_redraw = true;
                }
                KeyCode::Char('d') if form.mode != ModelFormMode::Add && form.current_field == 0 => {
                    // 'd' on Name field in edit mode triggers delete confirmation
                    form.confirm_delete = true;
                    self.needs_redraw = true;
                }
                KeyCode::Enter => {
                    // Submit the form
                    match form.build_entry() {
                        Ok(entry) => {
                            let api_key = form.field_buffers[4].trim().to_string();
                            let model_name = entry.name.clone();
                            let old_name = match &form.mode {
                                ModelFormMode::Edit(name) => Some(name.clone()),
                                ModelFormMode::Add => None,
                            };
                            self.model_form = None;
                            if let Some(old) = old_name {
                                self.pending_commands.push_back(TuiCommand::UpdateModel(old, entry));
                            } else {
                                self.pending_commands.push_back(TuiCommand::AddModel(entry));
                            }
                            if !api_key.is_empty() {
                                self.pending_commands.push_back(TuiCommand::SetModelApiKey(model_name, api_key));
                            }
                            self.needs_redraw = true;
                        }
                        Err(e) => {
                            form.error = Some(e);
                            self.needs_redraw = true;
                        }
                    }
                }
                KeyCode::Char(c) => {
                    // Provider field is read-only
                    if ModelFormField::ALL[form.current_field] == ModelFormField::Provider {
                        return;
                    }
                    let buf = &mut form.field_buffers[form.current_field];
                    let cursor = form.field_cursors[form.current_field];
                    if cursor <= buf.len() {
                        buf.insert(cursor, c);
                        form.field_cursors[form.current_field] += c.len_utf8();
                        form.error = None;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Backspace => {
                    let cursor = form.field_cursors[form.current_field];
                    if cursor > 0 {
                        let buf = &mut form.field_buffers[form.current_field];
                        let prev = buf[..cursor]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        buf.drain(prev..cursor);
                        form.field_cursors[form.current_field] = prev;
                        form.error = None;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Delete => {
                    let cursor = form.field_cursors[form.current_field];
                    let buf = &mut form.field_buffers[form.current_field];
                    if cursor < buf.len() {
                        let next = buf[cursor..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| cursor + i)
                            .unwrap_or(buf.len());
                        buf.drain(cursor..next);
                        self.needs_redraw = true;
                    }
                }
                _ => {}
            }
            return;
        }

        // If settings overlay is active, handle it exclusively
        if let Some(ref mut settings) = self.settings_overlay {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    settings.select_previous();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    settings.select_next();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Tab => {
                    settings.next_tab();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::BackTab => {
                    settings.prev_tab();
                    self.needs_redraw = true;
                    return;
                }
                KeyCode::Enter => {
                    match settings.active_tab {
                        SettingsTab::Models => {
                            // Signal the main loop to switch to the selected model.
                            if let Some(entry) = settings.selected_model() {
                                self.model_switch_requested = Some(entry.name.clone());
                            }
                            self.settings_overlay = None; // Close settings
                            self.needs_redraw = true;
                        }
                        SettingsTab::General => {
                            match GeneralRow::ALL.get(settings.general_selected) {
                                Some(GeneralRow::ThinkingLevel) => {
                                    let next = crate::model_registry::next_thinking_level(settings.general_thinking_level);
                                    settings.general_thinking_level = next;
                                    self.thinking_level_change_requested = Some(next);
                                    self.needs_redraw = true;
                                }
                                Some(GeneralRow::PermissionMode) => {
                                    let next = next_permission_mode(settings.general_permission_mode);
                                    settings.general_permission_mode = next;
                                    self.permission_mode_change_requested = Some(next);
                                    self.needs_redraw = true;
                                }
                                None => {}
                            }
                        }
                    }
                    return;
                }
                KeyCode::Char(' ') => {
                    // Space toggles expansion of available_models
                    if settings.active_tab == SettingsTab::Models {
                        settings.toggle_expand();
                        self.needs_redraw = true;
                    }
                    return;
                }
                KeyCode::Char('a') => {
                    // Add new model (only in Models tab)
                    if settings.active_tab == SettingsTab::Models {
                        self.model_form = Some(ModelFormState::new_add());
                        self.settings_overlay = None;
                        self.needs_redraw = true;
                    }
                    return;
                }
                KeyCode::Char('e') => {
                    // Edit selected model (only in Models tab)
                    if settings.active_tab == SettingsTab::Models {
                        if let Some(entry) = settings.selected_model() {
                            let entry_clone = entry.clone();
                            self.model_form = Some(ModelFormState::new_edit(&entry_clone));
                            self.settings_overlay = None;
                            self.needs_redraw = true;
                        }
                    }
                    return;
                }
                KeyCode::Char('d') => {
                    // Delete selected model (only in Models tab)
                    if settings.active_tab == SettingsTab::Models {
                        if let Some(entry) = settings.selected_model() {
                            let name = entry.name.clone();
                            // Check if it's the active model
                            if settings.active_model_name == name {
                                // Cannot delete active model — could show error but for now just ignore
                                self.needs_redraw = true;
                            } else {
                                self.pending_commands.push_back(TuiCommand::DeleteModel(name));
                                self.needs_redraw = true;
                            }
                        }
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.settings_overlay = None;
                    self.needs_redraw = true;
                    return;
                }
                _ => {} // Fall through for other keys
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
                    self.cursor_pos += c.len_utf8();
                    self.needs_redraw = true;
                }
                KeyCode::Backspace
                    if self.cursor_pos > 0 => {
                        let prev = self.input[..self.cursor_pos]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.input.drain(prev..self.cursor_pos);
                        self.cursor_pos = prev;
                        self.needs_redraw = true;
                    }
                _ => {}
            }
            return;
        }

        match key.code {
            // Autocomplete popup navigation
            KeyCode::Up if self.autocomplete.is_some() => {
                let ac = self.autocomplete.as_mut().unwrap();
                ac.selected = ac.selected.saturating_sub(1);
                self.needs_redraw = true;
                return;
            }
            KeyCode::Down if self.autocomplete.is_some() => {
                let ac = self.autocomplete.as_mut().unwrap();
                if ac.selected + 1 < ac.matches.len() {
                    ac.selected += 1;
                }
                self.needs_redraw = true;
                return;
            }
            KeyCode::Tab if self.autocomplete.is_some() => {
                let cmd = self.autocomplete.as_ref().unwrap().selected_command().to_string();
                self.input = cmd;
                self.cursor_pos = self.input.len();
                self.autocomplete = None;
                self.needs_redraw = true;
                return;
            }
            KeyCode::Enter if self.autocomplete.is_some() => {
                let cmd = self.autocomplete.as_ref().unwrap().selected_command().to_string();
                self.input = cmd;
                self.cursor_pos = self.input.len();
                self.autocomplete = None;
                self.needs_redraw = true;
                // Fall through to submit the command
            }
            KeyCode::Esc if self.autocomplete.is_some() => {
                self.autocomplete = None;
                self.needs_redraw = true;
                return;
            }
            _ => {}
        }
        // Dismiss autocomplete on any other key if it's active (e.g. typing continues)
        // but only for non-character keys that we haven't handled above.
        // Character input will update autocomplete below.

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Paste from clipboard (allowed even while streaming)
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
                // Request cancellation — the TUI main loop will send TuiCommand::Cancel
                self.cancel_requested = true;
                self.needs_redraw = true;
            }
            KeyCode::PageUp => {
                self.scroll_up(15);
            }
            KeyCode::PageDown => {
                self.scroll_down(15);
            }
            KeyCode::Char('@') if !self.is_streaming => {
                // Insert the @ character
                self.input.insert(self.cursor_pos, '@');
                self.cursor_pos += 1;
                
                // Open file picker
                let working_dir = self.working_dir.clone().unwrap_or_else(|| {
                    std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                });
                self.file_picker = Some(FilePickerState::new(
                    working_dir,
                    self.cursor_pos,
                ));
                self.needs_redraw = true;
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += c.len_utf8();
                self.autocomplete = AutocompleteState::compute(&self.input);
                self.needs_redraw = true;
            }
            KeyCode::Backspace if self.cursor_pos > 0 => {
                let prev = self.input[..self.cursor_pos]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.input.drain(prev..self.cursor_pos);
                self.cursor_pos = prev;
                self.autocomplete = AutocompleteState::compute(&self.input);
                self.needs_redraw = true;
            }
            KeyCode::Delete if self.cursor_pos < self.input.len() => {
                let next = self.input[self.cursor_pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| self.cursor_pos + i)
                    .unwrap_or(self.input.len());
                self.input.drain(self.cursor_pos..next);
                self.autocomplete = AutocompleteState::compute(&self.input);
                self.needs_redraw = true;
            }
            KeyCode::Left if self.cursor_pos > 0 => {
                self.cursor_pos = self.input[..self.cursor_pos]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.needs_redraw = true;
            }
            KeyCode::Right if self.cursor_pos < self.input.len() => {
                self.cursor_pos = self.input[self.cursor_pos..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| self.cursor_pos + i)
                    .unwrap_or(self.input.len());
                self.needs_redraw = true;
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                self.needs_redraw = true;
            }
            KeyCode::End => {
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
                // Tab-complete slash commands only when input is a bare command prefix (no spaces)
                && self.input.starts_with('/')
                && !self.input.contains(' ') => {
                    if self.autocomplete.is_none() {
                        self.autocomplete = AutocompleteState::compute(&self.input);
                    }
                    if let Some(ref ac) = self.autocomplete {
                        if !ac.matches.is_empty() {
                            let cmd = ac.matches[ac.selected].0.clone();
                            self.input = cmd;
                            self.cursor_pos = self.input.len();
                            self.autocomplete = None;
                            self.needs_redraw = true;
                        }
                    }
                }
            KeyCode::Enter if self.paste_mode => {
                // In paste mode, insert newline instead of submitting
                self.input.insert(self.cursor_pos, '\n');
                self.cursor_pos += 1;
                self.paste_mode = false;
                self.needs_redraw = true;
            }
            _ => {}
        }
    }



    /// Scroll up by `n` lines (toward older messages).
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
        self.needs_redraw = true;
    }

    /// Scroll down by `n` lines (toward newer messages).
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.needs_redraw = true;
    }

    /// Jump to the bottom (most recent content). Resets scroll offset to 0.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.needs_redraw = true;
    }

    /// Scroll to the very top (oldest message).
    pub fn scroll_top(&mut self) {
        self.scroll_offset = usize::MAX;
        self.needs_redraw = true;
    }

    /// Scroll to the very bottom (newest message). Alias for scroll_to_bottom.
    pub fn scroll_bottom(&mut self) {
        self.scroll_to_bottom();
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

    /// Insert a file reference from the file picker into the input
    pub fn insert_file_reference(&mut self, entry: &FileEntry) {
        if let Some(picker) = &self.file_picker {
            let at_pos = picker.at_position;
            // Find the @ position in input
            if at_pos <= self.input.len() {
                // Calculate what to replace: from @ to cursor
                let replace_end = self.cursor_pos;
                if at_pos <= replace_end {
                    let reference = entry.display.clone();
                    self.input.replace_range(at_pos..replace_end, &reference);
                    self.cursor_pos = at_pos + reference.len();
                }
            }
        }
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

    /// Record a parsed slash command for the main loop to consume.
    /// Clears the input buffer after recording.
    pub fn execute_slash_command(
        &mut self,
        cmd: SlashCommand,
        _cmd_tx: &mpsc::UnboundedSender<TuiCommand>,
    ) {
        let name = match cmd {
            SlashCommand::Help => "help",
            SlashCommand::Init => "init",
            SlashCommand::Resume => "resume",
            SlashCommand::Sessions => "sessions",
            SlashCommand::Compact => "compact",
            SlashCommand::Clear => "clear",
            SlashCommand::Quit => "quit",
            SlashCommand::Copy => "copy",
            SlashCommand::Model => "model",
            SlashCommand::Rename => "rename",
            SlashCommand::Permissions => "permissions",
            SlashCommand::Settings => "settings",
            SlashCommand::Version => "version",
            SlashCommand::Plan => "plan",
        };
        self.slash_command = Some(name.to_string());
        self.input.clear();
        self.cursor_pos = 0;
        self.needs_redraw = true;
    }

    /// Returns true if Enter was pressed and we have input to send
    pub fn take_pending_input(&mut self) -> Option<String> {
        None
    }

    /// Take the cancel-requested flag, resetting it.
    pub fn take_cancel_requested(&mut self) -> bool {
        std::mem::take(&mut self.cancel_requested)
    }

    /// Take a queued message if one exists.
    pub fn take_queued_message(&mut self) -> Option<String> {
        self.queued_message.take()
    }

    /// Take pre-built content blocks for the queued message.
    pub fn take_queued_blocks(&mut self) -> Option<Vec<rusty_core::ContentBlock>> {
        self.queued_blocks.take()
    }

    /// Queue the current input for sending after streaming finishes.
    /// Builds content blocks (including image blocks) before clearing input.
    /// Adds a temporary system message so the queued text is visible inline.
    pub fn queue_current_input(&mut self) {
        if !self.input.is_empty() {
            let text = self.input.clone();
            self.queued_blocks = Some(self.build_content_blocks());
            self.queued_message = Some(std::mem::take(&mut self.input));
            self.cursor_pos = 0;
            self.clear_pasted_content();
            self.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("Queued: {}", text),
            });
            self.needs_redraw = true;
        }
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
        // Flush any partial streaming content as an assistant message before the error
        if !self.streaming_text.is_empty() {
            self.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: self.streaming_text.clone(),
            });
        }
        // Save any remaining thinking text so it remains accessible
        if self.is_thinking && !self.thinking_text.is_empty() {
            self.saved_thinking = self.thinking_text.clone();
            self.thinking_line_count = self.thinking_text.lines().count();
        }

        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Error: {msg}"),
        });

        // Full cleanup of streaming state (mirrors finish_streaming)
        self.streaming_text.clear();
        self.thinking_text.clear();
        self.pending_tools.clear();
        self.is_streaming = false;
        self.is_thinking = false;
        self.thinking_expanded = false;
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
        let summary = tool_output_summary(name, output, is_error);
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

        // For todowrite, update the pinned panel only (don't duplicate inline)
        if name == "todowrite" && !output.trim().is_empty() {
            let has_active = output.contains("[ ]") || output.contains("[~]");
            if has_active {
                self.pinned_todos = Some(output.trim().to_string());
            } else {
                self.pinned_todos = None;
            }
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

    /// Build Vec<ContentBlock> from current input, replacing paste placeholders
    /// with proper content blocks (ContentBlock::Image for pasted images,
    /// inlined text for text pastes).
    pub fn build_content_blocks(&self) -> Vec<rusty_core::ContentBlock> {
        use base64::Engine;

        let input = &self.input;
        if self.pasted_content.is_empty() || !input.contains('\u{27E6}') {
            return vec![rusty_core::ContentBlock::Text { text: input.clone() }];
        }

        let placeholders = Self::find_paste_placeholders(input);
        let mut blocks: Vec<rusty_core::ContentBlock> = Vec::new();
        let mut last_end = 0;

        for (start, end) in &placeholders {
            // Add text before this placeholder
            let text_before = &input[last_end..*start];
            if !text_before.is_empty() {
                blocks.push(rusty_core::ContentBlock::Text { text: text_before.to_string() });
            }

            let placeholder_text = &input[*start..*end];

            // Extract ID from placeholder (format: "id=PASTE_N")
            if let Some(id_start) = placeholder_text.find("id=") {
                let id_part = &placeholder_text[id_start + 3..];
                if let Some(id_end) = id_part.find('\u{27E7}') {
                    let id = &id_part[..id_end];
                    if let Some(pasted) = self.pasted_content.get(id) {
                        match &pasted.content_type {
                            PastedContentType::Text(text) => {
                                blocks.push(rusty_core::ContentBlock::Text { text: text.clone() });
                            }
                            PastedContentType::Image { data, format, .. } => {
                                let media_type = match format.as_str() {
                                    "png" => "image/png",
                                    "jpeg" | "jpg" => "image/jpeg",
                                    "gif" => "image/gif",
                                    "webp" => "image/webp",
                                    _ => "image/png",
                                };
                                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                                blocks.push(rusty_core::ContentBlock::Image {
                                    media_type: media_type.to_string(),
                                    data: b64,
                                });
                            }
                        }
                    } else {
                        blocks.push(rusty_core::ContentBlock::Text { text: placeholder_text.to_string() });
                    }
                } else {
                    blocks.push(rusty_core::ContentBlock::Text { text: placeholder_text.to_string() });
                }
            } else {
                blocks.push(rusty_core::ContentBlock::Text { text: placeholder_text.to_string() });
            }

            last_end = *end;
        }

        // Add remaining text after last placeholder
        let text_after = &input[last_end..];
        if !text_after.is_empty() {
            blocks.push(rusty_core::ContentBlock::Text { text: text_after.to_string() });
        }

        // Merge adjacent text blocks
        let mut merged: Vec<rusty_core::ContentBlock> = Vec::new();
        for block in blocks {
            match block {
                rusty_core::ContentBlock::Text { text } => {
                    if let Some(rusty_core::ContentBlock::Text { text: prev }) = merged.last_mut() {
                        prev.push_str(&text);
                    } else {
                        merged.push(rusty_core::ContentBlock::Text { text });
                    }
                }
                other => merged.push(other),
            }
        }

        if merged.is_empty() {
            merged.push(rusty_core::ContentBlock::Text { text: String::new() });
        }

        merged
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
            if cmd.chars().count() > 60 { format!("{}...", cmd.chars().take(60).collect::<String>()) } else { cmd.to_string() }
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
            if task.chars().count() > 60 { format!("{}...", task.chars().take(60).collect::<String>()) } else { task.to_string() }
        }
        _ => String::new(),
    }
}

/// Generate a clean one-line summary for tool output, like Claude Code does.
fn tool_output_summary(name: &str, output: &str, is_error: bool) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // For errors, show the actual error message (truncated) instead of generic summaries
    if is_error {
        let first_line = trimmed.lines().next().unwrap_or("");
        let clean = first_line.trim();
        if clean.chars().count() > 80 {
            return format!("{}...", clean.chars().take(80).collect::<String>());
        } else if !clean.is_empty() {
            return clean.to_string();
        }
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
            if clean.chars().count() > 60 {
                format!("{}...", clean.chars().take(60).collect::<String>())
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
        "todowrite" => {
            // "Todo List (2/4 completed)"
            if let Some(first) = trimmed.lines().next() {
                if let Some(start) = first.find('(') {
                    if let Some(end) = first[start..].find(')') {
                        return first[start + 1..start + end].to_string();
                    }
                }
            }
            format!("{line_count} lines")
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
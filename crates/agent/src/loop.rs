// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures::StreamExt;
use rusty_core::permissions::{
    classify_bash_command, make_allow_key, BashClassification, PermissionDecision,
    PermissionLevel, PermissionRequest,
};
use rusty_core::{
    model_context_window, Config, ContentBlock, Message, PermissionMode, RustyError, UsageInfo,
};
use rusty_core::{dynamic_thinking_level, level_to_budget, ThinkingLevel};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use rusty_tools::{Tool, ToolContext, ToolResult};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::compact::maybe_compact;

/// Max characters for a tool result stored in conversation history.
/// Large outputs are truncated to prevent context bloat.
const MAX_TOOL_OUTPUT_CHARS: usize = 16_000;

/// Max retries for transient/retryable API errors.
const MAX_RETRIES: u32 = 3;

/// Truncate text at a line boundary to keep output readable.
/// Unlike a raw byte-slice, this never cuts mid-line or mid-UTF8 char.
fn smart_truncate_output(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let slice = &text[..max_chars];
    let cut_at = slice.rfind('\n').unwrap_or(max_chars);

    format!(
        "{}\n\n... (output truncated, showing {} of {} chars)",
        &text[..cut_at],
        cut_at,
        text.len(),
    )
}

/// Callback for streaming text deltas to the UI
pub type TextCallback = Box<dyn Fn(&str) + Send + Sync>;
/// Callback for streaming thinking/reasoning deltas
pub type ThinkingCallback = Box<dyn Fn(&str) + Send + Sync>;

/// Status of a tool execution, sent to the TUI
pub enum ToolStatus {
    Running { arguments: String },
    Done { output: String },
    Error { output: String },
}

/// Callback for tool execution status — receives name and status
pub type ToolCallback = Box<dyn Fn(&str, ToolStatus) + Send + Sync>;
/// Callback for token usage updates
pub type UsageCallback = Box<dyn Fn(u32, u32) + Send + Sync>;
/// Callback for thinking level changes
pub type ThinkingLevelCallback = Box<dyn Fn(Option<ThinkingLevel>) + Send + Sync>;
/// Callback for permission requests — receives a request, returns a decision future
pub type PermissionCallback = Arc<
    dyn Fn(PermissionRequest) -> Pin<Box<dyn Future<Output = PermissionDecision> + Send>>
        + Send
        + Sync,
>;

/// Lightweight cancellation token for cooperative cancellation of agent turns.
/// Checked between stream events so the agent can abort mid-stream or mid-tool.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: HashMap<String, Arc<dyn Tool>>,
    config: Config,
    working_dir: PathBuf,
    messages: Vec<Message>,
    system_prompt: String,
    total_usage: UsageInfo,
    permission_mode: PermissionMode,
    max_turns: u32,
    permission_callback: Option<PermissionCallback>,
    session_allowlist: HashSet<String>,
    permanent_allowlist: HashSet<String>,
}

#[derive(Default)]
pub struct AgentCallbacks<'a> {
    pub on_text: Option<&'a TextCallback>,
    pub on_thinking: Option<&'a ThinkingCallback>,
    pub on_tool: Option<&'a ToolCallback>,
    pub on_usage: Option<&'a UsageCallback>,
    pub on_thinking_level: Option<&'a ThinkingLevelCallback>,
    pub cancel: Option<&'a CancelToken>,
}

impl Agent {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: Vec<Box<dyn Tool>>,
        config: Config,
        working_dir: PathBuf,
        system_prompt: String,
    ) -> Self {
        let tool_map: HashMap<String, Arc<dyn Tool>> = tools
            .into_iter()
            .map(|t| (t.name().to_string(), Arc::from(t)))
            .collect();

        Self {
            provider,
            tools: tool_map,
            config,
            working_dir,
            messages: Vec::new(),
            system_prompt,
            total_usage: UsageInfo::default(),
            permission_mode: PermissionMode::Default,
            max_turns: 50,
            permission_callback: None,
            session_allowlist: HashSet::new(),
            permanent_allowlist: HashSet::new(),
        }
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        self.permission_mode = mode;
    }

    pub fn set_permission_callback(&mut self, cb: PermissionCallback) {
        self.permission_callback = Some(cb);
    }

    pub fn set_permanent_allowlist(&mut self, allowlist: HashSet<String>) {
        self.permanent_allowlist = allowlist;
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    /// Force-compact the conversation history, summarizing older messages.
    /// Returns true if compaction actually happened.
    pub async fn compact(&mut self) -> Result<bool, RustyError> {
        // Take messages out to avoid borrow conflicts with provider
        let mut msgs = std::mem::take(&mut self.messages);
        let result = crate::compact::force_compact(&mut msgs, &*self.provider, &self.system_prompt).await;
        self.messages = msgs;
        result
    }

    pub fn total_usage(&self) -> &UsageInfo {
        &self.total_usage
    }

    /// Run the agent loop: send messages, handle streaming, execute tools, repeat.
    /// Pass a `CancelToken` via callbacks to allow mid-turn cancellation (checked between stream events).
    pub async fn run(
        &mut self,
        user_input: &str,
        callbacks: AgentCallbacks<'_>,
    ) -> Result<String, RustyError> {
        let AgentCallbacks {
            on_text,
            on_thinking,
            on_tool,
            on_usage,
            on_thinking_level,
            cancel,
        } = callbacks;
        if let Some(c) = cancel {
            c.reset();
        }
        self.messages.push(Message::user(user_input));

        for turn in 0..self.max_turns {
            if let Some(c) = cancel {
                if c.is_cancelled() {
                    return Ok("Turn cancelled by user.".to_string());
                }
            }
            debug!("Agent turn {}/{}", turn + 1, self.max_turns);

            // Warn when approaching max turns
            if turn == self.max_turns.saturating_sub(3) && self.max_turns > 5 {
                warn!("Approaching max turns limit ({}/{})", turn + 1, self.max_turns);
            }

            // Maybe compact before sending
            maybe_compact(&mut self.messages, &*self.provider, &self.system_prompt).await?;

            let tool_defs: Vec<_> = self.tools.values().map(|t| t.definition()).collect();

            // Compute dynamic thinking level based on context fill
            let context_window = model_context_window(&self.config.model);
            let estimated_chars: usize = self.messages.iter().map(|m| m.get_all_text().len()).sum();
            let estimated_tokens = estimated_chars / 4;
            let context_pct = estimated_tokens as f64 / context_window as f64;
            let base_level = self.config.resolve_thinking_level();
            let effective_level = dynamic_thinking_level(base_level, context_pct);
            let thinking_budget = Some(level_to_budget(effective_level));

            if effective_level != base_level {
                debug!(
                    "Thinking reduced from {:?} to {:?} (context {:.1}% full)",
                    base_level, effective_level, context_pct * 100.0
                );
            }

            if let Some(cb) = on_thinking_level {
                cb(Some(effective_level));
            }

            let request = MessageRequest {
                model: self.config.model.clone(),
                system: Some(self.system_prompt.clone()),
                messages: self.messages.clone(),
                tools: tool_defs,
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
                thinking_budget,
            };

            debug!("Calling LLM API (model: {}, messages: {})", self.config.model, self.messages.len());
            let mut stream = match self.call_with_retry(&request, on_text).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("LLM API call failed after retries: {e}");
                    return Err(e);
                }
            };

            let mut assistant_text = String::new();
            let mut tool_calls: Vec<ToolCallState> = Vec::new();
            let mut stop_reason = None;

            while let Some(event) = stream.next().await {
                if let Some(c) = cancel {
                    if c.is_cancelled() {
                        return Ok("Turn cancelled by user.".to_string());
                    }
                }
                match event? {
                    StreamEvent::TextDelta(text) => {
                        assistant_text.push_str(&text);
                        if let Some(cb) = on_text {
                            cb(&text);
                        }
                    }
                    StreamEvent::ThinkingDelta(thinking) => {
                        if let Some(cb) = on_thinking {
                            cb(&thinking);
                        }
                    }
                    StreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => {
                        while tool_calls.len() <= index {
                            tool_calls.push(ToolCallState::default());
                        }
                        let tc = &mut tool_calls[index];
                        if let Some(id) = id {
                            tc.id = id;
                        }
                        if let Some(name) = name {
                            tc.name = name;
                        }
                        tc.arguments.push_str(&arguments_delta);
                    }
                    StreamEvent::Usage(usage) => {
                        self.total_usage.input_tokens += usage.input_tokens;
                        self.total_usage.output_tokens += usage.output_tokens;
                        if let Some(cb) = on_usage {
                            cb(self.total_usage.input_tokens, self.total_usage.output_tokens);
                        }
                    }
                    StreamEvent::Done { stop_reason: sr } => {
                        stop_reason = sr;
                        break;
                    }
                    StreamEvent::Error(msg) => {
                        return Err(RustyError::Api(msg));
                    }
                }
            }

            // Estimate tokens if the provider didn't report usage
            // (common with OpenAI-compatible providers that don't support stream_options)
            {
                let total_chars: usize = self.messages.iter().map(|m| m.get_all_text().len()).sum();
                let estimated_input = (total_chars / 4) as u32;
                let estimated_output = (assistant_text.len() / 4) as u32;
                if estimated_input > self.total_usage.input_tokens {
                    self.total_usage.input_tokens = estimated_input;
                }
                if estimated_output > self.total_usage.output_tokens {
                    self.total_usage.output_tokens = estimated_output;
                }
                if let Some(cb) = on_usage {
                    cb(self.total_usage.input_tokens, self.total_usage.output_tokens);
                }
            }

            // Build assistant message
            let mut blocks = Vec::new();
            if !assistant_text.is_empty() {
                blocks.push(ContentBlock::Text {
                    text: assistant_text.clone(),
                });
            }

            // If there are tool calls, execute them
            if !tool_calls.is_empty() {
                if let Some(c) = cancel {
                    if c.is_cancelled() {
                        return Ok("Turn cancelled by user.".to_string());
                    }
                }
                for tc in &tool_calls {
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null);
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input,
                    });
                }

                self.messages.push(Message::assistant_blocks(blocks.clone()));

                // Execute each tool
                let ctx = ToolContext {
                    working_dir: self.working_dir.clone(),
                    permission_mode: self.permission_mode,
                };

                for tc in &tool_calls {
                    if let Some(cb) = on_tool {
                        cb(&tc.name, ToolStatus::Running {
                            arguments: tc.arguments.clone(),
                        });
                    }

                    let result = self.execute_tool(&tc.name, &tc.arguments, &ctx).await;

                    let tool_result = match result {
                        Ok(r) => r,
                        Err(e) => ToolResult::error(e.to_string()),
                    };

                    if let Some(cb) = on_tool {
                        if tool_result.is_error {
                            cb(&tc.name, ToolStatus::Error {
                                output: tool_result.content.clone(),
                            });
                        } else {
                            cb(&tc.name, ToolStatus::Done {
                                output: tool_result.content.clone(),
                            });
                        }
                    }

                    // Truncate large tool outputs before storing in history.
                    // Use line-boundary-aware truncation to keep output readable.
                    let stored_content = if tool_result.content.len() > MAX_TOOL_OUTPUT_CHARS {
                        
                        smart_truncate_output(&tool_result.content, MAX_TOOL_OUTPUT_CHARS)
                    } else {
                        tool_result.content.clone()
                    };

                    self.messages.push(Message::user_blocks(vec![
                        ContentBlock::ToolResult {
                            tool_use_id: tc.id.clone(),
                            content: stored_content,
                            is_error: Some(tool_result.is_error),
                        },
                    ]));
                }

                // Continue the loop — the model needs to see tool results
                continue;
            }

            // No tool calls — store assistant message and return
            if !blocks.is_empty() {
                self.messages.push(Message::assistant_blocks(blocks));
            }

            // Check stop reason
            match stop_reason.as_deref() {
                Some("end_turn") | None => {
                    return Ok(assistant_text);
                }
                Some("max_tokens") => {
                    warn!("Hit max_tokens limit");
                    let warning = format!(
                        "{}\n\n[Response truncated: hit max_tokens limit. Consider using /compact if context is full.]",
                        assistant_text
                    );
                    return Ok(warning);
                }
                Some(other) => {
                    debug!("Unexpected stop reason: {other}");
                    return Ok(assistant_text);
                }
            }
        }

        // Hit max turns — ask the model to summarize progress
        warn!("Max turns ({}) exceeded, requesting summary", self.max_turns);

        let summary_request = MessageRequest {
            model: self.config.model.clone(),
            system: Some(self.system_prompt.clone()),
            messages: {
                let mut msgs = self.messages.clone();
                msgs.push(Message::user(
                    "The turn limit has been reached. Please provide a brief summary of what was accomplished and what tasks remain incomplete. Be concise."
                ));
                msgs
            },
            tools: vec![],
            max_tokens: 1024,
            temperature: self.config.temperature,
            thinking_budget: Some(level_to_budget(ThinkingLevel::Minimal)),
        };

        let mut stream = match self.provider.create_message_stream(summary_request).await {
            Ok(s) => s,
            Err(e) => return Err(e),
        };

        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(text) => {
                    summary.push_str(&text);
                    if let Some(cb) = on_text {
                        cb(&text);
                    }
                }
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }

        if !summary.is_empty() {
            self.messages.push(Message::assistant(&summary));
        }
        Ok(summary)
    }

    /// Call the LLM API with automatic retry for transient errors.
    /// Retries up to MAX_RETRIES times for rate limits and server errors.
    async fn call_with_retry(
        &self,
        request: &MessageRequest,
        on_text: Option<&TextCallback>,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<StreamEvent, RustyError>> + Send>>, RustyError>
    {
        let mut last_err = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(2u64.pow(attempt - 1));
                debug!("Retrying LLM API call (attempt {}/{}) after {delay:?}", attempt, MAX_RETRIES);
                if let Some(cb) = on_text {
                    cb(&format!("\n[Retrying API call (attempt {}/{})...]\n", attempt + 1, MAX_RETRIES + 1));
                }
                tokio::time::sleep(delay).await;
            }

            match self.provider.create_message_stream(request.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    if e.is_retryable() && attempt < MAX_RETRIES {
                        // For rate limits, use the retry_after hint
                        if let RustyError::RateLimit { retry_after: Some(secs) } = &e {
                            let delay = std::time::Duration::from_secs(*secs);
                            debug!("Rate limited, waiting {delay:?} before retry");
                            tokio::time::sleep(delay).await;
                        }
                        warn!("Retryable API error (attempt {}): {e}", attempt + 1);
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| RustyError::Api("Max retries exceeded".into())))
    }

    async fn execute_tool(
        &mut self,
        name: &str,
        arguments: &str,
        ctx: &ToolContext,
    ) -> Result<ToolResult, RustyError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| RustyError::Tool(format!("Unknown tool: {name}")))?;

        // Determine effective permission level (bash gets classified per-command)
        let effective_level = if name == "bash" {
            let input: serde_json::Value = serde_json::from_str(arguments)
                .unwrap_or(serde_json::Value::Null);
            let cmd = input["command"].as_str().unwrap_or("");
            match classify_bash_command(cmd) {
                BashClassification::ReadOnly => PermissionLevel::ReadOnly,
                BashClassification::Write => PermissionLevel::Execute,
            }
        } else {
            tool.permission_level()
        };

        // Tiered permission check
        let decision = self
            .check_permission_tiered(name, arguments, effective_level)
            .await;

        match decision {
            PermissionDecision::AllowOnce => { /* proceed */ }
            PermissionDecision::AllowSession => {
                let key = make_allow_key(name, arguments);
                self.session_allowlist.insert(key);
            }
            PermissionDecision::AllowAlways => {
                let key = make_allow_key(name, arguments);
                self.session_allowlist.insert(key.clone());
                if let Err(e) = rusty_core::config::add_permanent_permission(&key).await {
                    warn!("Failed to save permanent permission: {e}");
                }
            }
            PermissionDecision::Deny(reason) => {
                warn!("Tool {name} denied: {reason}");
                return Ok(ToolResult::error(format!("Permission denied: {reason}")));
            }
        }

        debug!("Executing tool: {name}");
        let input: serde_json::Value =
            serde_json::from_str(arguments).unwrap_or(serde_json::Value::Object(Default::default()));

        tool.execute(input, ctx).await
    }

    async fn check_permission_tiered(
        &self,
        tool_name: &str,
        arguments: &str,
        effective_level: PermissionLevel,
    ) -> PermissionDecision {
        // 1. Bypass mode — allow everything
        if self.permission_mode == PermissionMode::BypassPermissions {
            return PermissionDecision::AllowOnce;
        }

        // 2. Plan mode — deny write/execute
        if self.permission_mode == PermissionMode::Plan {
            if effective_level == PermissionLevel::ReadOnly
                || effective_level == PermissionLevel::None
            {
                return PermissionDecision::AllowOnce;
            }
            return PermissionDecision::Deny("Plan mode is read-only".into());
        }

        // 3. Read-only / None tools — auto-allow
        if effective_level == PermissionLevel::ReadOnly
            || effective_level == PermissionLevel::None
        {
            return PermissionDecision::AllowOnce;
        }

        // 4. AcceptEdits mode — allow Write, but prompt for Execute
        if self.permission_mode == PermissionMode::AcceptEdits
            && effective_level == PermissionLevel::Write
        {
            return PermissionDecision::AllowOnce;
        }

        // 5. Check permanent allowlist
        let key = make_allow_key(tool_name, arguments);
        if self.permanent_allowlist.contains(&key) {
            return PermissionDecision::AllowOnce;
        }

        // 6. Check session allowlist
        if self.session_allowlist.contains(&key) {
            return PermissionDecision::AllowOnce;
        }

        // 7. Interactive prompt via callback
        if let Some(ref cb) = self.permission_callback {
            let desc = rusty_core::permissions::build_tool_description(tool_name, arguments);
            let request = PermissionRequest {
                id: 0,
                tool_name: tool_name.to_string(),
                description: desc,
                raw_input: arguments.to_string(),
                is_read_only: false,
                required_level: effective_level,
            };
            return cb(request).await;
        }

        // No callback — deny by default
        PermissionDecision::Deny("No permission callback configured".into())
    }
}

#[derive(Default)]
struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── smart_truncate_output ────────────────────────────────────────

    #[test]
    fn truncate_short_text_unchanged() {
        let text = "hello";
        assert_eq!(smart_truncate_output(text, 100), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        let text = "abcde";
        assert_eq!(smart_truncate_output(text, 5), "abcde");
    }

    #[test]
    fn truncate_at_line_boundary() {
        let text = "line1\nline2\nline3";
        // max_chars=12 → slice = "line1\nline2\n", rfind('\n') = 11
        let result = smart_truncate_output(text, 12);
        assert!(result.contains("line1\nline2"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_no_newlines_cuts_at_max() {
        let text = "abcdefghijklmnop";
        let result = smart_truncate_output(text, 8);
        // No '\n' found → cut_at = 8
        assert!(result.starts_with("abcdefgh"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_preserves_utf8() {
        // "café" is 5 bytes in UTF-8 (é = 2 bytes)
        let text = "café\nline2";
        let result = smart_truncate_output(text, 6);
        // Should not panic on UTF-8 boundary
        assert!(result.contains("café"));
    }

    #[test]
    fn truncate_empty_text() {
        let result = smart_truncate_output("", 100);
        assert_eq!(result, "");
    }
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures::future::join_all;
use futures::StreamExt;
use rusty_core::permissions::{
    classify_bash_command, make_allow_key, BashClassification, PermissionDecision,
    PermissionLevel, PermissionRequest,
};
use rusty_core::plan::Plan;
use rusty_core::{
    CancelToken, Config, ContentBlock, Message, PermissionMode, Role, RustyError, UsageInfo,
};
use rusty_core::{level_to_budget, ThinkingLevel};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use rusty_tools::{Tool, ToolContext, ToolResult};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

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

    // Find a safe byte boundary at or before max_chars to avoid panicking
    // when max_chars falls in the middle of a multi-byte UTF-8 character.
    let safe = text.floor_char_boundary(max_chars);
    let slice = &text[..safe];
    let cut_at = slice.rfind('\n').unwrap_or(safe);

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
/// Callback for token usage updates — receives (total_input, total_output, current_context_tokens, cached_tokens)
pub type UsageCallback = Box<dyn Fn(u32, u32, u32, u32) + Send + Sync>;
/// Callback for thinking level changes
pub type ThinkingLevelCallback = Box<dyn Fn(Option<ThinkingLevel>) + Send + Sync>;
/// Callback for permission requests — receives a request, returns a decision future
pub type PermissionCallback = Arc<
    dyn Fn(PermissionRequest) -> Pin<Box<dyn Future<Output = PermissionDecision> + Send>>
        + Send
        + Sync,
>;

pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: HashMap<String, Arc<dyn Tool>>,
    config: Config,
    working_dir: PathBuf,
    messages: Vec<Message>,
    system_prompt: String,
    /// Base system prompt without plan context (used to rebuild with fresh plan each turn).
    base_system_prompt: String,
    total_usage: UsageInfo,
    /// The input token count from the most recent API call (current context size).
    /// Unlike `total_usage.input_tokens`, this is NOT accumulated across turns.
    current_context_tokens: u32,
    permission_mode: PermissionMode,
    max_turns: u32,
    permission_callback: Option<PermissionCallback>,
    session_allowlist: HashSet<String>,
    permanent_allowlist: HashSet<String>,
    /// How many times we've nudged the model to finish incomplete tasks.
    /// Prevents infinite loops when the model is stuck.
    task_nudge_count: u32,
    /// Persistent plan, injected into system prompt each turn.
    plan: Option<Arc<tokio::sync::Mutex<Plan>>>,
    /// When true, the agent is in explicit plan mode (no Write/Execute tools allowed).
    pub plan_mode: bool,
    /// When true, the model called exit_plan_mode this turn, so we should not
    /// nudge it to continue working — it has explicitly handed control back.
    exited_plan_mode_this_turn: bool,
    /// How many consecutive turns the model has stopped without making any
    /// write/execute changes. Used to escalate nudges when the model is stuck
    /// in a research loop.
    consecutive_read_turns: u32,
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

        let max_turns = config.max_turns;

        Self {
            provider,
            tools: tool_map,
            config,
            working_dir,
            messages: Vec::new(),
            base_system_prompt: system_prompt.clone(),
            system_prompt,
            total_usage: UsageInfo::default(),
            current_context_tokens: 0,
            permission_mode: PermissionMode::Default,
            max_turns,
            permission_callback: None,
            session_allowlist: HashSet::new(),
            permanent_allowlist: HashSet::new(),
            task_nudge_count: 0,
            plan: None,
            plan_mode: false,
            exited_plan_mode_this_turn: false,
            consecutive_read_turns: 0,
        }
    }

    pub fn set_permission_mode(&mut self, mode: PermissionMode) {
        self.permission_mode = mode;
    }

    pub fn enter_plan_mode(&mut self) {
        self.plan_mode = true;
    }

    pub fn exit_plan_mode(&mut self) {
        self.plan_mode = false;
    }

    pub fn set_permission_callback(&mut self, cb: PermissionCallback) {
        self.permission_callback = Some(cb);
    }

    pub fn set_permanent_allowlist(&mut self, allowlist: HashSet<String>) {
        self.permanent_allowlist = allowlist;
    }

    /// Set the persistent plan for this agent.
    pub fn set_plan(&mut self, plan: Arc<tokio::sync::Mutex<Plan>>) {
        self.plan = Some(plan);
    }

    /// Refresh the system prompt with current plan context.
    /// Called before each LLM call so the model always sees the latest plan state.
    async fn refresh_system_prompt(&mut self) {
        let mut prompt = self.base_system_prompt.clone();
        if let Some(plan) = &self.plan {
            let plan = plan.lock().await;
            let ctx = plan.format_for_system_prompt();
            if !ctx.is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(&ctx);
            }
        }
        self.system_prompt = prompt;
    }

    /// Replace the LLM provider at runtime (used for model switching).
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
    }

    /// Mutable access to the agent's config (e.g. to update model name).
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    /// Reset conversation and task state (used by /clear).
    pub async fn clear_state(&mut self) {
        self.messages.clear();
        self.task_nudge_count = 0;
        self.plan_mode = false;
        if let Some(plan) = &self.plan {
            let mut p = plan.lock().await;
            p.items.clear();
            let _ = p.save();
        }
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

    /// Send a minimal non-streaming request to verify the provider connection.
    /// Returns the model's response text on success, or an error describing
    /// what went wrong (auth, endpoint, model ID, etc.).
    pub async fn test_connection(&self) -> Result<String, RustyError> {
        let request = MessageRequest {
            model: self.config.model.clone(),
            system: None,
            messages: vec![Message::user("Respond with exactly the word pong.")],
            tools: vec![],
            max_tokens: 16,
            temperature: Some(0.0),
            thinking_budget: None,
        };
        let response = self.provider.create_message(request).await?;
        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        Ok(text)
    }

    /// Extract details of incomplete tasks from the most recent `todowrite` call.
    /// Returns a vec of (status, content) pairs for tasks that are not completed/cancelled.
    fn incomplete_task_details(&self) -> Vec<(String, String)> {
        for msg in self.messages.iter().rev().take(10) {
            if msg.role != Role::Assistant {
                continue;
            }
            for block in msg.content_blocks() {
                if let ContentBlock::ToolUse { name, input, .. } = block {
                    if name == "todowrite" {
                        if let Some(todos) = input.get("todos").and_then(|v| v.as_array()) {
                            let incomplete: Vec<(String, String)> = todos
                                .iter()
                                .filter_map(|t| {
                                    // Default to "pending" when status is missing (it's
                                    // optional in the schema and most models omit it).
                                    let status = t
                                        .get("status")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("pending");
                                    if status == "completed" || status == "cancelled" {
                                        return None;
                                    }
                                    let content = t.get("content")?.as_str()?.to_string();
                                    Some((status.to_string(), content))
                                })
                                .collect();
                            if !incomplete.is_empty() {
                                return incomplete;
                            }
                            return vec![];
                        }
                    }
                }
            }
        }
        vec![]
    }

    /// Finalise any `InProgress` plan items to `Completed` and log.
    /// Called on every exit path from the agent loop.
    async fn finalize_plan(&self) {
        if let Some(plan) = &self.plan {
            let mut plan = plan.lock().await;
            let count = plan.finalize_in_progress();
            if count > 0 {
                debug!("Finalised {count} in-progress task(s) to completed");
            }
        }
    }

    /// Run the agent loop: send messages, handle streaming, execute tools, repeat.
    /// Pass a `CancelToken` via callbacks to allow mid-turn cancellation (immediate via `tokio::select!`).
    pub async fn run(
        &mut self,
        content: Vec<ContentBlock>,
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
        self.exited_plan_mode_this_turn = false;
        self.consecutive_read_turns = 0;
        self.messages.push(Message::user_blocks(content));

        for turn in 0..self.max_turns {
            if let Some(c) = cancel {
                if c.is_cancelled() {
                    self.finalize_plan().await;
                    return Ok("Turn cancelled by user.".to_string());
                }
            }
            debug!("Agent turn {}/{}", turn + 1, self.max_turns);

            // Refresh system prompt with current plan context
            self.refresh_system_prompt().await;

            // Warn when approaching max turns
            if turn == self.max_turns.saturating_sub(3) && self.max_turns > 5 {
                warn!("Approaching max turns limit ({}/{})", turn + 1, self.max_turns);
            }

            // Maybe compact before sending
            let context_window = self.config.effective_context_window();
            maybe_compact(&mut self.messages, &*self.provider, &self.system_prompt, context_window).await?;

            let tool_defs: Vec<_> = self.tools.values().map(|t| t.definition()).collect();

            // Use the configured thinking level directly — no dynamic downgrade.
            let level = self.config.resolve_thinking_level();
            let thinking_budget = Some(level_to_budget(level));

            if let Some(cb) = on_thinking_level {
                cb(Some(level));
            }

            // Ensure max_tokens always exceeds thinking_budget + headroom to prevent API hangs.
            let max_tokens = self.config.max_tokens.max(thinking_budget.map(|b| b + 4096).unwrap_or(0));
            let request = MessageRequest {
                model: self.config.model.clone(),
                system: Some(self.system_prompt.clone()),
                messages: self.messages.clone(),
                tools: tool_defs,
                max_tokens,
                temperature: self.config.temperature,
                thinking_budget,
            };

            debug!(
                "Calling LLM API (model: {}, messages: {}, sys_prompt: {} chars, max_tokens: {}, thinking_budget: {:?})",
                self.config.model,
                self.messages.len(),
                self.system_prompt.len(),
                max_tokens,
                thinking_budget,
            );
            let mut stream = match self.call_with_retry(&request, on_text).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("LLM API call failed after retries: {e}");
                    self.finalize_plan().await;
                    return Err(e);
                }
            };

            let mut assistant_text = String::new();
            let mut tool_calls: Vec<ToolCallState> = Vec::new();
            let mut got_api_usage = false;
            // Track whether this turn had any write/execute tool calls.
            // Used to detect research loops where the model keeps reading
            // without making changes.
            let mut had_write_or_execute_this_turn = false;

            // Enforce a 5-minute ceiling on the entire turn, in addition to the 120s
            // per-event timeout.  This catches APIs that keep the connection alive with
            // SSE keepalives but never produce any real content.
            let turn_start = std::time::Instant::now();
            let api_call_start = turn_start;
            let mut first_event_time: Option<std::time::Instant> = None;
            let max_turn_duration = Duration::from_secs(300);

            let stop_reason = loop {
                let elapsed = turn_start.elapsed();
                let remaining = if elapsed > max_turn_duration {
                    Duration::from_secs(0)
                } else {
                    max_turn_duration - elapsed
                };
                let per_event_timeout = Duration::from_secs(120).min(remaining);
                if per_event_timeout.is_zero() {
                    warn!("LLM stream exceeded maximum turn duration of 5 minutes; aborting turn");
                    self.finalize_plan().await;
                    return Ok("Turn timed out — maximum duration exceeded.".to_string());
                }

                let next_event = if let Some(c) = cancel {
                    tokio::select! {
                        event = timeout(per_event_timeout, stream.next()) => event,
                        _ = c.cancelled() => {
                            self.finalize_plan().await;
                            return Ok("Turn cancelled by user.".to_string());
                        }
                    }
                } else {
                    timeout(per_event_timeout, stream.next()).await
                };

                match next_event {
                    Ok(Some(event)) => {
                        if first_event_time.is_none() {
                            first_event_time = Some(std::time::Instant::now());
                            let ttft = api_call_start.elapsed();
                            debug!("LLM first token received after {:?}", ttft);
                        }
                        trace!("Agent received stream event: {:?}", event);
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
                                    tc.name = normalize_tool_name(&name);
                                }
                                tc.arguments.push_str(&arguments_delta);
                            }
                            StreamEvent::Usage(usage) => {
                                got_api_usage = true;
                                // Use the API's prompt_tokens as the authoritative context size.
                                // Don't accumulate input_tokens across turns — each turn's prompt_tokens
                                // already includes all prior messages, so accumulating would double-count.
                                self.current_context_tokens = usage.input_tokens;
                                // For total_usage, track the max context seen (not a sum)
                                self.total_usage.input_tokens = usage.input_tokens;
                                self.total_usage.output_tokens += usage.output_tokens;
                                self.total_usage.cached_tokens += usage.cached_tokens;
                                if let Some(cb) = on_usage {
                                    cb(self.total_usage.input_tokens, self.total_usage.output_tokens, self.current_context_tokens, self.total_usage.cached_tokens);
                                }
                            }
                            StreamEvent::Done { stop_reason } => break stop_reason,
                            StreamEvent::Error(msg) => {
                                self.finalize_plan().await;
                                return Err(RustyError::Api(msg));
                            }
                        }
                    }
                    Ok(None) => {
                        trace!("Agent stream ended naturally");
                        // Stream ended naturally (no more events).
                        break None;
                    }
                    Err(_) => {
                        warn!("LLM stream timed out after {}s with no events; aborting turn", per_event_timeout.as_secs());
                        trace!("Agent stream timed out after {}s", per_event_timeout.as_secs());
                        self.finalize_plan().await;
                        return Ok("Turn timed out — no response from model.".to_string());
                    }
                }
            };

            let turn_duration = turn_start.elapsed();
            let streaming_time = first_event_time.map(|t| t.elapsed());
            debug!(
                "LLM turn complete in {:?} (TTFT: {:?}, streaming: {:?}, stop_reason: {:?}, text_len: {}, tool_calls: {})",
                turn_duration,
                first_event_time.map(|t| t.duration_since(api_call_start)),
                streaming_time,
                stop_reason,
                assistant_text.len(),
                tool_calls.len()
            );

            // Estimate tokens only if the provider didn't report usage
            // (common with OpenAI-compatible providers that don't support stream_options)
            if !got_api_usage {
                let messages_chars: usize = self.messages.iter().map(|m| m.get_all_text().len()).sum();
                let system_chars = self.system_prompt.len();
                let tool_chars: usize = self.tools.values().map(|t| {
                    t.name().len() + t.description().len() + t.input_schema().to_string().len()
                }).sum();
                let estimated_input = ((messages_chars + system_chars + tool_chars) / 4) as u32;
                let estimated_output = (assistant_text.len() / 4) as u32;
                self.current_context_tokens = estimated_input;
                if estimated_input > self.total_usage.input_tokens {
                    self.total_usage.input_tokens = estimated_input;
                }
                if estimated_output > self.total_usage.output_tokens {
                    self.total_usage.output_tokens = estimated_output;
                }
                if let Some(cb) = on_usage {
                    cb(self.total_usage.input_tokens, self.total_usage.output_tokens, self.current_context_tokens, self.total_usage.cached_tokens);
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
                        self.finalize_plan().await;
                        return Ok("Turn cancelled by user.".to_string());
                    }
                }
                for tc in &tool_calls {
                    let input: serde_json::Value = serde_json::from_str(&tc.arguments)
                        .ok()
                        .filter(|v: &serde_json::Value| v.is_object())
                        .unwrap_or(serde_json::json!({}));
                    blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input,
                    });
                }

                // Detect malformed tool calls: empty name/id (can happen with some
                // providers that omit the function name in streaming deltas) or
                // truncated arguments when max_tokens is hit.
                let empty_name: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| tc.name.trim().is_empty() || tc.id.trim().is_empty())
                    .collect();
                if !empty_name.is_empty() {
                    warn!(
                        "Skipping {} tool call(s) with empty name/id — provider sent malformed deltas",
                        empty_name.len()
                    );
                    self.messages.push(Message::user(
                        "One or more tool calls in your previous response were malformed (missing a name or ID). \
                         Please retry the request."
                            .to_string(),
                    ));
                    continue;
                }

                if stop_reason.as_deref() == Some("max_tokens") {
                    let bad: Vec<_> = tool_calls
                        .iter()
                        .filter(|tc| {
                            !tc.arguments.trim().is_empty()
                                && serde_json::from_str::<serde_json::Value>(&tc.arguments)
                                    .ok()
                                    .filter(|v| v.is_object())
                                    .is_none()
                        })
                        .collect();
                    if !bad.is_empty() {
                        warn!(
                            "Skipping {} tool call(s) with truncated arguments due to max_tokens",
                            bad.len()
                        );
                        self.messages.push(Message::user(
                            "Your previous response was truncated because it exceeded the output token limit. \
                             The tool call(s) you started had incomplete arguments and could not be executed. \
                             Please retry with a smaller request — for example, write large files in smaller \
                             chunks instead of all at once."
                                .to_string(),
                        ));
                        continue;
                    }
                }

                self.messages.push(Message::assistant_blocks(blocks.clone()));

                // Execute tools concurrently.
                // Permission checks happen first (sequential, may prompt user),
                // then all approved tools run in parallel via tokio::spawn.
                let ctx = ToolContext {
                    working_dir: self.working_dir.clone(),
                    permission_mode: self.permission_mode,
                    cancel: callbacks.cancel.cloned(),
                };

                // Phase 1: Check permissions for all tool calls (sequential)
                let mut permission_results: Vec<(&ToolCallState, PermissionDecision)> = Vec::new();
                for tc in &tool_calls {
                    let decision = self.check_permission_for_tool(tc, &ctx).await;
                    match &decision {
                        PermissionDecision::AllowSession => {
                            let key = make_allow_key(&tc.name, &tc.arguments);
                            self.session_allowlist.insert(key);
                        }
                        PermissionDecision::AllowAlways => {
                            let key = make_allow_key(&tc.name, &tc.arguments);
                            self.session_allowlist.insert(key.clone());
                            if let Err(e) = rusty_core::config::add_permanent_permission(&key).await {
                                warn!("Failed to save permanent permission: {e}");
                            }
                        }
                        _ => {}
                    }
                    permission_results.push((tc, decision));
                }

                // Phase 2: Fire "Running" callbacks and spawn approved tools concurrently
                let mut spawn_handles: Vec<Option<_>> = Vec::new();
                for (tc, decision) in &permission_results {
                    if let PermissionDecision::Deny(reason) = decision {
                        warn!("Tool {} denied: {}", tc.name, reason);
                        spawn_handles.push(None);
                        continue;
                    }

                    // Track write/execute usage for research-loop detection
                    if let Some(tool) = self.tools.get(&tc.name) {
                        let level = tool.permission_level();
                        if level == PermissionLevel::Write || level == PermissionLevel::Execute {
                            had_write_or_execute_this_turn = true;
                        }
                    }

                    if let Some(cb) = on_tool {
                        cb(&tc.name, ToolStatus::Running {
                            arguments: tc.arguments.clone(),
                        });
                    }

                    // Clone what we need for the spawned task
                    let tool_name = tc.name.clone();
                    let tool_args = tc.arguments.clone();
                    let tool_arc = self.tools.get(&tc.name).cloned();
                    let tool_ctx = ctx.clone();

                    let handle = tokio::spawn(async move {
                        let Some(tool) = tool_arc else {
                            return Err(RustyError::Tool(format!("Unknown tool: {tool_name}")));
                        };
                        let input: serde_json::Value = serde_json::from_str(&tool_args)
                            .ok()
                            .filter(|v: &serde_json::Value| v.is_object())
                            .unwrap_or(serde_json::json!({}));
                        tool.execute(input, &tool_ctx).await
                    });
                    spawn_handles.push(Some(handle));
                }

                // Phase 3: Collect results and fire callbacks
                let results: Vec<Option<Result<ToolResult, RustyError>>> = join_all(
                    spawn_handles.into_iter().map(|h| async {
                        match h {
                            Some(handle) => match handle.await {
                                Ok(r) => Some(r),
                                Err(e) => Some(Err(RustyError::Other(format!("Tool task panicked: {e}")))),
                            },
                            None => None,
                        }
                    })
                ).await;

                for ((tc, _decision), result) in permission_results.iter().zip(results) {
                    let tool_result = match result {
                        Some(Ok(r)) => r,
                        Some(Err(e)) => {
                            debug!("Tool {} failed: {}", tc.name, e);
                            ToolResult::error(e.to_string())
                        }
                        None => {
                            // Permission denied — already logged
                            if let Some(cb) = on_tool {
                                cb(&tc.name, ToolStatus::Error {
                                    output: format!("Permission denied"),
                                });
                            }
                            self.messages.push(Message::user_blocks(vec![
                                ContentBlock::ToolResult {
                                    tool_use_id: tc.id.clone(),
                                    content: format!("Permission denied"),
                                    is_error: Some(true),
                                },
                            ]));
                            continue;
                        }
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

                    // Update plan mode state based on special plan tools
                    if tc.name == "enter_plan_mode" && !tool_result.is_error {
                        self.plan_mode = true;
                        debug!("Entered explicit plan mode");
                    }
                    if tc.name == "exit_plan_mode" && !tool_result.is_error {
                        self.plan_mode = false;
                        self.exited_plan_mode_this_turn = true;
                        debug!("Exited explicit plan mode");
                    }
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
                    let incomplete = self.incomplete_task_details();
                    let nudge_limit = (incomplete.len() as u32 * 4).max(12);

                    if had_write_or_execute_this_turn {
                        self.consecutive_read_turns = 0;
                    } else {
                        self.consecutive_read_turns += 1;
                    }

                    if !incomplete.is_empty()
                        && self.task_nudge_count < nudge_limit
                        && !self.exited_plan_mode_this_turn
                    {
                        warn!(
                            "Model tried to stop with {} incomplete tasks (nudge {}/{}); nudging to continue",
                            incomplete.len(),
                            self.task_nudge_count + 1,
                            nudge_limit
                        );
                        self.task_nudge_count += 1;
                        let task_list: String = incomplete
                            .iter()
                            .enumerate()
                            .map(|(i, (status, content))| format!("  {}. [{}] {}", i + 1, status, content))
                            .collect::<Vec<_>>()
                            .join("\n");

                        let nudge_msg = if self.consecutive_read_turns >= 3 {
                            format!(
                                "You have {} incomplete task(s) remaining:\n{}\n\n\
                                 You have spent {} turns researching without making any changes. \
                                 STOP reading files and START implementing. Pick the FIRST pending task \
                                 and call the appropriate tool to make the change NOW. Do NOT do any \
                                 more research — execute immediately.",
                                incomplete.len(),
                                task_list,
                                self.consecutive_read_turns,
                            )
                        } else {
                            format!(
                                "You have {} incomplete task(s) remaining:\n{}\n\n\
                                 Continue working through them. Execute the next pending task by calling \
                                 the appropriate tool. When you finish a task, update its status via todowrite \
                                 in your next turn.",
                                incomplete.len(),
                                task_list,
                            )
                        };
                        self.messages.push(Message::user(nudge_msg));
                        continue;
                    }
                    self.finalize_plan().await;
                    return Ok(assistant_text);
                }
                Some("max_tokens") => {
                    warn!("Hit max_tokens limit");
                    let warning = format!(
                        "{}\n\n[Response truncated: hit max_tokens limit. Consider using /compact if context is full.]",
                        assistant_text
                    );
                    self.finalize_plan().await;
                    return Ok(warning);
                }
                Some(other) => {
                    debug!("Unexpected stop reason: {other}");
                    self.finalize_plan().await;
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
            Err(e) => {
                self.finalize_plan().await;
                return Err(e);
            }
        };

        let mut summary = String::new();
        loop {
            let next_event = if let Some(c) = cancel {
                tokio::select! {
                    event = stream.next() => event,
                    _ = c.cancelled() => {
                        self.finalize_plan().await;
                        return Ok("Turn cancelled by user.".to_string());
                    }
                }
            } else {
                stream.next().await
            };
            let Some(event) = next_event else { break };
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
        self.finalize_plan().await;
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

    /// Check permissions for a tool call without executing it.
    /// Returns the permission decision. Used by the concurrent execution path
    /// where permission checks happen before spawning tasks.
    async fn check_permission_for_tool(
        &self,
        tc: &ToolCallState,
        _ctx: &ToolContext,
    ) -> PermissionDecision {
        let normalized = normalize_tool_name(&tc.name);
        let effective_level = if normalized == "bash" {
            let input: serde_json::Value = serde_json::from_str(&tc.arguments)
                .unwrap_or(serde_json::Value::Null);
            let cmd = input["command"].as_str().unwrap_or("");
            match classify_bash_command(cmd) {
                BashClassification::ReadOnly => PermissionLevel::ReadOnly,
                BashClassification::Write => PermissionLevel::Execute,
                BashClassification::Execute => PermissionLevel::Execute,
            }
        } else {
            self.tools
                .get(&normalized)
                .map(|t| t.permission_level())
                .unwrap_or(PermissionLevel::Execute)
        };

        self.check_permission_tiered(&tc.name, &tc.arguments, effective_level)
            .await
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

        // 2. CLI Plan mode — deny write/execute
        if self.permission_mode == PermissionMode::Plan {
            if effective_level == PermissionLevel::ReadOnly
                || effective_level == PermissionLevel::None
            {
                return PermissionDecision::AllowOnce;
            }
            return PermissionDecision::Deny("Plan mode is read-only".into());
        }

        // 2.5 Explicit plan mode — deny write/execute (but allow exit_plan_mode)
        if self.plan_mode
            && effective_level != PermissionLevel::ReadOnly
            && effective_level != PermissionLevel::None
            && tool_name != "exit_plan_mode"
        {
            return PermissionDecision::Deny(
                "You are in plan mode. Write and execute tools are disabled. \
                 Use exit_plan_mode when you are ready to execute the plan."
                    .into(),
            );
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

        // 4.5 Protected files — always require explicit approval for sensitive paths
        if effective_level == PermissionLevel::Write {
            if let Some(path) = extract_path_from_tool_args(tool_name, arguments) {
                if rusty_core::permissions::is_protected_path(&path) {
                    return PermissionDecision::Deny(format!(
                        "{} targets a protected file/directory ({}). Explicit approval required.",
                        tool_name, path
                    ));
                }
            }
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

/// Extract the target file path from tool arguments, if applicable.
fn extract_path_from_tool_args(tool_name: &str, arguments: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    match tool_name {
        "file_read" | "file_write" | "file_edit" => {
            v["path"].as_str().or_else(|| v["file_path"].as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Normalize a tool name from the model to match our registered tools.
/// Handles case differences and common aliases (e.g. Claude-style `read` → `file_read`).
fn normalize_tool_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        "read" => "file_read".to_string(),
        "write" => "file_write".to_string(),
        "edit" => "file_edit".to_string(),
        _ => lower,
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
    fn truncate_mid_utf8_char_does_not_panic() {
        // Each CJK character is 3 bytes in UTF-8.
        // max_chars=5 lands in the middle of the second char (bytes 3..5).
        // Before the fix this would panic with "byte index N is not a char boundary".
        let text = "你好世界test";
        let result = smart_truncate_output(text, 5);
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_emoji_mid_char() {
        // Emoji are 4 bytes. max_chars=6 lands mid-emoji (bytes 4..6).
        let text = "hi😀😀😀end";
        let result = smart_truncate_output(text, 6);
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_empty_text() {
        let result = smart_truncate_output("", 100);
        assert_eq!(result, "");
    }
}

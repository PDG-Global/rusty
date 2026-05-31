// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures::StreamExt;
use rusty_core::permissions::{
    classify_bash_command, make_allow_key, BashClassification, PermissionDecision,
    PermissionLevel, PermissionRequest,
};
use rusty_core::{Config, ContentBlock, Message, PermissionMode, RustyError, UsageInfo};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use rusty_tools::{Tool, ToolContext, ToolResult};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::compact::maybe_compact;

/// Callback for streaming text deltas to the UI
pub type TextCallback = Box<dyn Fn(&str) + Send + Sync>;
/// Callback for streaming thinking/reasoning deltas
pub type ThinkingCallback = Box<dyn Fn(&str) + Send + Sync>;
/// Callback for tool execution status
pub type ToolCallback = Box<dyn Fn(&str, &str) + Send + Sync>;
/// Callback for token usage updates
pub type UsageCallback = Box<dyn Fn(u32, u32) + Send + Sync>;
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
    total_usage: UsageInfo,
    permission_mode: PermissionMode,
    max_turns: u32,
    permission_callback: Option<PermissionCallback>,
    session_allowlist: HashSet<String>,
    permanent_allowlist: HashSet<String>,
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

    pub fn total_usage(&self) -> &UsageInfo {
        &self.total_usage
    }

    /// Run the agent loop: send messages, handle streaming, execute tools, repeat.
    pub async fn run(
        &mut self,
        user_input: &str,
        on_text: Option<&TextCallback>,
        on_thinking: Option<&ThinkingCallback>,
        on_tool: Option<&ToolCallback>,
        on_usage: Option<&UsageCallback>,
    ) -> Result<String, RustyError> {
        self.messages.push(Message::user(user_input));

        for turn in 0..self.max_turns {
            debug!("Agent turn {}/{}", turn + 1, self.max_turns);

            // Maybe compact before sending
            maybe_compact(&mut self.messages, &*self.provider, &self.system_prompt).await?;

            let tool_defs: Vec<_> = self.tools.values().map(|t| t.definition()).collect();

            let request = MessageRequest {
                model: self.config.model.clone(),
                system: Some(self.system_prompt.clone()),
                messages: self.messages.clone(),
                tools: tool_defs,
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
            };

            debug!("Calling LLM API (model: {}, messages: {})", self.config.model, self.messages.len());
            let mut stream = match self.provider.create_message_stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("LLM API call failed: {e}");
                    return Err(e);
                }
            };

            let mut assistant_text = String::new();
            let mut tool_calls: Vec<ToolCallState> = Vec::new();
            let mut stop_reason = None;

            while let Some(event) = stream.next().await {
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

            // Build assistant message
            let mut blocks = Vec::new();
            if !assistant_text.is_empty() {
                blocks.push(ContentBlock::Text {
                    text: assistant_text.clone(),
                });
            }

            // If there are tool calls, execute them
            if !tool_calls.is_empty() {
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
                        cb(&tc.name, "running");
                    }

                    let result = self.execute_tool(&tc.name, &tc.arguments, &ctx).await;

                    let tool_result = match result {
                        Ok(r) => r,
                        Err(e) => ToolResult::error(e.to_string()),
                    };

                    if let Some(cb) = on_tool {
                        cb(&tc.name, if tool_result.is_error { "error" } else { "done" });
                    }

                    self.messages.push(Message::user_blocks(vec![
                        ContentBlock::ToolResult {
                            tool_use_id: tc.id.clone(),
                            content: tool_result.content,
                            is_error: Some(tool_result.is_error),
                        },
                    ]));
                }

                // Pause between tool rounds to avoid rate limiting
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
                    return Ok(assistant_text);
                }
                Some(other) => {
                    debug!("Unexpected stop reason: {other}");
                    return Ok(assistant_text);
                }
            }
        }

        Err(RustyError::Other(
            "Max turns exceeded without completion".into(),
        ))
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

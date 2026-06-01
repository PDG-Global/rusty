// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

pub mod compact;
pub mod r#loop;

use rusty_core::{Config, PermissionMode, RustyError};
use rusty_provider::LlmProvider;
use rusty_tools::Tool;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use r#loop::{Agent, CancelToken, PermissionCallback, ToolStatus};

/// Spawn a sub-agent as a same-process tokio task.
/// Returns the sub-agent's final text response.
/// Sub-agents run with BypassPermissions — no interactive prompts.
pub async fn spawn_subagent(
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn Tool>>,
    config: Config,
    working_dir: PathBuf,
    system_prompt: String,
    task: String,
) -> Result<String, RustyError> {
    let handle = tokio::spawn(async move {
        let mut agent = Agent::new(provider, tools, config, working_dir, system_prompt);
        agent.set_permission_mode(PermissionMode::BypassPermissions);
        agent.run(&task, None, None, None, None, None, None).await
    });

    handle
        .await
        .map_err(|e| RustyError::Other(format!("Sub-agent panicked: {e}")))?
}

/// Create an AgentTool wired to spawn sub-agents with the given provider and config.
/// Sub-agents receive a fresh set of tools (same as the main agent, minus the agent tool itself).
pub fn make_agent_tool(
    provider: Arc<dyn LlmProvider>,
    system_prompt: String,
    config: Config,
) -> rusty_tools::agent::AgentTool {
    let spawn_fn = Arc::new(move |task: String, working_dir: PathBuf| {
        let provider = provider.clone();
        let system_prompt = system_prompt.clone();
        let config = config.clone();
        Box::pin(async move {
            // Sub-agents get all tools except the agent tool (to prevent recursive spawning)
            let tools: Vec<Box<dyn rusty_tools::Tool>> = rusty_tools::all_tools();
            spawn_subagent(provider, tools, config, working_dir, system_prompt, task).await
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, RustyError>> + Send>>
    });

    rusty_tools::agent::AgentTool { spawn_fn }
}

/// Build the system prompt from config and context
pub async fn build_system_prompt(config: &Config, working_dir: &Path) -> String {
    let mut parts = Vec::new();

    // Base system prompt
    let base = config
        .system_prompt
        .as_deref()
        .unwrap_or("You are a helpful AI coding assistant. Be concise and precise.");
    parts.push(base.to_string());

    // Plan-with-tasks mode: instruct model to actively use todowrite
    if config.plan_with_tasks {
        parts.push(
            "## Task Tracking Mode\n\n\
            You are operating in plan-with-tasks mode. You must actively use the `todowrite` \
            tool to plan and track your work. At the start of each task:\n\
            1. Break the user's request into concrete sub-tasks using `todowrite` with status `pending`.\n\
            2. As you work through each sub-task, update its status to `in_progress` before starting \
            and `completed` when done.\n\
            3. If you discover additional work needed, add new tasks to the list.\n\
            4. Keep the task list visible and up-to-date throughout the conversation.\n\n\
            You are in read-only mode and cannot make file edits or run commands that modify the system. \
            Use your task list to present a clear, actionable plan to the user."
                .to_string(),
        );
    }

    // System context (platform, git, etc.)
    let sys_ctx = rusty_core::context::build_system_context(working_dir).await;
    if !sys_ctx.is_empty() {
        parts.push(sys_ctx);
    }

    // User context (CLAUDE.md, date, etc.)
    let user_ctx =
        rusty_core::context::build_user_context(working_dir, config.no_claude_md).await;
    if !user_ctx.is_empty() {
        parts.push(user_ctx);
    }

    // Append system prompt if configured
    if let Some(append) = &config.append_system_prompt {
        parts.push(append.clone());
    }

    parts.join("\n\n")
}

// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures::StreamExt;
use rusty_core::{Message, Role, RustyError};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use std::path::Path;
use tracing::{debug, info};

// ── Tier thresholds (fraction of context window) ────────────────────────────

/// Tier 1: micro-compaction — replace old tool results with placeholders.
const TIER1_FRACTION: f64 = 0.25;

/// Tier 2: structured extraction — LLM writes checkpoint.md.
const TIER2_FRACTION: f64 = 0.50;

/// Tier 3: full compaction — LLM summarises old messages.
const TIER3_FRACTION: f64 = 0.75;

/// Keep the last N messages un-compacted.
const KEEP_RECENT: usize = 10;

/// Fallback message count threshold for full compaction.
const COMPACT_MESSAGE_THRESHOLD: usize = 40;

/// Tool results longer than this are replaced with a placeholder during micro-compaction.
const MICRO_COMPACT_THRESHOLD_CHARS: usize = 500;

// ── Checkpoint tier tracking ────────────────────────────────────────────────

/// Tracks which checkpoint tiers have fired in the current context growth cycle.
/// After tier 3 (full compaction) shrinks the context, the tier resets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointTier {
    None = 0,
    Micro = 1,
    Extracted = 2,
    Compacted = 3,
}

impl CheckpointTier {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Micro,
            2 => Self::Extracted,
            _ => Self::Compacted,
        }
    }
}

// ── Tier 1: Micro-compaction ────────────────────────────────────────────────

/// Lightweight compaction: replace old tool results with placeholders.
/// Returns true if any replacements were made.
pub fn micro_compact(messages: &mut Vec<Message>) -> bool {
    let split_point = messages.len().saturating_sub(KEEP_RECENT);
    let mut modified = false;

    for msg in messages.iter_mut().take(split_point) {
        if let rusty_core::MessageContent::Blocks(ref mut blocks) = msg.content {
            for block in blocks.iter_mut() {
                if let rusty_core::ContentBlock::ToolResult { content, .. } = block {
                    if content.len() > MICRO_COMPACT_THRESHOLD_CHARS {
                        let lines = content.lines().count();
                        *content = format!(
                            "[Old tool result content cleared ({lines} lines)]"
                        );
                        modified = true;
                    }
                }
            }
        }
    }

    if modified {
        debug!("Micro-compacted tool results in {} oldest messages", split_point);
    }
    modified
}

// ── Tier 2: Structured extraction ───────────────────────────────────────────

const CHECKPOINT_PROMPT: &str = "\
You are extracting structured state from a conversation for future reference. \
Read the conversation below and produce a checkpoint with exactly these sections:

## Intent
What the user asked for and what the agent is trying to accomplish. 1-3 sentences.

## Key Decisions
Technical decisions made and their rationale. One line per decision.

## Files Changed
Files modified or created, with a one-line description each.

## Current State
What is done, what is in progress, what is next. Be specific.

## Notes
Anything else worth preserving: errors encountered, gotchas, open questions.

Rules:
- Be concise. Each section should be 1-5 lines.
- Focus on facts, not narration.
- Include file paths and specific technical details.
- Capture what the agent should know when it wakes up in a fresh context.";

/// Extract structured checkpoint from conversation history using an LLM call.
/// Writes the result to `checkpoint_path`. If `notes_content` is provided,
/// it is included in the extraction prompt and should be cleared afterward.
pub async fn extract_checkpoint(
    messages: &[Message],
    provider: &dyn LlmProvider,
    system_prompt: &str,
    checkpoint_path: &Path,
    notes_content: Option<&str>,
) -> Result<(), RustyError> {
    let old_text = messages_to_text(messages);

    let mut prompt = String::from(CHECKPOINT_PROMPT);
    if let Some(notes) = notes_content {
        if !notes.trim().is_empty() {
            prompt.push_str("\n\nScratchpad notes from the session:\n");
            prompt.push_str(notes);
        }
    }
    prompt.push_str("\n\nConversation:\n");
    prompt.push_str(&old_text);

    let request = MessageRequest {
        model: provider.model().to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(&prompt)],
        tools: vec![],
        max_tokens: 1024,
        temperature: None,
        thinking_budget: None,
    };

    let mut stream = provider.create_message_stream(request).await?;
    let mut checkpoint = String::new();

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(text) => checkpoint.push_str(&text),
            StreamEvent::Done { .. } => break,
            StreamEvent::Error(msg) => return Err(RustyError::Api(msg)),
            _ => {}
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = checkpoint_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    tokio::fs::write(checkpoint_path, &checkpoint).await.map_err(|e| {
        RustyError::Other(format!("Failed to write checkpoint: {e}"))
    })?;

    info!("Extracted checkpoint ({} chars)", checkpoint.len());
    Ok(())
}

// ── Tier 3: Full compaction ─────────────────────────────────────────────────

/// Compute the token threshold for a given tier fraction.
fn tier_threshold(context_window: u32, fraction: f64) -> usize {
    (context_window as f64 * fraction) as usize
}

/// Check if compaction is needed and perform it.
/// Handles the three-tier checkpoint system:
/// - Tier 1 (25%): micro-compaction (cheap placeholder replacement)
/// - Tier 2 (50%): structured extraction to checkpoint.md
/// - Tier 3 (75%): full LLM-based compaction
///
/// Returns the highest tier that was executed.
pub async fn maybe_compact(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    system_prompt: &str,
    context_window: u32,
    plan_text: Option<&str>,
    last_tier: CheckpointTier,
    notes_path: Option<&Path>,
    checkpoint_path: Option<&Path>,
) -> Result<CheckpointTier, RustyError> {
    let estimated_tokens = estimate_tokens(messages);
    let t1_threshold = tier_threshold(context_window, TIER1_FRACTION);
    let t2_threshold = tier_threshold(context_window, TIER2_FRACTION);
    let t3_threshold = tier_threshold(context_window, TIER3_FRACTION);

    let needs_t3 = messages.len() >= COMPACT_MESSAGE_THRESHOLD
        || estimated_tokens >= t3_threshold;

    // ── Tier 3: Full compaction ───────────────────────────────────────
    if needs_t3 && last_tier.as_u8() < CheckpointTier::Compacted.as_u8() {
        info!(
            "Tier 3 compaction: {} messages (~{} tokens, threshold {})",
            messages.len(), estimated_tokens, t3_threshold
        );

        // Read checkpoint.md if available to enrich the summary
        let checkpoint_context = if let Some(cp_path) = checkpoint_path {
            read_file_content(cp_path).await
        } else {
            None
        };

        // Also read any remaining notes
        let notes_context = if let Some(n_path) = notes_path {
            let content = read_file_content(n_path).await;
            // Clear notes after reading
            if content.is_some() {
                clear_file(n_path).await;
            }
            content
        } else {
            None
        };

        let split_point = messages.len().saturating_sub(KEEP_RECENT);
        let old_messages = &messages[..split_point];
        let recent_messages = &messages[split_point..];

        let old_text = messages_to_text(old_messages);

        let mut summary_prompt = String::new();
        if let Some(cp) = &checkpoint_context {
            if !cp.trim().is_empty() {
                summary_prompt.push_str("Previous checkpoint state:\n");
                summary_prompt.push_str(cp);
                summary_prompt.push_str("\n\n");
            }
        }
        if let Some(notes) = &notes_context {
            if !notes.trim().is_empty() {
                summary_prompt.push_str("Session notes:\n");
                summary_prompt.push_str(notes);
                summary_prompt.push_str("\n\n");
            }
        }
        summary_prompt.push_str("Summarize the following conversation concisely, preserving key context, decisions, and any code changes discussed:\n\n");
        summary_prompt.push_str(&old_text);

        let request = MessageRequest {
            model: provider.model().to_string(),
            system: Some(system_prompt.to_string()),
            messages: vec![Message::user(&summary_prompt)],
            tools: vec![],
            max_tokens: 2048,
            temperature: None,
            thinking_budget: None,
        };

        let mut stream = provider.create_message_stream(request).await?;
        let mut summary = String::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::TextDelta(text) => summary.push_str(&text),
                StreamEvent::Done { .. } => break,
                StreamEvent::Error(msg) => return Err(RustyError::Api(msg)),
                _ => {}
            }
        }

        debug!("Compacted {} messages into summary", split_point);

        let summary_with_plan = if let Some(plan) = plan_text {
            format!("{summary}\n\n{plan}")
        } else {
            summary
        };
        let mut new_messages = Vec::new();
        new_messages.push(Message::user(format!(
            "[Previous conversation summary]\n{summary_with_plan}"
        )));
        new_messages.push(Message::assistant(
            "Understood. I have the context from our previous conversation.",
        ));
        new_messages.extend_from_slice(recent_messages);

        *messages = new_messages;

        let new_tokens = estimate_tokens(messages);
        info!(
            "After compaction: {} messages (~{} tokens)",
            messages.len(),
            new_tokens
        );

        return Ok(CheckpointTier::Compacted);
    }

    // ── Tier 2: Structured extraction ─────────────────────────────────
    if estimated_tokens >= t2_threshold && last_tier.as_u8() < CheckpointTier::Extracted.as_u8() {
        info!(
            "Tier 2 extraction: ~{} tokens (threshold {})",
            estimated_tokens, t2_threshold
        );

        // Read and clear notes
        let notes_content = if let Some(n_path) = notes_path {
            let content = read_file_content(n_path).await;
            if content.is_some() {
                clear_file(n_path).await;
            }
            content
        } else {
            None
        };

        if let Some(cp_path) = checkpoint_path {
            extract_checkpoint(
                messages,
                provider,
                system_prompt,
                cp_path,
                notes_content.as_deref(),
            )
            .await?;
        }

        // Also run micro-compaction at this tier
        micro_compact(messages);

        return Ok(CheckpointTier::Extracted);
    }

    // ── Tier 1: Micro-compaction ──────────────────────────────────────
    if estimated_tokens >= t1_threshold && last_tier.as_u8() < CheckpointTier::Micro.as_u8() {
        info!(
            "Tier 1 micro-compaction: ~{} tokens (threshold {})",
            estimated_tokens, t1_threshold
        );
        if micro_compact(messages) {
            return Ok(CheckpointTier::Micro);
        }
    }

    Ok(last_tier)
}

// ── Force compaction (used by /compact command) ─────────────────────────────

/// Force compaction regardless of thresholds — used for /compact command.
/// If `plan_text` is provided, it is appended to the summary.
pub async fn force_compact(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    system_prompt: &str,
    plan_text: Option<&str>,
    notes_path: Option<&Path>,
    checkpoint_path: Option<&Path>,
) -> Result<bool, RustyError> {
    if messages.len() < 4 {
        return Ok(false);
    }

    info!("Force compacting: {} messages", messages.len());

    // Read checkpoint and notes for context
    let checkpoint_context = if let Some(cp_path) = checkpoint_path {
        read_file_content(cp_path).await
    } else {
        None
    };
    let notes_context = if let Some(n_path) = notes_path {
        let content = read_file_content(n_path).await;
        if content.is_some() {
            clear_file(n_path).await;
        }
        content
    } else {
        None
    };

    let split_point = messages.len().saturating_sub(KEEP_RECENT);
    let old_messages = &messages[..split_point];
    let recent_messages = &messages[split_point..];

    let old_text = messages_to_text(old_messages);

    let mut summary_prompt = String::new();
    if let Some(cp) = &checkpoint_context {
        if !cp.trim().is_empty() {
            summary_prompt.push_str("Previous checkpoint state:\n");
            summary_prompt.push_str(cp);
            summary_prompt.push_str("\n\n");
        }
    }
    if let Some(notes) = &notes_context {
        if !notes.trim().is_empty() {
            summary_prompt.push_str("Session notes:\n");
            summary_prompt.push_str(notes);
            summary_prompt.push_str("\n\n");
        }
    }
    summary_prompt.push_str("Summarize the following conversation concisely, preserving key context, decisions, and any code changes discussed:\n\n");
    summary_prompt.push_str(&old_text);

    let request = MessageRequest {
        model: provider.model().to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(&summary_prompt)],
        tools: vec![],
        max_tokens: 2048,
        temperature: None,
        thinking_budget: None,
    };

    let mut stream = provider.create_message_stream(request).await?;
    let mut summary = String::new();

    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(text) => summary.push_str(&text),
            StreamEvent::Done { .. } => break,
            StreamEvent::Error(msg) => return Err(RustyError::Api(msg)),
            _ => {}
        }
    }

    debug!("Force compacted {} messages into summary", split_point);

    let summary_with_plan = if let Some(plan) = plan_text {
        format!("{summary}\n\n{plan}")
    } else {
        summary
    };
    let mut new_messages = Vec::new();
    new_messages.push(Message::user(format!(
        "[Previous conversation summary]\n{summary_with_plan}"
    )));
    new_messages.push(Message::assistant(
        "Understood. I have the context from our previous conversation.",
    ));
    new_messages.extend_from_slice(recent_messages);

    *messages = new_messages;
    Ok(true)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Read file content, returning None if the file doesn't exist or is empty.
async fn read_file_content(path: &Path) -> Option<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) if !content.trim().is_empty() => Some(content),
        _ => None,
    }
}

/// Truncate a file to zero length (clear it).
async fn clear_file(path: &Path) {
    let _ = tokio::fs::write(path, "").await;
}

/// Estimate token count for messages.
/// Counts ALL content blocks (text, tool results, tool use, thinking) rather
/// than just text, so compaction triggers when context is actually full.
pub fn estimate_tokens(messages: &[Message]) -> usize {
    let mut total_chars: usize = 0;
    for msg in messages {
        total_chars += 16; // per-message overhead (~4 tokens)
        match &msg.content {
            rusty_core::MessageContent::Text(text) => {
                total_chars += text.len();
            }
            rusty_core::MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        rusty_core::ContentBlock::Text { text } => {
                            total_chars += text.len();
                        }
                        rusty_core::ContentBlock::Thinking { thinking } => {
                            total_chars += thinking.len();
                        }
                        rusty_core::ContentBlock::ToolUse { id, name, input } => {
                            total_chars += id.len() + name.len();
                            total_chars += input.to_string().len();
                            total_chars += 32;
                        }
                        rusty_core::ContentBlock::ToolResult { content, tool_use_id, .. } => {
                            total_chars += content.len() + tool_use_id.len() + 32;
                        }
                        rusty_core::ContentBlock::Image { .. } => {
                            total_chars += 256;
                        }
                    }
                }
            }
        }
    }
    ((total_chars as f64 * 1.2 / 4.0).ceil() as usize).max(1)
}

fn messages_to_text(messages: &[Message]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let text = msg.get_all_text();
        if !text.is_empty() {
            parts.push(format!("{role}: {text}"));
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── estimate_tokens ──────────────────────────────────────────────

    #[test]
    fn estimate_empty_messages() {
        let msgs: Vec<Message> = vec![];
        assert_eq!(estimate_tokens(&msgs), 1); // max(1) floor
    }

    #[test]
    fn estimate_single_message() {
        let msgs = vec![Message::user("hello")]; // 5 chars + 16 overhead = 21
        let tokens = estimate_tokens(&msgs);
        assert_eq!(tokens, 7); // ceil(21 * 1.2 / 4) = 7
    }

    #[test]
    fn estimate_scales_with_content() {
        let short = vec![Message::user("hi")];
        let long = vec![Message::user("a".repeat(1000))];
        assert!(estimate_tokens(&long) > estimate_tokens(&short));
    }

    #[test]
    fn estimate_400_chars_is_125_tokens() {
        let msgs = vec![Message::user("a".repeat(400))]; // 400 + 16 = 416
        assert_eq!(estimate_tokens(&msgs), 125); // ceil(416 * 1.2 / 4) = 125
    }

    // ── messages_to_text ─────────────────────────────────────────────

    #[test]
    fn messages_to_text_basic() {
        let msgs = vec![Message::user("hello"), Message::assistant("hi there")];
        let text = messages_to_text(&msgs);
        assert_eq!(text, "User: hello\n\nAssistant: hi there");
    }

    #[test]
    fn messages_to_text_skips_empty() {
        let msgs = vec![
            Message::user("hello"),
            Message::user(""),    // empty — skipped
            Message::assistant("ok"),
        ];
        let text = messages_to_text(&msgs);
        assert_eq!(text, "User: hello\n\nAssistant: ok");
    }

    #[test]
    fn messages_to_text_empty_list() {
        let msgs: Vec<Message> = vec![];
        assert_eq!(messages_to_text(&msgs), "");
    }

    // ── micro_compact ────────────────────────────────────────────────

    #[test]
    fn micro_compact_replaces_long_tool_results() {
        use rusty_core::ContentBlock;

        // Need > KEEP_RECENT messages so the first one is in the old group
        let mut msgs: Vec<Message> = (0..11)
            .map(|i| {
                if i == 0 {
                    Message::user_blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "1".into(),
                        content: "a".repeat(1000),
                        is_error: Some(false),
                    }])
                } else {
                    Message::assistant("ok")
                }
            })
            .collect();

        let modified = micro_compact(&mut msgs);
        assert!(modified);
        let block_text = match &msgs[0].content {
            rusty_core::MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!("expected ToolResult"),
            },
            _ => panic!("expected Blocks"),
        };
        assert!(block_text.contains("Old tool result content cleared"));
    }

    #[test]
    fn micro_compact_skips_short_tool_results() {
        use rusty_core::ContentBlock;

        let mut msgs = vec![
            Message::user_blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "1".into(),
                content: "short".into(),
                is_error: Some(false),
            }]),
            Message::assistant("ok"),
        ];

        let modified = micro_compact(&mut msgs);
        assert!(!modified);
    }

    #[test]
    fn micro_compact_preserves_recent_messages() {
        use rusty_core::ContentBlock;

        // All messages are in the recent window
        let mut msgs: Vec<Message> = (0..5)
            .map(|_| {
                Message::user_blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "1".into(),
                    content: "a".repeat(1000),
                    is_error: Some(false),
                }])
            })
            .collect();

        let modified = micro_compact(&mut msgs);
        assert!(!modified); // all in KEEP_RECENT window
    }

    // ── CheckpointTier ───────────────────────────────────────────────

    #[test]
    fn checkpoint_tier_roundtrip() {
        for v in 0..=3 {
            assert_eq!(CheckpointTier::from_u8(v).as_u8(), v);
        }
        assert_eq!(CheckpointTier::from_u8(99).as_u8(), 3); // saturates
    }
}

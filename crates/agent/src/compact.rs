// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures::StreamExt;
use rusty_core::{Message, Role, RustyError};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use tracing::{debug, info};

/// Fraction of context window at which to trigger auto-compaction
const COMPACT_THRESHOLD_FRACTION: f64 = 0.75;
/// Keep the last N messages un-compacted
const KEEP_RECENT: usize = 10;
/// Fallback message count threshold
const COMPACT_MESSAGE_THRESHOLD: usize = 40;
/// Tool results longer than this are replaced with a placeholder during micro-compaction
const MICRO_COMPACT_THRESHOLD_CHARS: usize = 500;

/// Lightweight compaction: replace old tool results with placeholders.
/// This is much cheaper than an LLM summarization pass.
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

/// Compute the token threshold for compaction based on the model's context window.
fn compact_token_threshold(context_window: u32) -> usize {
    (context_window as f64 * COMPACT_THRESHOLD_FRACTION) as usize
}

/// Check if compaction is needed and perform it.
/// Tries micro-compaction first (cheap placeholder replacement), then falls
/// back to LLM-based full compaction if still over the threshold.
/// If `plan_text` is provided, it is appended to the summary so the todo list
/// is not lost during compaction.
/// Returns `true` if full compaction was performed, `false` if no compaction
/// or only micro-compaction was needed.
pub async fn maybe_compact(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    system_prompt: &str,
    context_window: u32,
    plan_text: Option<&str>,
) -> Result<bool, RustyError> {
    let estimated_tokens = estimate_tokens(messages);
    let token_threshold = compact_token_threshold(context_window);

    let needs_compact = messages.len() >= COMPACT_MESSAGE_THRESHOLD
        || estimated_tokens >= token_threshold;

    if !needs_compact {
        return Ok(false);
    }

    info!(
        "Auto-compacting: {} messages (~{} tokens)",
        messages.len(),
        estimated_tokens
    );

    // Phase 1: micro-compaction — replace old tool results with placeholders.
    // This often drops enough tokens to avoid the expensive LLM summarization.
    if micro_compact(messages) {
        let after_micro = estimate_tokens(messages);
        info!(
            "After micro-compaction: ~{} tokens (saved ~{})",
            after_micro,
            estimated_tokens.saturating_sub(after_micro)
        );
        if after_micro < token_threshold && messages.len() < COMPACT_MESSAGE_THRESHOLD {
            debug!("Micro-compaction sufficient; skipping full compact");
            return Ok(false);
        }
    }

    let split_point = messages.len().saturating_sub(KEEP_RECENT);
    let old_messages = &messages[..split_point];
    let recent_messages = &messages[split_point..];

    // Build a summary of old messages
    let old_text = messages_to_text(old_messages);

    let summary_prompt = format!(
        "Summarize the following conversation concisely, preserving key context, decisions, and any code changes discussed:\n\n{old_text}"
    );

    let request = MessageRequest {
        model: provider.model().to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(&summary_prompt)],
        tools: vec![],
        max_tokens: 2048,
        temperature: None,
        thinking_budget: Some(rusty_core::level_to_budget(rusty_core::ThinkingLevel::Minimal)),
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

    // Replace old messages with summary + recent
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

    Ok(true)
}

/// Force compaction regardless of thresholds — used for /compact command.
/// If `plan_text` is provided, it is appended to the summary.
pub async fn force_compact(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    system_prompt: &str,
    plan_text: Option<&str>,
) -> Result<bool, RustyError> {
    if messages.len() < 4 {
        return Ok(false);
    }

    info!("Force compacting: {} messages", messages.len());

    let split_point = messages.len().saturating_sub(KEEP_RECENT);
    let old_messages = &messages[..split_point];
    let recent_messages = &messages[split_point..];

    let old_text = messages_to_text(old_messages);
    let summary_prompt = format!(
        "Summarize the following conversation concisely, preserving key context, decisions, and any code changes discussed:\n\n{old_text}"
    );

    let request = MessageRequest {
        model: provider.model().to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(&summary_prompt)],
        tools: vec![],
        max_tokens: 2048,
        temperature: None,
        thinking_budget: Some(rusty_core::level_to_budget(rusty_core::ThinkingLevel::Minimal)),
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

/// Estimate token count for messages.
/// Counts ALL content blocks (text, tool results, tool use, thinking) rather
/// than just text, so compaction triggers when context is actually full.
fn estimate_tokens(messages: &[Message]) -> usize {
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
        let block_text = match &msgs[0].content {
            rusty_core::MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!("expected ToolResult"),
            },
            _ => panic!("expected Blocks"),
        };
        assert_eq!(block_text, "short");
    }

    #[test]
    fn micro_compact_preserves_recent_messages() {
        use rusty_core::ContentBlock;

        // Create 12 messages so the last 10 are preserved
        let mut msgs: Vec<Message> = (0..12)
            .map(|i| {
                Message::user_blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: format!("{i}"),
                    content: "a".repeat(1000),
                    is_error: Some(false),
                }])
            })
            .collect();

        micro_compact(&mut msgs);

        // First 2 should be compacted
        let t0 = match &msgs[0].content {
            rusty_core::MessageContent::Blocks(blocks) => match &blocks[0] {
                ContentBlock::ToolResult { content, .. } => content.clone(),
                _ => panic!("expected ToolResult"),
            },
            _ => panic!("expected Blocks"),
        };
        assert!(t0.contains("cleared"));

        // Last 10 should remain intact
        for msg in msgs.iter().skip(2) {
            let text = match &msg.content {
                rusty_core::MessageContent::Blocks(blocks) => match &blocks[0] {
                    ContentBlock::ToolResult { content, .. } => content.clone(),
                    _ => panic!("expected ToolResult"),
                },
                _ => panic!("expected Blocks"),
            };
            assert!(!text.contains("cleared"));
        }
    }
}

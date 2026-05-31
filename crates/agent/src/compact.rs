use futures::StreamExt;
use rusty_core::{Message, Role, RustyError};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use tracing::{debug, info};

/// Threshold: if estimated tokens exceed this, trigger compaction
const COMPACT_TOKEN_THRESHOLD: usize = 80_000;
/// Keep the last N messages un-compacted
const KEEP_RECENT: usize = 10;
/// Fallback message count threshold
const COMPACT_MESSAGE_THRESHOLD: usize = 40;

/// Check if compaction is needed and perform it
pub async fn maybe_compact(
    messages: &mut Vec<Message>,
    provider: &dyn LlmProvider,
    system_prompt: &str,
) -> Result<(), RustyError> {
    let estimated_tokens = estimate_tokens(messages);

    let needs_compact = messages.len() >= COMPACT_MESSAGE_THRESHOLD
        || estimated_tokens >= COMPACT_TOKEN_THRESHOLD;

    if !needs_compact {
        return Ok(());
    }

    info!(
        "Auto-compacting: {} messages (~{} tokens)",
        messages.len(),
        estimated_tokens
    );

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
    let mut new_messages = Vec::new();
    new_messages.push(Message::user(format!(
        "[Previous conversation summary]\n{summary}"
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

    Ok(())
}

/// Estimate token count for messages (rough: ~4 chars per token)
fn estimate_tokens(messages: &[Message]) -> usize {
    let total_chars: usize = messages
        .iter()
        .map(|m| m.get_all_text().len())
        .sum();
    total_chars / 4
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

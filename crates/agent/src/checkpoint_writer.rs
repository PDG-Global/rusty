// Copyright (C) 2026 PDG Global Limited
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Background checkpoint writer subagent.
//!
//! Runs as a background task to extract structured state from the conversation
//! without blocking the main agent loop. Inspired by MiMo Code's checkpoint
//! writer architecture.

use futures::StreamExt;
use rusty_core::{Message, RustyError};
use rusty_provider::{LlmProvider, MessageRequest, StreamEvent};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

use crate::compact::{
    read_file_content, clear_file, messages_to_text,
};

/// Result from a checkpoint writer run.
#[derive(Debug)]
pub enum WriterResult {
    /// Checkpoint was written successfully.
    Success { chars: usize },
    /// Writer failed with an error.
    Failed(String),
    /// Writer was skipped (already running or no messages).
    Skipped(String),
}

/// Checkpoint writer state, shared between the agent and the writer task.
pub struct CheckpointWriterState {
    /// Whether a writer is currently running.
    pub running: Arc<Mutex<bool>>,
    /// Channel to receive writer results.
    result_rx: mpsc::UnboundedReceiver<WriterResult>,
    /// Channel to send writer results (cloned for each writer spawn).
    pub result_tx: mpsc::UnboundedSender<WriterResult>,
}

impl CheckpointWriterState {
    pub fn new() -> Self {
        let (result_tx, result_rx) = mpsc::unbounded_channel();
        Self {
            running: Arc::new(Mutex::new(false)),
            result_rx,
            result_tx,
        }
    }

    /// Check if a writer is currently running.
    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// Try to receive a writer result (non-blocking).
    pub fn try_recv_result(&mut self) -> Option<WriterResult> {
        self.result_rx.try_recv().ok()
    }
}

/// Spawn a background checkpoint writer task.
///
/// Returns `true` if the writer was spawned, `false` if one is already running.
pub fn spawn_checkpoint_writer(
    messages: Vec<Message>,
    provider: Arc<dyn LlmProvider>,
    system_prompt: String,
    checkpoint_path: PathBuf,
    notes_path: Option<PathBuf>,
    running: Arc<Mutex<bool>>,
    result_tx: mpsc::UnboundedSender<WriterResult>,
) -> bool {
    // Check if already running
    {
        let guard = running.try_lock();
        match guard {
            Ok(mut is_running) => {
                if *is_running {
                    debug!("Checkpoint writer already running, skipping");
                    return false;
                }
                *is_running = true;
            }
            Err(_) => {
                debug!("Checkpoint writer lock contended, skipping");
                return false;
            }
        }
    }

    tokio::spawn(async move {
        let result = run_checkpoint_writer(
            &messages,
            &*provider,
            &system_prompt,
            &checkpoint_path,
            notes_path.as_deref(),
        )
        .await;

        // Clear running flag
        {
            let mut is_running = running.lock().await;
            *is_running = false;
        }

        // Send result
        let writer_result = match result {
            Ok(chars) => {
                info!("Checkpoint writer completed ({} chars)", chars);
                WriterResult::Success { chars }
            }
            Err(e) => {
                warn!("Checkpoint writer failed: {e}");
                WriterResult::Failed(e.to_string())
            }
        };

        let _ = result_tx.send(writer_result);
    });

    true
}

/// Run the checkpoint writer (extracts structured state from conversation).
async fn run_checkpoint_writer(
    messages: &[Message],
    provider: &dyn LlmProvider,
    system_prompt: &str,
    checkpoint_path: &Path,
    notes_path: Option<&Path>,
) -> Result<usize, RustyError> {
    let old_text = messages_to_text(messages);

    // Read existing checkpoint if it exists
    let existing = read_file_content(checkpoint_path).await;

    // Read notes if they exist
    let notes_content = if let Some(n_path) = notes_path {
        let content = read_file_content(n_path).await;
        // Clear notes after reading
        if content.is_some() {
            clear_file(n_path).await;
        }
        content
    } else {
        None
    };

    let mut prompt = String::from(CHECKPOINT_PROMPT);

    if let Some(ref existing_cp) = existing {
        if !existing_cp.trim().is_empty() {
            prompt.push_str("\n\nExisting checkpoint (update incrementally, preserve unchanged sections):\n");
            prompt.push_str(existing_cp);
        }
    }

    if let Some(notes) = &notes_content {
        if !notes.trim().is_empty() {
            prompt.push_str("\n\nScratchpad notes from the session (reconcile into appropriate sections, then clear):\n");
            prompt.push_str(notes);
        }
    }

    prompt.push_str("\n\nConversation:\n");
    prompt.push_str(&old_text);

    prompt.push_str("\n\nProduce the full checkpoint with all 11 sections. \
        Use the exact `## §N Title` headers. Update sections incrementally — \
        preserve content that's still accurate, update what changed.");

    let request = MessageRequest {
        model: provider.model().to_string(),
        system: Some(system_prompt.to_string()),
        messages: vec![Message::user(&prompt)],
        tools: vec![],
        max_tokens: 2048,
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

    tokio::fs::write(checkpoint_path, &checkpoint)
        .await
        .map_err(|e| RustyError::Other(format!("Failed to write checkpoint: {e}")))?;

    Ok(checkpoint.len())
}

/// Checkpoint extraction prompt (11 sections, matching MiMo Code structure).
const CHECKPOINT_PROMPT: &str = "\
You are extracting structured state from a conversation for future reference. \
Read the conversation below and produce a checkpoint with exactly these 11 sections:

## §1 Active intent
The user's most recent explicit request, VERBATIM BLOCK-QUOTED from the conversation.
Format: `> \"exact user words\"`
This is the anchor. Without verbatim, the next-cycle agent will lose the user's actual words.

## §2 Next concrete action
The single next concrete step, derived from §1 and current state.
Include a verbatim quote when the user explicitly stated a next step.

## §3 Directives (this session)
Session-specific working style only (e.g. 'prefer functional methods', 'no try/catch').
Project-level rules belong in MEMORY.md, not here.

## §4 Task tree
Hierarchical view of tasks with status: 🔵 open / 🔄 in_progress / 🟡 blocked / ✅ done / ❌ abandoned.
Source of truth = the todowrite tool's last output.

## §5 Current work
What was being done immediately before this checkpoint. Mention specific file paths and code locations.

## §6 Files and code sections
Files actively being read or modified. List with one-line purpose each.

## §7 Discovered knowledge (cross-task)
Facts learned during this session that may apply to future tasks.
Items here are candidates for promotion to MEMORY.md if they prove durable.

## §8 Errors and fixes
Errors encountered this session and how they were resolved. Newest first.

## §9 Live resources
Runtime state: git branch, uncommitted files, running processes, temp artifacts.

## §10 Design decisions and discussion outcomes
Decisions reached through discussion that produced no immediate code/file artifact.
Captures user intent and trade-off rationale.

## §11 Open notes
Catch-all for items that don't fit §1-§10. Quotes, unresolved questions, micro-observations.

Rules:
- Be concise. Each section should be 1-5 lines.
- Focus on facts, not narration.
- Include file paths and specific technical details.
- §1 MUST contain at least one block-quoted verbatim user request.
- §2 MUST be present and actionable — the agent wakes up with only this checkpoint.
- If a section has nothing, write '(none)' — do not omit sections.";

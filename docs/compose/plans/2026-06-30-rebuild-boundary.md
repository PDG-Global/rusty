# Rebuild Boundary Pattern Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the scattered post-compaction context injection with a single, rich synthetic user message at the compaction boundary — matching MiMo Code's rebuild boundary architecture.

**Architecture:** After Tier 3 compaction, instead of injecting checkpoint into the system prompt + a generic user message, we insert a single synthetic user message containing: checkpoint.md content, notes.md content, recent verbatim user messages, seam framing ("resume directly"), and a tail-aware reminder. This is the pattern MiMo uses and it prevents the agent from losing its bearings after compaction.

**Tech Stack:** Rust, tokio, existing `compact.rs` and `loop.rs` in `crates/agent/`

## Global Constraints

- No new dependencies — use only existing crate imports
- The `extract_section()` helper already exists in `loop.rs` — reuse it
- Keep `messages_to_text()` as-is — the rebuild context uses a different format
- `read_file_content()` and `clear_file()` already exist in `compact.rs` — reuse them
- The rebuild context replaces BOTH the system prompt checkpoint injection AND the generic post-compaction user message

## Files

| File | Action | Purpose |
|---|---|---|
| `crates/agent/src/compact.rs` | Modify | Add `render_rebuild_context()`, `recent_user_messages()`, `tail_aware_reminder()` |
| `crates/agent/src/loop.rs` | Modify | Replace post-compaction injection with rebuild boundary message; remove checkpoint from system prompt |

---

### Task 1: Add `render_rebuild_context()` to compact.rs

**Covers:** Core rebuild context assembly — checkpoint, notes, recent user messages, seam framing.

**Files:**
- Modify: `crates/agent/src/compact.rs` (add new public function after `messages_to_text`)
- Test: `crates/agent/src/compact.rs` (add tests in existing `mod tests`)

**Interfaces:**
- Consumes: `read_file_content()` (existing), `extract_section()` (from loop.rs — will be moved)
- Produces: `pub fn render_rebuild_context(checkpoint: Option<&str>, notes: Option<&str>, recent_user_texts: &[String], last_role: &str) -> String`

- [ ] **Step 1: Move `extract_section` from loop.rs to compact.rs**

The `extract_section` helper is currently in `loop.rs` but is needed by both files. Move it to `compact.rs` as a `pub(crate)` function so both modules can use it.

In `crates/agent/src/compact.rs`, add after the `messages_to_text` function:

```rust
/// Extract a section from a checkpoint markdown file by its `## §N Title` header.
/// Returns the section body (everything until the next `## §` header or end of file).
pub(crate) fn extract_section<'a>(checkpoint: &'a str, section_header: &str) -> Option<&'a str> {
    let start = checkpoint.find(section_header)?;
    let body_start = start + checkpoint[start..].find('\n')?;
    let rest = &checkpoint[body_start..];
    let end = rest[1..]
        .find("\n## §")
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    Some(&rest[..end])
}
```

In `crates/agent/src/loop.rs`, update all calls to `extract_section` to use `crate::compact::extract_section` instead of the local function, then remove the local `extract_section` function.

- [ ] **Step 2: Add `recent_user_messages()` helper**

In `crates/agent/src/compact.rs`, add after `extract_section`:

```rust
/// Extract the last N user messages as plain text, for verbatim preservation
/// in the rebuild context. Skips system messages and empty texts.
pub fn recent_user_messages(messages: &[Message], count: usize) -> Vec<String> {
    messages
        .iter()
        .rev()
        .filter(|m| m.role == Role::User)
        .filter_map(|m| {
            let text = m.get_all_text();
            // Skip compaction boundary messages
            if text.contains("[Previous conversation summary]")
                || text.contains("Conversation history was automatically compacted")
            {
                return None;
            }
            if text.trim().is_empty() {
                return None;
            }
            Some(text)
        })
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}
```

- [ ] **Step 3: Add `tail_aware_reminder()` helper**

In `crates/agent/src/compact.rs`, add after `recent_user_messages`:

```rust
/// Generate a tail-aware reminder based on the last message role in the
/// preserved tail. Mirrors MiMo's tail-aware system reminders.
pub fn tail_aware_reminder(messages: &[Message]) -> &'static str {
    let last = match messages.last() {
        Some(m) => m,
        None => return "",
    };
    match last.role {
        Role::Assistant => {
            // If the assistant's last message had tool use, it was mid-loop
            if last.has_tool_use() {
                "The previous assistant turn made tool calls. Process the tool results \
                 and continue your work loop."
            } else {
                "The previous assistant turn ended. Check your task list before stopping \
                 again — if work remains, continue."
            }
        }
        Role::User => "",
    }
}
```

- [ ] **Step 4: Add `render_rebuild_context()` function**

In `crates/agent/src/compact.rs`, add after `tail_aware_reminder`:

```rust
/// Render the full rebuild context that is injected as a synthetic user message
/// after Tier 3 compaction. Modeled after MiMo Code's renderRebuildContext().
///
/// Assembles: checkpoint.md content, notes.md content, recent verbatim user
/// messages, seam framing, and a tail-aware reminder into a single message
/// that tells the agent exactly where to resume.
pub async fn render_rebuild_context(
    checkpoint_path: Option<&Path>,
    notes_path: Option<&Path>,
    messages: &[Message],
) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Header: explicit framing that this is a continuation
    lines.push(
        "This session is being continued from a previous conversation that was \
         compacted to save context space. The checkpoint and notes below cover \
         the earlier portion of the conversation."
            .to_string(),
    );
    lines.push(String::new());

    // Section 1: Session checkpoint (full body)
    if let Some(cp_path) = checkpoint_path {
        if let Some(checkpoint) = read_file_content(cp_path).await {
            if !checkpoint.trim().is_empty() {
                lines.push("## Session checkpoint".to_string());
                lines.push(checkpoint.trim().to_string());
                lines.push(String::new());
            }
        }
    }

    // Section 2: Session notes (full body)
    if let Some(n_path) = notes_path {
        if let Some(notes) = read_file_content(n_path).await {
            if !notes.trim().is_empty() {
                lines.push("## Session notes".to_string());
                lines.push(notes.trim().to_string());
                lines.push(String::new());
            }
        }
    }

    // Section 3: Recent user messages (verbatim)
    let recent = recent_user_messages(messages, 5);
    if !recent.is_empty() {
        lines.push("## Recent user input (verbatim)".to_string());
        for msg in &recent {
            // Truncate very long messages
            if msg.len() > 2000 {
                lines.push(format!("> {}...(truncated)", &msg[..2000]));
            } else {
                lines.push(format!("> {msg}"));
            }
        }
        lines.push(String::new());
    }

    // Section 4: Seam framing — explicit resume instructions
    lines.push(
        "Recent messages are preserved below — the assistant turn (and any tool results) \
         you will see is real history, not pseudo-content. Continue your task by responding \
         to the most recent state."
            .to_string(),
    );
    lines.push(String::new());
    lines.push(
        "Resume directly. Do not acknowledge this memory dump, do not recap, \
         do not preface with \"I will continue\" or similar. Pick up the last task \
         as if the break never happened."
            .to_string(),
    );

    // Section 5: Tail-aware reminder
    let reminder = tail_aware_reminder(messages);
    if !reminder.is_empty() {
        lines.push(String::new());
        lines.push(reminder.to_string());
    }

    lines.join("\n")
}
```

- [ ] **Step 5: Add tests for the new functions**

In `crates/agent/src/compact.rs`, inside `mod tests`, add:

```rust
#[test]
fn extract_section_finds_intent() {
    let cp = "\
# Session checkpoint

## §1 Active intent
> Transfer the site files to index.html

## §2 Next concrete action
Read rusty-website (1).html and extract content.

## §3 Directives (this session)
(none)
";
    let intent = extract_section(cp, "§1 Active intent").unwrap();
    assert!(intent.contains("Transfer the site files"));
    let next = extract_section(cp, "§2 Next concrete action").unwrap();
    assert!(next.contains("Read rusty-website"));
}

#[test]
fn extract_section_returns_none_for_missing() {
    let cp = "## §1 Active intent\nhello\n";
    assert!(extract_section(cp, "§99 Missing").is_none());
}

#[test]
fn extract_section_last_section_goes_to_eof() {
    let cp = "## §11 Open notes\nSome notes here.\nNo trailing section.";
    let body = extract_section(cp, "§11 Open notes").unwrap();
    assert!(body.contains("Some notes here."));
    assert!(body.contains("No trailing section."));
}

#[test]
fn recent_user_messages_skips_compaction_boundaries() {
    let msgs = vec![
        Message::user("First real request"),
        Message::assistant("Working on it"),
        Message::user("[Previous conversation summary]\nSome summary"),
        Message::user("Second real request"),
        Message::assistant("Done"),
    ];
    let recent = recent_user_messages(&msgs, 10);
    assert_eq!(recent.len(), 2);
    assert!(recent[0].contains("First real request"));
    assert!(recent[1].contains("Second real request"));
}

#[test]
fn recent_user_messages_limits_count() {
    let msgs: Vec<Message> = (0..10)
        .map(|i| Message::user(format!("Request {i}")))
        .collect();
    let recent = recent_user_messages(&msgs, 3);
    assert_eq!(recent.len(), 3);
    assert!(recent[0].contains("Request 7"));
    assert!(recent[2].contains("Request 9"));
}

#[test]
fn tail_aware_reminder_for_assistant_with_tools() {
    use rusty_core::ContentBlock;
    let msgs = vec![Message::assistant_blocks(vec![
        ContentBlock::Text { text: "Let me check".into() },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        },
    ])];
    let reminder = tail_aware_reminder(&msgs);
    assert!(reminder.contains("tool calls"));
    assert!(reminder.contains("continue your work loop"));
}

#[test]
fn tail_aware_reminder_for_user_message() {
    let msgs = vec![Message::user("Do this")];
    let reminder = tail_aware_reminder(&msgs);
    assert_eq!(reminder, "");
}
```

- [ ] **Step 6: Run tests to verify**

Run: `cargo test -p rusty-agent`
Expected: All tests pass including the new ones.

- [ ] **Step 7: Verify compilation**

Run: `cargo check --workspace`
Expected: Clean compilation.

---

### Task 2: Wire rebuild boundary into the agent loop

**Covers:** Replace scattered post-compaction injection with the single rebuild boundary message.

**Files:**
- Modify: `crates/agent/src/loop.rs` (lines ~237-254 in `refresh_system_prompt`, lines ~729-755 in `run`)

**Interfaces:**
- Consumes: `crate::compact::render_rebuild_context()` (from Task 1), `crate::compact::extract_section()` (from Task 1)
- Produces: Modified agent loop with single rebuild boundary message

- [ ] **Step 1: Remove checkpoint injection from `refresh_system_prompt()`**

In `crates/agent/src/loop.rs`, replace the `refresh_system_prompt` method. Remove the checkpoint injection block (lines 237-254) since the rebuild context now handles this:

```rust
    async fn refresh_system_prompt(&mut self) {
        let mut prompt = self.base_system_prompt.clone();
        // Inject permission-mode guidance so the model knows whether it should
        // be autonomous or wait for approvals.
        if let Some(mode_text) =
            rusty_core::permissions::permission_mode_prompt(self.permission_mode)
        {
            prompt.push_str("\n\n");
            prompt.push_str(mode_text);
        }

        self.system_prompt = prompt;
    }
```

- [ ] **Step 2: Replace post-compaction message with rebuild boundary**

In `crates/agent/src/loop.rs`, in the `run()` method, replace the entire `if new_tier == CheckpointTier::Compacted` block (lines ~729-755) with:

```rust
                if new_tier == crate::compact::CheckpointTier::Compacted {
                    // Insert a rich rebuild boundary message — the single source
                    // of truth for post-compaction recovery. Matches MiMo Code's
                    // insertRebuildBoundary pattern.
                    let rebuild_ctx = crate::compact::render_rebuild_context(
                        self.checkpoint_path.as_deref(),
                        self.notes_path.as_deref(),
                        &self.messages,
                    )
                    .await;
                    self.messages.push(Message::user(rebuild_ctx));
                }
```

- [ ] **Step 3: Update `extract_section` calls in loop.rs**

Since `extract_section` was moved to `compact.rs` in Task 1, verify that any remaining calls in `loop.rs` use `crate::compact::extract_section`. The calls in the post-compaction block were removed in Step 2, so there should be none remaining. If there are, update them.

- [ ] **Step 4: Remove the local `extract_section` from loop.rs**

Delete the `extract_section` function definition from `loop.rs` (around line 1599-1611) since it now lives in `compact.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p rusty-agent`
Expected: All tests pass.

- [ ] **Step 6: Full workspace check**

Run: `cargo check --workspace`
Expected: Clean compilation across all crates.

---

### Task 3: Verify end-to-end

**Covers:** Ensure the full flow works correctly.

**Files:**
- No new files

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: All tests pass across all crates.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace`
Expected: No new warnings.

- [ ] **Step 3: Review changes**

Run: `git diff`
Expected: Changes are minimal and focused:
- `compact.rs`: new functions (`extract_section`, `recent_user_messages`, `tail_aware_reminder`, `render_rebuild_context`) + tests
- `loop.rs`: simplified `refresh_system_prompt`, replaced post-compaction block, removed local `extract_section`

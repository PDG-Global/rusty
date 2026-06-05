# Rusty 10-Feature Research Report

This document covers the research findings for 10 proposed features/improvements to the Rusty codebase, including what exists today, what needs to change, key files, data structures, and implementation notes.

---

## 1. Multimodal / Image Support (ContentBlock::Image)

### Current State

Image support exists but is partial. There are two entry points:

1. **Pasted images in TUI** (`crates/tui/src/app.rs:226-231`): When an image is pasted (iTerm2 inline image protocol), the TUI stores it as `ContentBlock::Image { media_type, data }` and attaches it to the user message.

2. **Image tool results** (`crates/provider/src/types.rs:286-299`): When a tool returns base64 image data, the provider layer converts it to OpenAI's `image_url` format with a `data:{media_type};base64,{data}` URL.

### Key Data Structures

```rust
// crates/core/src/types.rs
pub enum ContentBlock {
    Text { text: String },
    Image { media_type: String, data: String },  // base64-encoded
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: Vec<ContentBlock>, is_error: bool },
    Thinking { thinking: String },
}
```

### Gaps / What Would Need to Change

- **Image content in user messages is silently dropped by the provider**: `rusty_content_to_oai()` (`crates/provider/src/types.rs:142-171`) only handles `Text` and `Thinking` blocks. When a `ContentBlock::Image` appears in a user message (e.g. from pasted images), it is skipped. This means pasted images never actually reach the LLM despite the TUI collecting them.
- **Fix**: Add an `Image` arm to `rusty_content_to_oai()` that converts to OpenAI's `image_url` content part format, similar to how `format_tool_result_content()` handles images at line 286.
- **Vision model support**: The model registry does not track which models support vision. A `supports_vision` flag on the `ModelEntry` would allow the UI to warn when attaching images to non-vision models.
- **File-based images**: No tool exists to load images from disk (e.g. `file_read` with binary mode or a dedicated `image_read` tool).

### Files

| File | Relevance |
|---|---|
| `crates/core/src/types.rs` | `ContentBlock::Image` definition |
| `crates/provider/src/types.rs:142-171` | `rusty_content_to_oai()` (missing Image arm) |
| `crates/provider/src/types.rs:286-299` | `format_tool_result_content()` (has Image handling) |
| `crates/tui/src/app.rs:226-231` | Pasted image capture in TUI |
| `crates/cli/src/main.rs:329-339` | Clipboard image polling (headless mode) |

---

## 2. Configurable Context Window Per Model Entry

### Current State

**Already implemented.** The `ModelEntry` struct in the model registry has a `context_window` field:

```rust
// crates/core/src/model_registry.rs:40
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub provider: ProviderType,
    pub context_window: Option<usize>,   // <-- per-model override
    pub max_output_tokens: Option<usize>,
    pub supports_vision: bool,
    pub supports_thinking: bool,
    pub reasoning_effort: Option<String>,
    pub aliases: Vec<String>,
    pub description: Option<String>,
}
```

When `context_window` is `None`, a default of 128,000 tokens is used (`crates/core/src/model_registry.rs:413-414`).

### How It's Used

- `resolve_model()` in `crates/cli/src/main.rs:347-364` looks up the model in the registry and applies the context window to the config.
- `AgentConfig` has a `context_window` field that feeds into compaction decisions.

### What Could Be Improved

- No way for users to override context_window in `settings.json` per model. The registry values are hardcoded.
- No CLI flag to set context window (only `--max-tokens` for output).
- The compaction threshold in `compact.rs` uses a hardcoded 80% of context window; could be configurable.

### Files

| File | Relevance |
|---|---|
| `crates/core/src/model_registry.rs:40-53` | `ModelEntry` with `context_window` field |
| `crates/core/src/model_registry.rs:55-414` | `build_default_registry()` with per-model values |
| `crates/cli/src/main.rs:347-364` | `resolve_model()` applies context window |
| `crates/agent/src/compact.rs` | Compaction logic using context window |

---

## 3. Model Registry with Provider Type Routing + Anthropic Support

### Current State

**Already implemented.** The model registry (`crates/core/src/model_registry.rs`) provides:

1. **`ModelEntry`** with a `provider: ProviderType` field that routes to the correct API backend.
2. **`ProviderType`** enum with variants: `OpenAICompatible`, `Anthropic`, `Ollama`.
3. **100+ pre-configured models** across providers (OpenAI, Anthropic, Google, DeepSeek, Xiaomi, Moonshot, Qwen, Mistral, Meta, xAI, etc.).
4. **Registry API**: `list_models()`, `find_model()`, `resolve_alias()`, `search_models()`, `get_providers()`, `get_models_by_provider()`.

### Anthropic Provider

The Anthropic provider (`crates/provider/src/anthropic.rs`) implements the Anthropic Messages API:

- Endpoint: `https://api.anthropic.com/v1/messages`
- Uses `anthropic-version: 2023-06-01` header
- Streaming via SSE with `content_block_delta` events
- Supports tool use, thinking blocks, and extended thinking
- Handles Anthropic-specific error codes (overloaded, rate limits)
- Prompt caching via `cache_control` breakpoints on system messages and the last two user messages

### Provider Routing

`resolve_provider()` in `crates/cli/src/main.rs:235-307`:
- Takes `ProviderConfig` + `ProviderType`
- Dispatches to `OpenAiProvider::new()` or `AnthropicProvider::new()`
- `resolve_model()` checks if the selected model is in the registry and overrides provider type if found

### Key Design Decisions

- The registry is a singleton built at startup via `build_default_registry()` (lazy_static).
- Provider-specific headers (e.g. Kimi's `X-Msh-Context-Id`) are passed via `ProviderConfig.extra_headers`.
- The `reasoning_content` field in `OaiMessage` handles MiMo/DeepSeek-style reasoning that arrives in a non-standard `reasoning_content` field.

### Files

| File | Relevance |
|---|---|
| `crates/core/src/model_registry.rs` | Full registry with 100+ models |
| `crates/provider/src/anthropic.rs` | Anthropic Messages API provider |
| `crates/provider/src/openai.rs` | OpenAI-compatible provider |
| `crates/cli/src/main.rs:235-307` | `resolve_provider()` routing logic |
| `crates/cli/src/main.rs:347-364` | `resolve_model()` with registry lookup |

---

## 4. Concurrent Tool Execution

### Current State

**Already implemented.** Tools are executed in parallel with a concurrency limit of 8.

```rust
// crates/agent/src/loop.rs:466-530
let tool_results = futures::stream::iter(tool_calls.iter())
    .map(|tc| async { self.execute_tool(tc).await })
    .buffer_unordered(8)  // up to 8 concurrent tool executions
    .collect::<Vec<_>>()
    .await;
```

### How It Works

1. The LLM returns one or more tool calls in a single response.
2. All tool calls are collected into a `Vec<ToolCall>`.
3. `futures::stream::iter()` creates a stream from the tool calls.
4. `.buffer_unordered(8)` executes up to 8 tools concurrently (order of results matches input order).
5. Each tool execution includes permission checks, sandbox validation, and the actual tool `execute()` call.
6. Results are collected and appended to the message history before the next LLM turn.

### Permission Handling

Permission checks happen inside `execute_tool()` (line 501-507), which calls `check_permission_tiered()`. Since permissions are checked per-tool, concurrent execution does not create race conditions: each tool gets its own permission decision.

### Limitations

- The concurrency limit (8) is hardcoded. Could be made configurable.
- `buffer_unordered` means results may arrive in any order, but `collect()` preserves the original order of the `tool_calls` slice.
- No timeout per individual tool execution (only the overall HTTP client timeout of 600s).

### Files

| File | Relevance |
|---|---|
| `crates/agent/src/loop.rs:466-530` | Concurrent tool execution with `buffer_unordered(8)` |
| `crates/agent/src/loop.rs:688-735` | `execute_tool()` with permission checks |

---

## 5. Persistent Project Memory System

### Current State

**Already implemented.** The memory system provides persistent, cross-session storage scoped to the current working directory.

### Storage

- **Location**: `~/.rusty/memory/{sanitized_path}/memories.json`
- **Path sanitisation**: Working directory path is sanitised (slashes to underscores, leading underscore removed) to create a unique directory per project.
- **Format**: JSON array of `Memory` objects.

### Data Structure

```rust
// crates/core/src/memory.rs:28-36
pub struct Memory {
    pub id: String,           // nanoid
    pub name: String,         // lowercase hyphenated identifier
    pub content: String,      // the memory content
    pub category: MemoryCategory,  // UserPreference, ProjectDecision, Context, Other
    pub created_at: String,
    pub updated_at: String,
}
```

### Tools

| Tool | Action | Description |
|---|---|---|
| `memory` | `store` | Create or update a memory by name |
| `memory` | `get` | Retrieve a specific memory by name |
| `memory` | `list` | List all memories for the current project |
| `memory` | `delete` | Delete a memory by name |

### Integration

- Memories are injected into the system prompt via `build_system_prompt()` in `crates/agent/src/lib.rs:181-190`.
- The memory tool is excluded from the sub-agent tool list (line 120) to prevent sub-agents from modifying the parent's memory.
- The memory tool has `PermissionLevel::None` (no permission prompt needed).

### Files

| File | Relevance |
|---|---|
| `crates/core/src/memory.rs` | `MemoryManager`, `Memory` struct, CRUD operations |
| `crates/tools/src/memory.rs` | `MemoryTool` implementation |
| `crates/agent/src/lib.rs:181-190` | Memory injection into system prompt |

---

## 6. /permissions Slash Command

### Current State

**Does not exist.** The current slash commands are:

| Command | Alias | Description |
|---|---|---|
| `/help` | `/h`, `/?` | List available commands |
| `/init` | | Generate AGENTS.md |
| `/resume` | `/r` | Resume a saved session |
| `/sessions` | `/s` | List saved sessions |
| `/compact` | | Force compaction |
| `/clear` | | Clear screen |
| `/copy` | `/c` | Copy last response to clipboard |
| `/model` | `/m` | Show current model |
| `/rename` | | Rename session |
| `/quit` | `/exit`, `/q` | Exit |

### What a /permissions Command Would Need

1. **Display current mode**: Show `PermissionMode` (Default/AcceptEdits/Bypass/Plan).
2. **Show allowlist**: Display both permanent (`settings.json`) and session-level allowed tools.
3. **Switch mode**: Allow changing permission mode at runtime (e.g. `/permissions bypass`).
4. **Show pending requests**: If a permission prompt is active, show its details.

### Implementation Path

- Add `SlashCommand::Permissions` variant to `crates/tui/src/app.rs`.
- Handle it in the CLI's stdin command processing (`crates/cli/src/main.rs:887-897`).
- The `AppState` already tracks `permission_prompt` state; a display command could introspect this.
- Mode switching would require updating `config.permission_mode` at runtime and potentially clearing session allowlists.

### Files

| File | Relevance |
|---|---|
| `crates/tui/src/app.rs` | `SlashCommand` enum, command handling |
| `crates/cli/src/main.rs:887-897` | Stdin mode command processing |
| `crates/core/src/permissions.rs` | `PermissionMode` enum, permission logic |
| `crates/agent/src/loop.rs:688-735` | `check_permission_tiered()` |

---

## 7. Security Hardening / Prompt Injection Protection

### Current State

**No dedicated prompt injection protection exists.** The system relies on:

1. **Path sandboxing**: `resolve_path()` in `crates/tools/src/mod.rs:13-64` canonicalises paths and rejects any that escape the working directory. All file tools use this.
2. **Bash command classification**: `classify_bash_command()` in `crates/core/src/permissions.rs:299-437` classifies bash commands as read-only or write/execute based on command name and flags.
3. **Permission tiering**: The tiered permission system prevents unauthorised tool use.
4. **Sub-agent isolation**: Sub-agents get `BypassPermissions` but cannot spawn further sub-agents (agent tool is excluded).

### Gaps

- **No content sanitisation**: Tool results (especially from `web_fetch`, `bash`, `file_read`) are passed directly to the LLM without sanitisation. A malicious webpage or file could contain prompt injection text.
- **No system prompt hardening**: The system prompt does not include instructions to ignore embedded instructions in tool results.
- **No output filtering**: The LLM's output is not checked for sensitive data exfiltration patterns (e.g. API keys in tool arguments).
- **No rate limiting on tool calls**: A compromised model could make unlimited tool calls within a single turn.
- **Web fetch has no content-type restrictions**: Could fetch and process executable content.

### Potential Improvements

1. Add system prompt instructions warning about prompt injection in tool results.
2. Implement content-type filtering for `web_fetch` (restrict to text/html, text/plain, application/json).
3. Add per-turn tool call limits.
4. Sanitise or tag tool result content to help the model distinguish between trusted (system) and untrusted (tool output) content.
5. Add API key detection/redaction in tool arguments.

### Files

| File | Relevance |
|---|---|
| `crates/tools/src/mod.rs:13-64` | `resolve_path()` path sandboxing |
| `crates/core/src/permissions.rs:299-437` | `classify_bash_command()` |
| `crates/tools/src/web_fetch.rs` | Web fetch (no content-type filtering) |
| `crates/tools/src/bash.rs` | Bash execution |
| `crates/agent/src/lib.rs:51-214` | System prompt construction |

---

## 8. Session TTL

### Current State

**No session TTL exists.** Sessions are persisted indefinitely as JSON files in `~/.rusty/sessions/`.

### Session Storage

- **Location**: `~/.rusty/sessions/{session_id}.json`
- **Format**: `ConversationSession` struct with id, title, messages, model, timestamps, metadata, name, working_dir.
- **Listing**: `list_sessions()` returns all sessions sorted by `updated_at` descending.
- **No expiry**: Sessions are never automatically cleaned up.

### What a TTL System Would Need

1. **Configurable TTL**: Add `session_ttl_days: Option<u32>` to `Settings`.
2. **Cleanup on startup**: Run a cleanup pass when the CLI starts, deleting sessions older than the TTL.
3. **Cleanup command**: Add `/cleanup-sessions` or integrate into `/sessions` with a filter.
4. **Soft delete**: Move expired sessions to a trash directory before permanent deletion.
5. **Per-session override**: Allow pinning sessions to prevent expiry.

### Implementation Path

- Add `session_ttl_days` to `Settings` in `crates/core/src/config.rs`.
- Add `cleanup_expired_sessions()` to `ConversationSession` in `crates/core/src/history.rs`.
- Call it from `crates/cli/src/main.rs` during startup.
- Parse `updated_at` timestamps and compare against `Utc::now() - Duration::days(ttl)`.

### Files

| File | Relevance |
|---|---|
| `crates/core/src/history.rs` | `ConversationSession`, save/load/list |
| `crates/core/src/config.rs` | `Settings` struct (would add TTL field) |
| `crates/cli/src/main.rs` | Startup logic (would call cleanup) |

---

## 9. Task Tracking as Default Behaviour

### Current State

**Partially implemented with soft enforcement.** The system has two layers:

1. **System prompt instructions** (soft): The system prompt in `crates/agent/src/lib.rs:86-115` includes detailed "Task Tracking" rules instructing the model to always use `todowrite` for multi-step work. However, this is a prompt-level instruction, not enforced in code.

2. **Task nudge mechanism** (hard, reactive): When the model tries to stop (`end_turn` or `stop` finish reason) with incomplete tasks, the agent loop nudges it to continue (`crates/agent/src/loop.rs:626-683`):
   - Scans the message history for the most recent `todowrite` call
   - Extracts tasks where `status != "completed"` and `status != "cancelled"`
   - Also handles missing `status` field (treats as incomplete)
   - Nudge limit scales with task count: `(incomplete_count * 2).max(8)`
   - If the nudge limit is hit, it gives up and lets the model stop

3. **`--plan-with-tasks` flag** (opt-in): When enabled, adds additional planning-focused instructions to the system prompt (`crates/agent/src/lib.rs:210-211`). Defaults to `false`.

### What "Default Behaviour" Would Mean

- Making `plan_with_tasks: true` the default in `Config::default()`.
- Or: removing the flag entirely and always including planning instructions.
- The nudge mechanism already provides hard enforcement once tasks are created; the gap is ensuring tasks are created in the first place.

### Files

| File | Relevance |
|---|---|
| `crates/agent/src/lib.rs:86-115` | Task tracking system prompt instructions |
| `crates/agent/src/lib.rs:210-211` | `--plan-with-tasks` conditional instructions |
| `crates/agent/src/loop.rs:626-683` | Task nudge mechanism |
| `crates/agent/src/loop.rs:230-268` | `incomplete_task_details()` extraction |
| `crates/tools/src/todowrite.rs` | `todowrite` tool implementation |
| `crates/core/src/config.rs:235,264` | `plan_with_tasks` field (defaults to false) |

---

## 10. User-Agent Header on HTTP Clients

### Current State

**Already implemented.** The `OpenAiProvider` sets a User-Agent header on its HTTP client:

```rust
// crates/provider/src/openai.rs:41-42
let client = reqwest::Client::builder()
    .user_agent(rusty_core::rusty_user_agent())
    .default_headers(headers)
    .timeout(Duration::from_secs(600))
    .build()
    .map_err(|e| RustyError::Other(format!("Failed to build HTTP client: {e}")))?;
```

The `rusty_user_agent()` function (`crates/core/src/lib.rs:31-34`) returns a string like `rusty/0.1.5`.

### What Could Be Improved

- **Anthropic provider**: The `AnthropicProvider` in `crates/provider/src/anthropic.rs` builds its own `reqwest::Client`. It should also set the User-Agent header (needs verification).
- **Richer user agent**: Include platform info (e.g. `rusty/0.1.5 (macos; aarch64)`) for better server-side analytics.
- **Configurable**: Allow users to override the User-Agent string in settings.

### Files

| File | Relevance |
|---|---|
| `crates/core/src/lib.rs:31-34` | `rusty_user_agent()` function |
| `crates/provider/src/openai.rs:41-42` | User-Agent set on OpenAI client |
| `crates/provider/src/anthropic.rs` | Anthropic client (needs User-Agent check) |
| `crates/tools/src/web_fetch.rs` | Web fetch client (should also set User-Agent) |

---

## Summary Matrix

| # | Feature | Status | Effort | Key Files |
|---|---|---|---|---|
| 1 | Multimodal/Image | Partial (bug: images dropped in user messages) | Small | `provider/types.rs` |
| 2 | Context Window Per Model | Done | None | `core/model_registry.rs` |
| 3 | Model Registry + Anthropic | Done | None | `core/model_registry.rs`, `provider/anthropic.rs` |
| 4 | Concurrent Tool Execution | Done (limit 8) | Tiny (make configurable) | `agent/loop.rs` |
| 5 | Persistent Memory | Done | None | `core/memory.rs`, `tools/memory.rs` |
| 6 | /permissions Command | Not started | Medium | `tui/app.rs`, `cli/main.rs` |
| 7 | Security Hardening | Not started | Medium-Large | Multiple |
| 8 | Session TTL | Not started | Small | `core/history.rs`, `core/config.rs` |
| 9 | Task Tracking Default | Partial (soft enforcement) | Small | `core/config.rs`, `agent/lib.rs` |
| 10 | User-Agent Header | Done (OpenAI provider) | Tiny (verify Anthropic, web_fetch) | `provider/anthropic.rs`, `tools/web_fetch.rs` |

---

*Generated: 2026-06-05*
*Based on codebase at commit 9b1cbd2*

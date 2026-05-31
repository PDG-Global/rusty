# AGENTS.md — Rusty Codebase Guide

## Overview

**Rusty** is a lightweight AI coding agent written in Rust. It connects to OpenAI-compatible LLM APIs, streams responses, executes tools (file I/O, bash, search, web fetch, sub-agents), and enforces a tiered permission system. It runs as a terminal UI (TUI) or in headless/pipe modes.

- **Language:** Rust (edition 2021)
- **Runtime:** Tokio async
- **Default model:** `mimo-v2.5-pro` (Xiaomi MiMo)
- **Config dir:** `~/.rusty/`

---

## Workspace Structure

```
rusty/
├── Cargo.toml              # Workspace root (6 crates)
├── crates/
│   ├── core/               # rusty-core: types, config, permissions, errors, history
│   ├── provider/           # rusty-provider: OpenAI-compatible HTTP/SSE streaming client
│   ├── tools/              # rusty-tools: all tool implementations
│   ├── agent/              # rusty-agent: agent loop, compaction, sub-agent spawning
│   ├── tui/                # rusty-tui: ratatui-based terminal UI (app state + rendering)
│   └── cli/                # rusty (binary): CLI args, main entry point, TUI/headless modes
└── tests/                  # (currently empty — tests live inline in modules)
```

### Dependency Graph

```
cli → agent → provider
  │       ↘ tools → core
  ├→ tui ──────→ core
  └→ core
```

---

## Crate Details

### `crates/core` — Foundation Layer

| File | Purpose |
|---|---|
| `types.rs` | `Role` (User/Assistant), `ContentBlock` (Text/ToolUse/ToolResult/Thinking), `Message`, `MessageContent` (Text/Blocks), `UsageInfo`, `ToolDefinition`. Message has helpers: `user()`, `assistant()`, `user_blocks()`, `assistant_blocks()`, `get_text()`, `get_all_text()`, `has_tool_use()`, `get_tool_use_blocks()`. |
| `config.rs` | `Config` (runtime config with model, api_key, api_base, max_tokens, temperature, working_dir, permission_mode), `Settings` (persisted `~/.rusty/settings.json` with theme, default_model, permanent permissions), `add_permanent_permission()` |
| `permissions.rs` | `PermissionMode` (Default/AcceptEdits/Bypass/Plan), `PermissionLevel` (None/ReadOnly/Write/Execute), `PermissionRequest`/`PermissionDecision`, bash command classifier (`classify_bash_command` categorizes commands as ReadOnly/Write/Execute based on executable name and flags), `build_tool_description`, `make_allow_key` |
| `error.rs` | `RustyError` enum with `thiserror`: Api, ApiStatus, Auth, PermissionDenied, Tool, Io, Json, Http, RateLimit (with retry_after), ContextWindowExceeded, MaxTokensReached, Cancelled, Config, Other. Helpers: `is_retryable()`, `is_context_limit()` |
| `context.rs` | `build_system_context()` (platform info, git branch/commit, environment), `build_user_context()` (reads `~/.rusty/CLAUDE.md` for user instructions, includes current date) |
| `history.rs` | `ConversationSession` — save/load/list sessions in `~/.rusty/sessions/` as JSON files with id, messages, model, timestamps |
| `cost.rs` | Token cost calculation utilities (pricing per model) |

### `crates/provider` — LLM API Client

| File | Purpose |
|---|---|
| `types.rs` | `StreamEvent` enum (TextDelta, ThinkingDelta, ToolCallDelta, Usage, Done, Error), `MessageRequest`, `ToolDefinition` (API-facing), `ProviderConfig`. Also contains OpenAI wire format types (`OaiRequest`, `OaiMessage`, `OaiTool`, `OaiToolCall`, `OaiResponse`, `OaiStreamChunk`, etc.) and conversion helpers (`rusty_messages_to_oai`, `rusty_tools_to_oai`, `oai_response_to_rusty`). |
| `openai.rs` | `OpenAiProvider` — full OpenAI Chat Completions streaming implementation with SSE parsing, retry logic (exponential backoff for rate limits/529s), delta accumulation into `StreamEvent`s |
| `mod.rs` | `LlmProvider` trait: `create_message_stream()`, `model()`, `messages()` |

**Key pattern:** All LLM communication is streaming-first. The provider yields a `Stream<Item = Result<StreamEvent, RustyError>>` which the agent loop consumes.

### `crates/tools` — Tool Implementations

| File | Tool | Permission | Description |
|---|---|---|---|
| `file_read` | `file_read` | ReadOnly | Read file contents with optional offset/limit. Uses `resolve_path` sandbox. |
| `file_write.rs` | `file_write` | Write | Create/overwrite files, auto-creates parent dirs. Uses `resolve_path` sandbox. |
| `file_edit.rs` | `file_edit` | Write | Exact string match-and-replace editing. Uses `resolve_path` sandbox. |
| `bash.rs` | `bash` | **Classified per-command** | Execute shell commands; read-only commands (ls, git status, cargo check, etc.) bypass write permissions via `classify_bash_command`. |
| `grep.rs` | `grep` | ReadOnly | Regex search across files with glob filtering. Caps at 200 results. Skips binary file extensions. |
| `glob.rs` | `glob` | ReadOnly | File pattern matching. |
| `web_fetch.rs` | `web_fetch` | ReadOnly | Fetch URLs via `reqwest` (30s timeout). Returns text with configurable `max_length` (default 10000 chars). Truncates with length notice. |
| `agent.rs` | `agent` | None | Spawn sub-agents via a `SubAgentFn` callback. Accepts `task` (required) and `context` (optional) params. |

**Tool trait** (`mod.rs`): `name()`, `description()`, `input_schema()`, `permission_level()`, `execute()`, `definition()` (generates `ToolDefinition` for the API).

**`resolve_path(path_str, working_dir)`** — shared sandbox utility in `mod.rs`. Canonicalizes paths, resolves symlinks and `..`, and rejects any path that escapes the working directory. Used by `file_read`, `file_write`, and `file_edit`.

**`all_tools()`** returns the standard set. The `agent` tool is added separately by the CLI with a wired-up spawn function.

### `crates/agent` — Agent Loop

| File | Purpose |
|---|---|
| `loop.rs` | **`Agent` struct** — the core orchestrator. Holds provider, tools, config, message history, permission state. `run()` method implements the turn loop: send messages → stream response → accumulate text + tool calls → execute tools → repeat until done, max turns, or cancellation. Supports `cancel()` via `AtomicBool`. |
| `compact.rs` | Auto-compaction: when messages exceed ~80k tokens or 40 messages, summarizes older messages (keeping last 10) via an LLM call. Uses a dedicated compaction prompt. |
| `lib.rs` | `spawn_subagent()` (same-process tokio task with BypassPermissions), `make_agent_tool()` (constructs `AgentTool` with spawn callback), `build_system_prompt()` (assembles system prompt with tool descriptions, permissions, platform info, date) |

**Agent loop flow:**
1. Add user message to history
2. Maybe compact (summarize old messages)
3. Send to LLM with streaming
4. Accumulate text deltas and tool call deltas
5. If tool calls present: execute each tool, append results, loop back to step 2
6. If no tool calls: return final text

**Permission system** (tiered check in `check_permission_tiered`):
1. Bypass mode → allow all
2. Plan mode → read-only only
3. ReadOnly/None tools → auto-allow
4. AcceptEdits + Write → auto-allow
5. Permanent allowlist (from `~/.rusty/settings.json`)
6. Session allowlist (runtime)
7. Interactive callback (TUI prompt)
8. Default: deny

### `crates/tui` — Terminal UI

| File | Purpose |
|---|---|
| `app.rs` | `AppState` — all UI state (input buffer, cursor position, messages, streaming state, permission prompts, history navigation, scroll offset). `AgentEvent` enum for agent→UI communication (TextDelta, ThinkingDelta, ToolCallStart, ToolCallDelta, ToolResult, Done, Error). Key handling logic with command history. |
| `ui.rs` | Ratatui rendering: chat area with markdown support (bold, italic, code blocks, tables with box-drawing characters), input box with cursor, status bar (model, permissions, token usage), permission prompt overlay. Message roles are color-coded. |
| `lib.rs` | Generic `run()` function (unused by CLI — CLI implements its own loop for tighter control) |

**UI features:**
- Message history with user/assistant/system roles (color-coded)
- Streaming text display with cursor indicator
- Thinking/reasoning display (dimmed, italic)
- Table rendering with box-drawing characters
- Inline markdown: `**bold**`, `*italic*`, `` `code` ``
- Command history (Up/Down arrows)
- Permission prompt overlay (y/n/a/d/Esc)
- Esc cancels streaming

### `crates/cli` — Binary Entry Point

`main.rs` handles:

1. **CLI args** (clap): `--prompt`, `--model`, `--preset` (xiaomi/kimi/openai/ollama/deepseek), `--api-key`, `--api-base`, `--cwd`, `--permissions` (default/accept-edits/bypass/plan), `--resume`, `--list-sessions`, `--headless`, `--max-turns`, `--max-tokens`, `--temperature`, `--verbose`
2. **Config resolution**: preset defaults → `~/.rusty/settings.json` → CLI flags (later overrides earlier). Presets define `api_base`, `default_model`, and `default_permission_mode`.
3. **Three run modes:**
   - **TUI mode** (default): full terminal UI with streaming, permission prompts, session save on exit
   - **Headless mode** (`--prompt`): single prompt, print response, save session
   - **Stdin mode** (`--headless`): interactive line-by-line REPL without TUI

---

## Key Architectural Patterns

1. **Streaming-first**: All LLM interaction uses SSE streaming. The provider yields `Stream<Item = Result<StreamEvent, RustyError>>` which the agent loop consumes event-by-event.
2. **Callback-based UI**: The agent accepts optional callbacks (`TextCallback`, `ThinkingCallback`, `ToolCallback`) for real-time UI updates during streaming.
3. **Permission as value**: Permissions are data (`PermissionDecision`) flowing through a tiered check system, not booleans. The bash classifier examines command names and flags to determine read-only vs write.
4. **Sub-agents as tokio tasks**: The `agent` tool spawns a new `Agent` instance in a separate tokio task with BypassPermissions, using `SubAgentFn` callback.
5. **Auto-compaction**: Long conversations are automatically summarized (keeping last 10 messages) to stay within context limits (~80k tokens or 40 messages threshold).
6. **Session persistence**: Full message history saved to `~/.rusty/sessions/` as JSON, resumable via `--resume`.
7. **Path sandboxing**: File tools use `resolve_path()` to canonicalize and validate all paths stay within the working directory.
8. **Wire format conversion**: Internal `Message`/`ToolDefinition` types are converted to/from OpenAI wire format via helpers in `provider/types.rs`. Supports reasoning content (`reasoning_content` field for MiMo/DeepSeek models).

---

## Building & Running

```bash
# Build
cargo build --release

# Run with preset
cargo run -- --preset xiaomi --api-key YOUR_KEY

# Run with specific model
cargo run -- --model gpt-4o --api-key YOUR_KEY

# Non-interactive
cargo run -- --preset xiaomi --prompt "Explain this codebase"

# Headless stdin mode
cargo run -- --preset xiaomi --headless

# Resume session
cargo run -- --resume SESSION_ID

# List sessions
cargo run -- --list-sessions
```

---

## Testing

Tests are inline (in-module `#[cfg(test)]`). Run with:

```bash
cargo test --workspace
```

Notable test coverage:
- `crates/core/src/permissions.rs`: bash command classification (read-only vs write)
- `crates/provider/src/openai.rs`: SSE parsing, delta accumulation, stream completion, error handling

---

## Configuration

### Settings File (`~/.rusty/settings.json`)

```json
{
  "default_model": "mimo-v2.5-pro",
  "theme": "dark",
  "permanent_permissions": [
    "bash:git status",
    "bash:cargo check"
  ]
}
```

### Environment Variables

- `OPENAI_API_KEY` — API key (can also be passed via `--api-key`)
- `RUST_LOG` — Logging level (e.g., `debug`, `info`, `warn`)

### Presets

Presets define default configurations for different providers:

| Preset | API Base | Default Model | Notes |
|--------|----------|---------------|-------|
| `xiaomi` | `https://api.xiaomimimo.com/v1` | `mimo-v2.5-pro` | Xiaomi MiMo |
| `kimi` | `https://api.moonshot.cn/v1` | `moonshot-v1-8k` | Moonshot/Kimi |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` | OpenAI |
| `ollama` | `http://localhost:11434/v1` | `llama3` | Local Ollama |
| `deepseek` | `https://api.deepseek.com/v1` | `deepseek-chat` | DeepSeek |

---

## Error Handling

The codebase uses a custom `RustyError` enum with `thiserror` for structured error handling. Key error variants:

- `Api` / `ApiStatus` — LLM API errors
- `Auth` — Authentication failures
- `PermissionDenied` — Tool permission denied
- `RateLimit { retry_after }` — Rate limiting with optional retry-after seconds
- `ContextWindowExceeded` — Message history too long
- `MaxTokensReached` — Response hit token limit
- `Cancelled` — User cancelled operation

Errors are classified as retryable (`is_retryable()`) for automatic retry logic in the provider.

---

## Development Workflow

1. **Code changes**: Edit files in `crates/` subdirectories
2. **Check compilation**: `cargo check --workspace`
3. **Run tests**: `cargo test --workspace`
4. **Build release**: `cargo build --release`
5. **Test manually**: `./target/release/rusty --preset xiaomi --api-key YOUR_KEY`

### Adding a New Tool

1. Create `crates/tools/src/new_tool.rs`
2. Implement the `Tool` trait
3. Add to `all_tools()` in `crates/tools/src/lib.rs`
4. Update permission level in `permissions.rs` if needed

### Adding a New Provider

1. Create `crates/provider/src/new_provider.rs`
2. Implement the `LlmProvider` trait
3. Add preset configuration in `crates/cli/src/main.rs`

---

## Performance Notes

- **Streaming**: Responses stream in real-time via SSE
- **Async**: All I/O is async via Tokio
- **Memory**: Messages are cloned for each LLM call (consider Arc for large histories)
- **Rate limiting**: Built-in exponential backoff for API rate limits
- **Compaction**: Auto-summarization prevents context window overflow

---

## Security Considerations

- **Path sandboxing**: File operations are restricted to the working directory
- **Permission system**: Tiered permissions prevent unauthorized tool execution
- **API key handling**: Keys are never logged or persisted in session files
- **Command classification**: Bash commands are classified as read-only or write/execute

---

## Future Improvements

Potential areas for enhancement:

1. **Plugin system**: Dynamic tool loading
2. **Multi-model support**: Switch models mid-conversation
3. **Persistent context**: Long-term memory across sessions
4. **Collaborative editing**: Multiple agents working on same codebase
5. **Web UI**: Browser-based interface
6. **Mobile support**: Touch-friendly TUI
7. **Performance profiling**: Built-in metrics
8. **Custom themes**: User-configurable UI colors

---

## Troubleshooting

### Common Issues

1. **API key not found**: Set `OPENAI_API_KEY` environment variable or use `--api-key`
2. **Rate limiting**: Increase delay between requests or use a different API key
3. **Context window exceeded**: Use `--resume` to continue sessions or let auto-compaction handle it
4. **Permission denied**: Check permission mode (`--permissions`) and allowlist
5. **Build failures**: Ensure Rust toolchain is up-to-date (`rustup update`)

### Debug Logging

Enable debug logging with:

```bash
RUST_LOG=debug cargo run -- --preset xiaomi --api-key YOUR_KEY
```

### Session Recovery

Sessions are automatically saved to `~/.rusty/sessions/`. Resume with:

```bash
cargo run -- --resume SESSION_ID
```

---

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests for new functionality
5. Ensure all tests pass: `cargo test --workspace`
6. Submit a pull request

### Code Style

- Follow Rust naming conventions
- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Document public APIs with doc comments
- Keep functions focused and small

---

## License

MIT License — see `Cargo.toml` for details.

---

## Quick Reference

### Essential Commands

```bash
# Build and run
cargo build --release && ./target/release/rusty --preset xiaomi --api-key KEY

# Development
cargo check --workspace
cargo test --workspace
cargo fmt
cargo clippy

# Session management
./target/release/rusty --list-sessions
./target/release/rusty --resume SESSION_ID
```

### Key Files

- **Entry point**: `crates/cli/src/main.rs`
- **Agent loop**: `crates/agent/src/loop.rs`
- **Tool definitions**: `crates/tools/src/lib.rs`
- **Configuration**: `crates/core/src/config.rs`
- **Permissions**: `crates/core/src/permissions.rs`
- **TUI rendering**: `crates/tui/src/ui.rs`

### Data Flow

```
User Input → CLI → Agent Loop → LLM Provider → Streaming Response
    ↓                                              ↓
TUI Display ← Agent Events ← Tool Execution ← Tool Calls
```

---

*Last updated: May 2026*
*Codebase version: 0.1.0*
*Rust edition: 2021*
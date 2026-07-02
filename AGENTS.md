# AGENTS.md - Rusty Codebase Guide

## Overview

Rusty is a terminal-based AI coding agent written in Rust. It connects to OpenAI-compatible LLM APIs via SSE streaming, executes tools (file I/O, bash, search, patches, web fetch, sub-agents), and enforces a tiered permission system. It runs as a ratatui TUI, in headless mode, or in stdin REPL mode.

- Language: Rust (edition 2021)
- Async runtime: Tokio
- Default model: `mimo-v2.5-pro` (Xiaomi MiMo)
- Config directory: `~/.rusty/`
- License: AGPL-3.0-or-later

---

## Workspace Structure

```
rusty/
├── Cargo.toml              # Workspace root (6 crates)
├── crates/
│   ├── core/               # rusty-core: types, config, permissions, errors, credentials, setup wizard
│   ├── provider/           # rusty-provider: OpenAI-compatible HTTP/SSE streaming client
│   ├── tools/              # rusty-tools: all tool implementations
│   ├── agent/              # rusty-agent: agent loop, compaction, sub-agent spawning
│   ├── tui/                # rusty-tui: ratatui terminal UI (app state + rendering)
│   └── cli/                # rusty (binary): CLI args, main entry point, run modes
└── site/                   # Landing page (index.html)
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

### `crates/core` - Foundation Layer

| File | Purpose |
|---|---|
| `types.rs` | `Role` (User/Assistant), `ContentBlock` (Text/Image/ToolUse/ToolResult/Thinking), `Message`, `MessageContent` (Text/Blocks), `UsageInfo`, `ToolDefinition`. `ContentBlock::Image` carries `media_type` (MIME) and base64-encoded `data` for pasted images. Message helpers: `user()`, `assistant()`, `user_blocks()`, `assistant_blocks()`, `get_text()`, `get_all_text()`, `has_tool_use()`, `get_tool_use_blocks()`, `has_image_blocks()`, `get_image_blocks()`. |
| `config.rs` | `Config` (runtime config: model, api_key, api_base, max_tokens, temperature, permission_mode, thinking_budget, thinking_level, plan_with_tasks, auto_compact, system_prompt, append_system_prompt, no_claude_md, provider_type, context_window). `Settings` (persisted `~/.rusty/settings.json`: api_key, api_base, default_model, models, active_model, api_keys, allowed_tools, credential_store, thinking_level, permission_mode, permissions). `ModelEntry` struct: group, name, provider (ProviderType), api_base, model, available_models, max_tokens, temperature, thinking_budget, extra_headers, context_window. `ProviderType` enum: `OpenAI`, `Anthropic`. `CredentialStore` enum (Keyring / SettingsFile). `add_permanent_permission()`. |
| `credentials.rs` | `CredentialManager` - tiered API key resolution. Priority: (1) env vars `RUSTY_API_KEY` then `OPENAI_API_KEY`, (2) OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service), (3) settings file. Also provides `store_in_keyring()`, `delete_from_keyring()`, `is_keyring_available()`. |
| `setup_wizard.rs` | Interactive first-run wizard. Provider selection (Xiaomi, Kimi, OpenAI, DeepSeek, Ollama, Custom), API key entry with masked input, credential storage choice (keyring vs settings file), model selection, connectivity test. Plain terminal mode, no TUI dependency. |
| `permissions.rs` | `PermissionMode` (Default/AcceptEdits/Bypass/Plan), `PermissionLevel` (None/ReadOnly/Write/Execute), `PermissionRequest`/`PermissionDecision`, `PermissionChoice` (AllowOnce/AllowSession/AllowAlways/Deny) user-facing prompt options, bash command classifier (`classify_bash_command`), `build_tool_description`, `make_allow_key`. |
| `error.rs` | `RustyError` enum: Api, ApiStatus, Auth, PermissionDenied, Tool, Io, Json, Http, RateLimit (with retry_after), ContextWindowExceeded, MaxTokensReached, Cancelled, Config, Other. Helpers: `is_retryable()`, `is_context_limit()`. |
| `context.rs` | `build_system_context()` (platform info, working directory, git status, recent commits, sandbox notice). `build_user_context()` (discovers AGENTS.md/CLAUDE.md/RUSTY.md files walking up from working dir to root, plus `~/.rusty/` global files, includes current date). Context files capped at 30KB (`CONTEXT_FILES_MAX_BYTES`). Content sanitised: `<` and `>` escaped to fullwidth equivalents to prevent XML tag breakout from the `<environment_context>` wrapper. |
| `history.rs` | `ConversationSession` - save/load/list sessions in `~/.rusty/sessions/` as JSON files with id, messages, model, timestamps. `notes_path()` and `checkpoint_path()` helpers for sidecar files. |
| `cost.rs` | `CostTracker` - Thread-safe token usage tracking (input/output totals, total thinking tokens). Estimates cost from configurable per-token pricing. Available via `/cost` slash command. |
| `memory.rs` | `ProjectMemory` - per-project persistent memory storage in `~/.rusty/memory/{project_id}.json`. Project ID derived from git root slug. CRUD operations: save, search (substring), list, delete. 100-entry cap with FIFO eviction. Multi-layer content sanitisation: control character stripping, length truncation (2000 chars), XML tag neutralisation, regex injection pattern matching. |

### `crates/provider` - LLM API Client

| File | Purpose |
|---|---|
| `types.rs` | `StreamEvent` enum (TextDelta, ThinkingDelta, ToolCallDelta, Usage, Done, Error), `MessageRequest`, `ToolDefinition` (API-facing), `ProviderConfig`. OpenAI wire format types (`OaiRequest`, `OaiMessage`, `OaiTool`, `OaiToolCall`, `OaiResponse`, `OaiStreamChunk`) and conversion helpers (`rusty_messages_to_oai`, `rusty_tools_to_oai`, `oai_response_to_rusty`). |
| `openai.rs` | `OpenAiProvider` - OpenAI Chat Completions streaming implementation with SSE parsing, retry logic (exponential backoff for rate limits/529s), delta accumulation into `StreamEvent`s. |
| `anthropic.rs` | `AnthropicProvider` - Claude Messages API streaming implementation. Native Anthropic wire format (content blocks, `message_delta`, `message_stop` events). Supports tool use, thinking/reasoning blocks, and the `x-api-key` authentication header. |
| `mod.rs` | `LlmProvider` trait: `create_message_stream()`, `model()`, `messages()`. Factory function `create_provider()` selects the appropriate implementation based on `api_base` URL pattern. |

All LLM communication is streaming-first. The provider yields `Stream<Item = Result<StreamEvent, RustyError>>` which the agent loop consumes.

### `crates/tools` - Tool Implementations

| File | Tool | Permission | Description |
|---|---|---|---|
| `file_read.rs` | `file_read` | ReadOnly | Read file contents with optional offset/limit. Path sandbox. |
| `file_write.rs` | `file_write` | Write | Create/overwrite files, auto-creates parent dirs. Path sandbox. |
| `file_edit.rs` | `file_edit` | Write | Exact string match-and-replace editing. Path sandbox. |
| `apply_patch.rs` | `apply_patch` | Write | Apply unified diff patches. Supports `*** Begin Patch` / `*** End Patch` format with `*** Add File`, `*** Update File`, `*** Delete File` sections. Fuzzy matching for context lines (3-line search window). Uses `similar` crate for diff stats. Path sandbox. |
| `bash.rs` | `bash` | Classified per-command | Execute shell commands. Read-only commands (ls, git status, cargo check, etc.) bypass write permissions via `classify_bash_command`. Path sandboxing: blocks commands with absolute paths or redirect targets outside the working directory. |
| `grep.rs` | `grep` | ReadOnly | Regex search across files with glob filtering. Caps at 200 results. Skips binary extensions. |
| `glob.rs` | `glob` | ReadOnly | File pattern matching. |
| `web_fetch.rs` | `web_fetch` | ReadOnly | Fetch URLs via reqwest (30s timeout). Configurable max_length (default 10000 chars). Comprehensive SSRF protection (see Security section). |
| `todowrite.rs` | `todowrite` | None | Structured task list management. Accepts array of todo items with content, status (pending/in_progress/completed/cancelled), and priority (high/medium/low). Renders grouped by priority with status indicators. Persists across conversation. |
| `note.rs` | `note` | None | Session-scoped scratchpad for recording observations. Appends to `~/.rusty/sessions/{id}.notes.md`. Content is processed during checkpoint extraction and cleared after use. |
| `agent.rs` | `agent` | None | Spawn sub-agents via `SubAgentFn` callback. Accepts `task` (required) and `context` (optional). |
| `memory.rs` | `memory` | None | Per-project persistent memory. Actions: `save`, `search`, `list`, `delete`. Backed by `ProjectMemory` in core (`~/.rusty/memory/{project_id}.json`). Capped at 100 entries per project with FIFO eviction. Content sanitised on input and output to prevent prompt injection. |

**Tool trait** (`mod.rs`): `name()`, `description()`, `input_schema()`, `permission_level()`, `execute()`, `definition()`.

**`resolve_path(path_str, working_dir)`** in `mod.rs`: canonicalizes paths, resolves symlinks and `..`, rejects paths that escape the working directory. TOCTOU-hardened: avoids `path.exists()` before `canonicalize()` to prevent symlink races. Pre-write verification via `verify_not_escaping_symlink()`, post-write re-verification via `verify_no_symlink_escape()`. Used by all file tools and `apply_patch`.

**`all_tools()`** returns all built-in tools except `agent` and `memory` (both are wired separately by the CLI with custom callbacks).

### `crates/agent` - Agent Loop

| File | Purpose |
|---|---|
| `loop.rs` | `Agent` struct - core orchestrator. Holds provider, tools, config, message history, permission state. `run()` method: send messages, stream response, accumulate text + tool calls, execute tools concurrently via `JoinSet`, repeat until done/max turns/cancellation. Supports `cancel()` via `AtomicBool`. |
| `compact.rs` | Multi-tier auto-compaction: Tier 1 (25% context) micro-compacts old tool results, Tier 2 (50%) extracts structured checkpoint to `checkpoint.md`, Tier 3 (75%) summarizes old messages via LLM. Integrates with notes scratchpad. |
| `lib.rs` | `spawn_subagent()` (same-process tokio task, BypassPermissions), `make_agent_tool()` (constructs `AgentTool` with spawn callback), `build_system_prompt()` (assembles system prompt with tool descriptions, permissions, platform info, date, optional plan-with-tasks instructions). |

**Agent loop flow:**
1. Add user message to history
2. Maybe compact (summarize old messages)
3. Send to LLM with streaming
4. Accumulate text deltas and tool call deltas
5. If tool calls present: execute each tool, append results, loop back to step 2
6. If no tool calls: return final text

**Permission system** (tiered check in `check_permission_tiered`):
1. Bypass mode - allow all
2. Plan mode - read-only only
3. ReadOnly/None tools - auto-allow
4. AcceptEdits + Write - auto-allow
5. Permanent allowlist (from `~/.rusty/settings.json`)
6. Session allowlist (runtime)
7. Interactive callback (TUI prompt)
8. Default: deny

### `crates/tui` - Terminal UI

| File | Purpose |
|---|---|
| `app.rs` | `AppState` (input buffer, cursor, messages, streaming state, permission prompts, history navigation, scroll). `AgentEvent` enum (TextDelta, ThinkingDelta, ToolCallStart, ToolCallDelta, ToolResult, Done, Error). `SlashCommand` enum with tab-completion. Key handling with command history. |
| `ui.rs` | Ratatui rendering: chat area with markdown support (bold, italic, code blocks, tables with box-drawing), input box with cursor, status bar (model, permissions, token usage), permission prompt overlay. Color-coded roles. |
| `lib.rs` | Generic `run()` function (unused by CLI, which implements its own loop). |

**Slash commands** (available in both TUI and headless stdin modes):

| Command | Alias | Description |
|---|---|---|
| `/help` | `/h`, `/?` | List available commands |
| `/init` | | Generate AGENTS.md for the current codebase |
| `/resume` | `/r` | Resume a saved session (interactive picker in TUI) |
| `/sessions` | `/s` | List saved sessions |
| `/compact` | | Force conversation compaction |
| `/clear` | | Clear screen |
| `/copy` | `/c` | Copy last assistant response to clipboard |
| `/model` | `/m` | Show current model |
| `/rename` | | Rename current session |
| `/permissions` | `/perms` | View or revoke always-approved tool permissions |
| `/settings` | | Open model registry and general settings overlay (TUI only) |
| `/quit` | `/exit`, `/q` | Exit (saves session) |

### `crates/cli` - Binary Entry Point

`main.rs` handles:

1. **CLI args** (clap): `--prompt`, `--model`, `--preset`, `--api-key`, `--api-base`, `--cwd`, `--permissions`, `--plan-with-tasks`, `--resume`, `--list-sessions`, `--headless`, `--max-turns`, `--max-tokens`, `--temperature`, `--thinking-budget`, `--verbose`, `--setup`, `--no-claude-md`, `--append-system-prompt`

2. **First-run detection**: If `~/.rusty/settings.json` does not exist, the setup wizard launches automatically before any other logic. `--setup` forces the wizard explicitly.

3. **Credential resolution**: `CredentialManager::resolve_api_key(&settings)` handles the full chain (env vars, keyring, settings file). If no key is found and no subcommand was given, the wizard launches as a fallback.

4. **Config resolution**: preset defaults, then `~/.rusty/settings.json`, then CLI flags (later wins). Presets define `api_base` and `default_model`.

5. **Three run modes:**
   - **TUI mode** (default): full terminal UI with streaming, permission prompts, slash commands, session save on exit. Supports model registry picker via the sidebar.
   - **Headless mode** (`--prompt`): single prompt, print response, save session
   - **Stdin mode** (`--headless`): interactive line-by-line REPL with slash commands, no TUI

   `--no-claude-md` disables discovery of AGENTS.md/CLAUDE.md/RUSTY.md context files. `--append-system-prompt` appends text to the system prompt.

---

## Agent Task Management

When given multi-step work, follow this discipline:

### Phase 1: Plan (brief)

1. Create a task list with `todowrite`. Each task must be a concrete action (e.g. "Add X field to Y struct in Z.rs"), not a vague goal (e.g. "Improve error handling").
2. If the request is complex, use your thinking to research the codebase and identify what needs to change. This is where deep thinking belongs: at the planning stage, not during execution.
3. Keep the plan short. 3-7 tasks is typical. If the plan exceeds 10 tasks, break the request into phases.

### Phase 2: Execute (immediately)

4. After creating the task list, execute the FIRST task immediately. Do not narrate what you are about to do.
5. Mark each task `in_progress` before starting it, and `completed` the moment it is done. Never batch updates.
6. After completing a task, proceed to the next one without pausing. Do not stop to summarise progress or ask the user if you should continue.
7. If a task requires information you do not have, gather it (read a file, grep the codebase) as part of executing the task, then continue.
8. If you discover a new task while executing, add it to the list and keep going.
9. If you are blocked (e.g. permission denied, external dependency), mark the task `cancelled` with a reason, and proceed to the next task.

### Phase 3: Verify

10. After all tasks are done, run `cargo check`, tests, or other verification if applicable. "Done" means verified.

### Phase 4: Review

11. Re-read the original request. Go through each completed task and verify it was done correctly and completely.
12. Check for: missed requirements, inconsistencies between tasks, files that should have been updated but weren't, and anything that contradicts the original request.
13. If you find gaps, add new tasks to the list and execute them.

### Rules

- **Planning is not execution.** A task list without tool calls is incomplete work.
- **Never stop with incomplete tasks** unless you are genuinely blocked. The user should not need to prompt you to continue.
- **Do not ask the user if you should continue.** You should always continue until all tasks are `completed` or `cancelled`.
- **Always review before finishing.** Do not conclude your response until you have checked your work against the original request.

---

## Key Architectural Patterns

1. **Streaming-first**: All LLM interaction uses SSE streaming. The provider yields `Stream<Item = Result<StreamEvent, RustyError>>` consumed event-by-event by the agent loop.

2. **Callback-based UI**: The agent accepts optional callbacks (`TextCallback`, `ThinkingCallback`, `ToolCallback`, `PermissionCallback`) for real-time UI updates during streaming.

3. **Permission as data**: Permissions are `PermissionDecision` values flowing through a tiered check system. The bash classifier examines command names and flags to determine read-only vs write/execute.

4. **Sub-agents as tokio tasks**: The `agent` tool spawns a new `Agent` instance in a separate tokio task with BypassPermissions. Sub-agents get all tools except the agent tool (prevents recursive spawning).

5. **Concurrent tool execution**: Multiple tool calls within a single LLM response are executed concurrently via `tokio::JoinSet`. Each tool call runs as an independent spawned task, with results collected and returned in call-order once all complete. This significantly reduces wall-clock time when the model issues several independent calls.

6. **Auto-compaction**: Long conversations are automatically managed via a three-tier system: Tier 1 (25% context) replaces old tool results with placeholders, Tier 2 (50%) extracts structured checkpoint to `checkpoint.md` using an LLM call, Tier 3 (75%) summarizes old messages (keeping last 10). Notes scratchpad content is incorporated into checkpoints and summaries.

7. **Notes scratchpad**: The `note` tool writes observations to a session-scoped `notes.md` file. Content is processed during checkpoint extraction and cleared, providing a low-friction way to persist context that would otherwise be lost during compaction.

8. **Session persistence**: Full message history saved to `~/.rusty/sessions/` as JSON, resumable via `--resume` or `/resume`. Sidecar files: `{id}.notes.md` (scratchpad) and `{id}.checkpoint.md` (structured state) are cleaned up with the session.

8. **Path sandboxing**: File tools use `resolve_path()` to canonicalize and validate all paths stay within the working directory. Bash tool uses `check_bash_paths()` to block commands with path arguments or redirect targets outside the working directory.

9. **Wire format conversion**: Internal `Message`/`ToolDefinition` types are converted to/from OpenAI wire format via helpers in `provider/types.rs`. Supports reasoning content (`reasoning_content` field for MiMo/DeepSeek models).

10. **Multi-provider support**: The `LlmProvider` trait abstracts LLM communication. Built-in providers include `OpenAiProvider` (OpenAI-compatible Chat Completions with SSE) and `AnthropicProvider` (Claude Messages API with Anthropic-native wire format). Both support streaming, retry logic, and tool calling.

11. **Tiered credential management**: `CredentialManager` resolves API keys through env vars, OS keyring, and settings file. The setup wizard can store keys in the OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service) or in the settings file.

---

## Configuration

### Settings File (`~/.rusty/settings.json`)

```json
{
  "api_key": "sk-...",
  "api_base": "https://api.xiaomimimo.com/v1",
  "default_model": "mimo-v2.5-pro",
  "models": [
    {
      "group": "Xiaomi",
      "name": "MiMo Pro",
      "provider": "OpenAI",
      "api_base": "https://api.xiaomimimo.com/v1",
      "model": "mimo-v2.5-pro",
      "available_models": ["mimo-v2.5-pro", "mimo-v2-flash"],
      "context_window": 131072,
      "thinking_budget": 4096
    },
    {
      "group": "Anthropic",
      "name": "Claude Sonnet",
      "provider": "Anthropic",
      "api_base": "https://api.anthropic.com",
      "model": "claude-sonnet-4-20250514",
      "context_window": 200000
    }
  ],
  "active_model": "mimo-v2.5-pro",
  "api_keys": {},
  "allowed_tools": [
    "bash:git status",
    "bash:cargo check"
  ],
  "credential_store": "keyring",
  "thinking_level": "normal",
  "permission_mode": "default",
  "permissions": {}
}
```

### Model Registry

The `models` array in `settings.json` defines available LLM models. Each `ModelEntry` has:

| Field | Type | Description |
|---|---|---|
| `group` | `String` | Display grouping (e.g. "Xiaomi", "Anthropic") |
| `name` | `String` | Human-readable name |
| `provider` | `ProviderType` | `OpenAI` or `Anthropic` (determines wire format) |
| `api_base` | `String` | API endpoint URL |
| `model` | `String` | Model ID sent to the API |
| `available_models` | `Vec<String>` | List of models to offer in the picker |
| `context_window` | `Option<usize>` | Context window size in tokens |
| `thinking_budget` | `Option<u32>` | Token budget for reasoning/thinking |
| `max_tokens` | `Option<u32>` | Max output tokens |
| `temperature` | `Option<f32>` | Sampling temperature |
| `extra_headers` | `Option<HashMap>` | Additional HTTP headers |

The `active_model` field selects which model entry is used. Per-model API keys can be stored in `api_keys` (keyed by model ID). The TUI `/settings` command provides a visual editor for the model registry.

Note: `ProviderType::Anthropic` is defined and deserialisable but the `AnthropicProvider` implementation is not yet complete. Use `OpenAI`-compatible endpoints for all currently supported providers.

### Thinking Levels

The `ThinkingLevel` enum controls the token budget allocated to reasoning/thinking:

| Level | Token Budget | Use Case |
|---|---|---|
| `minimal` | 1024 | Simple queries, quick responses |
| `normal` | 4096 | Standard multi-step tasks |
| `deep` | 16384 | Complex reasoning, architecture decisions |

**Dynamic adjustment** (`dynamic_thinking_level()`): The agent automatically adjusts the thinking level based on context:
- Multi-step tasks (2+ tool turns) are boosted from Minimal to Normal.
- Context usage above 70% triggers a one-level step-down.
- Context usage above 85% forces Minimal regardless of setting.

Set via `--thinking-budget` CLI flag, `thinking_level` in config/settings, or the TUI `/settings` General tab.

### Environment Variables

| Variable | Purpose |
|---|---|
| `OPENAI_API_KEY` | API key (clap `env` binding) |
| `RUSTY_API_KEY` | Alternative API key (higher priority than `OPENAI_API_KEY` in `CredentialManager`) |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | Logging level (`debug`, `info`, `warn`) |

### Credential Resolution Order

`CredentialManager::resolve_api_key()` checks in this order:
1. `RUSTY_API_KEY` env var
2. `OPENAI_API_KEY` env var
3. OS keyring (if `credential_store` is `Keyring`)
4. `api_key` field in `~/.rusty/settings.json`

Empty strings are treated as absent at every stage.

### Presets

| Preset | API Base | Default Model |
|---|---|---|
| `xiaomi` | `https://token-plan-cn.xiaomimimo.com/v1` | `mimo-v2.5-pro` |
| `kimi` | `https://api.kimi.com/coding/v1/` | `kimi-k2.6` |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` |
| `deepseek` | `https://api.deepseek.com` | `deepseek-v4-pro` |
| `ollama` | `http://localhost:11434/v1` | `llama3` |

Note: The setup wizard uses slightly different default models for some providers (e.g., `kimi-k2`, `gpt-4.1`, `qwen3:8b` for Ollama). CLI presets take precedence when `--preset` is used.

---

## Error Handling

`RustyError` enum with `thiserror`:

| Variant | Meaning |
|---|---|
| `Api` / `ApiStatus` | LLM API errors |
| `Auth` | Authentication failures |
| `PermissionDenied` | Tool permission denied |
| `RateLimit { retry_after }` | Rate limiting with optional retry-after seconds |
| `ContextWindowExceeded` | Message history too long |
| `MaxTokensReached` | Response hit token limit |
| `Cancelled` | User cancelled operation |
| `Config` | Configuration errors (including keyring failures) |

Errors are classified as retryable (`is_retryable()`) for automatic retry logic in the provider.

---

## Security

### SSRF Protection (`web_fetch`)

The `web_fetch` tool defends against server-side request forgery with multiple layers:

- **Scheme restriction**: Only `http` and `https` schemes allowed. Blocks `file://`, `ftp://`, etc.
- **Hostname blocklist**: Rejects `localhost`, `*.localhost`, `*.local`, and variants.
- **IP blocklist**: Blocks loopback (127.0.0.0/8, ::1), private (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, fc00::/7), link-local (169.254.0.0/16, fe80::/10), CGNAT (100.64.0.0/10), TEST-NET ranges, and multicast addresses.
- **DNS rebinding prevention**: Resolves the hostname, pins the resolved IP, and rejects requests where DNS resolves to a private/blocklisted address. Builds a per-request client with `resolve()` override.
- **Redirect validation**: Every redirect target is re-checked against the full blocklist before following.

### Path Sandboxing (`resolve_path`)

All file tools use `resolve_path()` which:

- Canonicalizes paths and resolves symlinks and `..` components.
- Rejects any path that escapes the working directory.
- TOCTOU-hardened: avoids `path.exists()` before `canonicalize()` to prevent symlink race conditions.
- Pre-write verification (`verify_not_escaping_symlink()`) and post-write re-verification (`verify_no_symlink_escape()`) guard against symlink attacks during file creation.

### Bash Path Sandboxing (`check_bash_paths`)

The bash tool performs lightweight path checking before executing commands:

- **Path token extraction**: Parses the command for path-like tokens (absolute paths, `./`, `../`, `~` expansions).
- **Redirect target validation**: Extracts and validates redirect targets (`>`, `>>`, `2>`) against the working directory.
- **Boundary enforcement**: Rejects commands where any path token or redirect target resolves outside the working directory.
- **Limitations**: Does not catch paths constructed via shell variables (`$VAR`), subshells, or commands that `cd` internally. Complex pipelines may have reduced coverage.

### Prompt Injection Defences

- **Memory tool**: Multi-layer sanitisation in `sanitize_content()` on both input and output. Strips control characters, truncates to 2000 chars, neutralises XML tags, and blocks regex injection patterns.
- **Context files**: `<` and `>` in discovered context files escaped to fullwidth equivalents to prevent breakout from the `<environment_context>` wrapper. 30KB byte budget prevents context flooding.
- **TUI paste sanitisation**: Strips ANSI escape sequences (CSI, OSC, DCS), C0/C1 control characters (except `\n`, `\t`), Unicode bidi override characters (U+202A-U+202E, U+2066-U+2069), and zero-width characters. 100KB max paste length.

### File Permission Hardening

- Config directories (`~/.rusty/`) created with `0o700` permissions via `ensure_restricted_dir()`.
- Settings and memory files created with `0o600` permissions via `set_restrictive_file_permissions()`.

### Credential Security

- API keys resolved through a tiered chain (env vars, OS keyring, settings file).
- OS keyring integration (macOS Keychain, Windows Credential Manager, Linux Secret Service) avoids storing secrets on disk.
- Setup wizard offers keyring vs settings file choice with clear trade-off explanation.

---

## Building and Running

```bash
# Build
cargo build --release

# Run (auto-launches setup wizard on first run if ~/.rusty/settings.json is missing)
./target/release/rusty

# Run with preset
./target/release/rusty --preset xiaomi --api-key YOUR_KEY

# Explicit setup wizard
./target/release/rusty --setup

# Non-interactive
./target/release/rusty --preset xiaomi --prompt "Explain this codebase"

# Headless stdin mode
./target/release/rusty --preset xiaomi --headless

# Plan mode with task tracking
./target/release/rusty --plan-with-tasks

# Resume session
./target/release/rusty --resume SESSION_ID

# List sessions
./target/release/rusty --list-sessions
```

### Development

```bash
cargo check --workspace
cargo test --workspace
cargo fmt
cargo clippy
```

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

## Data Flow

```
User Input → CLI → Agent Loop → LLM Provider → Streaming Response
    ↓                                              ↓
TUI Display ← Agent Events ← Tool Execution ← Tool Calls
```

---

## Key Files

| Purpose | Path |
|---|---|
| Entry point | `crates/cli/src/main.rs` |
| Agent loop | `crates/agent/src/loop.rs` |
| Tool definitions | `crates/tools/src/lib.rs` |
| Configuration | `crates/core/src/config.rs` |
| Credentials | `crates/core/src/credentials.rs` |
| Setup wizard | `crates/core/src/setup_wizard.rs` |
| Permissions | `crates/core/src/permissions.rs` |
| TUI rendering | `crates/tui/src/ui.rs` |
| TUI state | `crates/tui/src/app.rs` |
| Provider | `crates/provider/src/openai.rs` |

---

*Last updated: June 2026*
*Version: 0.2.1*
*Rust edition: 2021*
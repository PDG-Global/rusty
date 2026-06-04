# Changelog

## v0.1.5 (2026-06-04)

### Features

- **Persistent project memory**: Cross-session memory storage scoped to each project. Memories are injected into the system prompt at startup and managed via a `memory` tool with store/get/list/delete actions.
- **Model registry with Anthropic support**: Hierarchical model entries with provider type routing. Kimi is now routed through the Anthropic Messages API instead of OpenAI. Includes per-model `extra_headers` wired through to the HTTP client.
- **Cached token tracking**: `UsageInfo` and `CostTracker` now track cached tokens from both OpenAI (`prompt_tokens_details.cached_tokens`) and DeepSeek (`prompt_cache_hit_tokens`) wire formats. Cached tokens display in the TUI status bar when non-zero.
- **Concurrent tool execution**: Multiple tool calls in a single LLM response now execute in parallel via `tokio::spawn` + `join_all`, instead of sequentially. Permission checks still run sequentially to handle interactive prompts.
- **User-Agent header**: All HTTP clients now send `Rusty/{version}` as User-Agent.
- **Immediate task enforcement**: System prompt now enforces immediate task execution with a post-completion review phase.

### Security

- **Prompt injection protection**: Memory system sanitises content on both write and read paths, stripping control characters, role impersonation, instruction override attempts, special token delimiters, XML injection tags, and markdown headers. Context file content is escaped to prevent XML/tag breakout via malicious AGENTS.md files.
- **Restrictive file permissions**: Config directory (0o700) and log files (0o600) now have restrictive permissions via `ensure_restricted_dir()` and `set_restrictive_file_permissions()`.
- **Session TTL**: Sessions older than 30 days are automatically cleaned up on startup and session list.
- **SSRF protection**: `web_fetch` now blocks requests to private/reserved IPs, enforces redirect limits, and validates content-type.
- **Sub-agent permission fix**: Sub-agents now inherit the parent permission mode instead of always using BypassPermissions. Default mode is promoted to AcceptEdits so sub-agents never block for interactive input.

### Fixes

- **`/init` prompt grounding**: Rewrote the `/init` prompt to require the LLM to explore the repository with tools before generating AGENTS.md, preventing hallucinated content.

## v0.1.2 (2026-06-02)

### Features

- **Writing style rules**: System prompt now enforces consistent writing style: no emojis, British English spelling, no em-dashes, clear and concise phrasing, match existing tone.
- **Bracketed paste support**: Terminal events are now drained in a batch loop with dedicated paste_mode, preventing mid-paste message sends in the TUI.

### Fixes

- **Token usage tracking**: Input tokens no longer double-count across conversation turns. The API's prompt_tokens value is now used as the authoritative context size.
- **Token estimation**: Auto-compaction token estimation now includes system prompt and tool definition overhead, preventing delayed compaction in tool-heavy conversations.

### Removed

- Removed stale `build-release.sh` script.

## v0.1.1 (2026-06-02)

### Features

- **Model registry**: Hierarchical model entries with groups (Xiaomi, Kimi, DeepSeek, etc.), per-model API keys, and multiple available models per provider. Registry takes priority over legacy flat settings for model selection.
- **Provider factory**: `create_provider()` maps `ProviderType` to concrete `LlmProvider` implementations. Currently all providers use the OpenAI-compatible protocol.
- **Settings TUI**: `/settings` slash command opens a tabbed overlay with Models and General tabs for browsing, switching models, and adjusting runtime settings in-session.
- **Task tracking is now default behavior**: The todowrite system prompt instructions and incomplete-task nudging are active in all modes. `--plan-with-tasks` is now a convenience alias for `--permissions plan`.
- **CancelToken upgrade**: Replaced `AtomicBool` polling with `tokio::Notify` for immediate async cancellation without busy-waiting.

### Fixes

- **Dynamic thinking thresholds**: Raised first step-down threshold from 50% to 70% context usage and force-minimal threshold from 75% to 85%. Thinking no longer drops to Minimal prematurely during long-running tool-heavy tasks.
- **Task complexity boost**: Multi-step tasks (turn >= 2) automatically boost thinking from Minimal to Normal before context pressure logic applies, maintaining reasoning quality during complex workflows.
- **DeepSeek preset**: API base corrected to `https://api.deepseek.com` (removed trailing `/v1`), default model updated to `deepseek-v4-pro`.
- **Setup wizard**: Improved first-run experience with auto-launch when no config exists.

## v0.1.0 (2026-05-28)

Initial release.

- SSE streaming LLM client (OpenAI-compatible)
- Tool suite: file_read, file_write, file_edit, apply_patch, bash, grep, glob, web_fetch, todowrite, sub-agent
- Tiered permission system (Default, AcceptEdits, Bypass, Plan)
- Bash command classifier for automatic read-only detection
- Auto-compaction for long conversations
- Session persistence and resume
- Ratatui TUI with markdown rendering, permission prompts, slash commands
- Headless and stdin REPL modes
- Setup wizard with OS keyring credential storage
- Cross-platform builds (Linux x86_64/aarch64, macOS x86_64/aarch64/universal, Windows x86_64/aarch64)

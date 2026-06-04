# Changelog

## v0.1.5 (2026-06-03)

### Fixes

- **`/init` prompt grounding**: Rewrote the `/init` prompt to require the LLM to explore the repository with tools before generating AGENTS.md. Previously the prompt prescribed a fixed 12-section template, causing the LLM to hallucinate content to fill sections that had no basis in the actual repo.

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

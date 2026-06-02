# Rusty

A lightweight AI coding agent that runs in your terminal. Connects to OpenAI-compatible LLM APIs, streams responses, and executes tools like file editing, bash commands, and web search.

## Features

- Streaming LLM responses with real-time terminal UI
- File read/write/edit operations with path sandboxing
- Unified diff patch application (apply_patch tool)
- Bash command execution with read-only/write classification
- Regex search and glob file matching
- Web page fetching
- Structured task list management (todowrite tool)
- Sub-agent spawning for complex tasks
- Auto-compaction for long conversations
- Session persistence and resume
- Tiered permission system (bypass, accept-edits, plan, default)
- Interactive first-run setup wizard
- API key management with OS keyring support (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- Multiple API presets (Xiaomi MiMo, Kimi/Moonshot, OpenAI, Ollama, DeepSeek)

## Installation

```bash
cargo install --path crates/cli
```

Or build from source:

```bash
cargo build --release
```

The binary will be at `target/release/rusty`.

## Quick Start

On first run, rusty launches an interactive setup wizard that guides you through selecting a provider, entering your API key, and choosing where to store it:

```bash
rusty
```

To re-run the setup wizard at any time:

```bash
rusty --setup
```

## Usage

### Interactive TUI mode

```bash
# With Xiaomi MiMo
rusty --preset xiaomi --api-key YOUR_KEY

# With OpenAI
rusty --preset openai --api-key YOUR_KEY

# With local Ollama
rusty --preset ollama
```

### Non-interactive mode

```bash
rusty --preset xiaomi --api-key YOUR_KEY --prompt "Explain this codebase"
```

### Headless stdin mode

```bash
rusty --preset xiaomi --api-key YOUR_KEY --headless
```

### Plan mode with task tracking

```bash
rusty --plan-with-tasks
```

This enables read-only permissions. Task tracking via the todowrite tool is always active; this flag is a convenience alias for `--permissions plan`.

### Resume a session

```bash
rusty --resume SESSION_ID
```

### List sessions

```bash
rusty --list-sessions
```

## Configuration

### API Key Resolution

API keys are resolved in this order:

1. `RUSTY_API_KEY` environment variable
2. `OPENAI_API_KEY` environment variable
3. OS keyring (if `credential_store` is `keyring` in settings)
4. `api_key` field in `~/.rusty/settings.json`

The setup wizard can store your key in the OS keyring or in the settings file.

### Settings File

`~/.rusty/settings.json`:

```json
{
  "api_key": "your_key",
  "api_base": "https://token-plan-cn.xiaomimimo.com/v1",
  "default_model": "mimo-v2.5-pro",
  "credential_store": "keyring",
  "allowed_tools": ["bash:git status", "bash:cargo check"]
}
```

`credential_store` can be `"keyring"` (default, stores API key in OS keyring) or `"settings_file"` (stores in this JSON file).

### Environment Variables

| Variable | Purpose |
|---|---|
| `RUSTY_API_KEY` | API key (highest priority) |
| `OPENAI_API_KEY` | API key (clap default) |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | Logging level (`debug`, `info`, `warn`) |

### Presets

| Preset | API Base | Default Model |
|--------|----------|---------------|
| `xiaomi` | `https://token-plan-cn.xiaomimimo.com/v1` | `mimo-v2.5-pro` |
| `kimi` | `https://api.moonshot.cn/v1` | `kimi-k2.6` |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` |
| `deepseek` | `https://api.deepseek.com` | `deepseek-v4-pro` |
| `ollama` | `http://localhost:11434/v1` | `llama3` |

## CLI Options

```
-p, --prompt <TEXT>           Initial prompt (non-interactive mode)
-m, --model <MODEL>          Model to use
    --api-base <URL>         API base URL
    --preset <PRESET>        API provider preset
    --api-key <KEY>          API key
    --cwd <DIR>              Working directory
    --permissions <MODE>     Permission mode (default/accept-edits/bypass/plan)
    --plan-with-tasks        Plan mode with task tracking (implies --permissions plan)
    --setup                  Run interactive setup wizard
    --resume <ID>            Resume session by ID
    --list-sessions          List saved sessions
-v, --verbose                Verbose output
    --max-turns <N>          Max turns before stopping
    --max-tokens <N>         Max tokens per response
    --temperature <F>        Temperature
    --thinking-budget <N>    Reasoning token budget
    --headless               No TUI, just print responses
```

## Permission Modes

- **default**: Prompt for write/execute operations
- **accept-edits**: Auto-allow file writes, prompt for bash
- **bypass**: Allow everything without prompting
- **plan**: Read-only mode, no writes or execution

## Slash Commands

Available in TUI and headless stdin modes:

| Command | Alias | Description |
|---|---|---|
| `/help` | `/h`, `/?` | List commands |
| `/init` | | Generate AGENTS.md for the codebase |
| `/resume` | `/r` | Resume a saved session |
| `/sessions` | `/s` | List saved sessions |
| `/compact` | | Force conversation compaction |
| `/clear` | | Clear screen |
| `/copy` | `/c` | Copy last response to clipboard |
| `/model` | `/m` | Show current model |
| `/rename` | | Rename current session |
| `/permissions` | `/perms` | Manage always-approved tools list |
| `/settings` | | Open settings/model registry TUI |
| `/quit` | `/exit`, `/q` | Exit (saves session) |

## Tools

The agent has access to these built-in tools:

| Tool | Description |
|---|---|
| `file_read` | Read file contents with optional offset/limit |
| `file_write` | Create or overwrite files |
| `file_edit` | Exact string match-and-replace editing |
| `apply_patch` | Apply unified diff patches to files |
| `bash` | Execute shell commands |
| `grep` | Regex search across files |
| `glob` | File pattern matching |
| `web_fetch` | Fetch web page content |
| `todowrite` | Manage a structured task list |
| `agent` | Spawn a sub-agent for complex subtasks |

## Architecture

```
crates/
├── core/       Foundation: types, config, permissions, errors, credentials, setup wizard
├── provider/   OpenAI-compatible HTTP/SSE streaming client
├── tools/      File I/O, bash, search, web fetch, patches, task list, sub-agents
├── agent/      Agent loop, compaction, sub-agent spawning
├── tui/        ratatui-based terminal UI
└── cli/        Binary entry point, CLI args, run modes
```

See [AGENTS.md](AGENTS.md) for detailed architecture documentation.

## Development

```bash
# Check compilation
cargo check --workspace

# Run tests
cargo test --workspace

# Build release
cargo build --release

# Format code
cargo fmt

# Lint
cargo clippy
```

## License

Copyright (C) 2026 PDG Global Limited

SPDX-License-Identifier: AGPL-3.0-or-later

This project is licensed under the GNU Affero General Public License v3.0 or later.
See [LICENSE](LICENSE) for the full license text.

# Rusty

A lightweight AI coding agent that runs in your terminal. Connects to OpenAI-compatible LLM APIs, streams responses, and executes tools like file editing, bash commands, and web search.

## Features

- Streaming LLM responses with real-time terminal UI
- File read/write/edit operations with path sandboxing
- Bash command execution with read-only/write classification
- Regex search and glob file matching
- Web page fetching
- Sub-agent spawning for complex tasks
- Auto-compaction for long conversations
- Session persistence and resume
- Tiered permission system (bypass, accept-edits, plan, default)
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

### Resume a session

```bash
rusty --resume SESSION_ID
```

### List sessions

```bash
rusty --list-sessions
```

## Configuration

### API Key

Set via environment variable, CLI flag, or config file:

```bash
# Environment variable
export OPENAI_API_KEY=your_key

# CLI flag
rusty --api-key your_key

# Config file (~/.rusty/settings.json)
{
  "api_key": "your_key"
}
```

### Settings File

Create `~/.rusty/settings.json`:

```json
{
  "api_key": "your_key",
  "api_base": "https://api.openai.com/v1",
  "default_model": "gpt-4o",
  "allowed_tools": ["bash:git status", "bash:cargo check"]
}
```

### Presets

| Preset | API Base | Default Model |
|--------|----------|---------------|
| `xiaomi` | `https://api.xiaomimimo.com/v1` | `mimo-v2.5-pro` |
| `kimi` | `https://api.moonshot.cn/v1` | `kimi-k2.6` |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` |
| `deepseek` | `https://api.deepseek.com/v1` | `deepseek-chat` |
| `ollama` | `http://localhost:11434/v1` | `llama3` |

## CLI Options

```
-p, --prompt <TEXT>        Initial prompt (non-interactive mode)
-m, --model <MODEL>        Model to use
    --api-base <URL>       API base URL
    --preset <PRESET>      API provider preset
    --api-key <KEY>        API key
    --cwd <DIR>            Working directory
    --permissions <MODE>   Permission mode (default/accept-edits/bypass/plan)
    --resume <ID>          Resume session by ID
    --list-sessions        List saved sessions
-v, --verbose              Verbose output
    --max-turns <N>        Max turns before stopping
    --max-tokens <N>       Max tokens per response
    --temperature <F>      Temperature
    --headless             No TUI, just print responses
```

## Permission Modes

- **default**: Prompt for write/execute operations
- **accept-edits**: Auto-allow file writes, prompt for bash
- **bypass**: Allow everything without prompting
- **plan**: Read-only mode, no writes or execution

## Architecture

```
crates/
├── core/       Foundation: types, config, permissions, errors, history
├── provider/   OpenAI-compatible HTTP/SSE streaming client
├── tools/      File I/O, bash, search, web fetch, sub-agents
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

MIT
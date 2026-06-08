---
title: Rusty
description: A lightweight, statically compiled terminal AI coding agent
---

# Rusty

**A lightweight, statically compiled terminal AI coding agent.** Connects to OpenAI-compatible LLM APIs via SSE streaming, executes tools (file I/O, bash, search, patches, web fetch, sub-agents), and enforces a tiered permission system.

## Features

<div class="grid cards" markdown>

- :material-console:{ .lg .middle } **Terminal UI**

    ---

    Full ratatui-based terminal interface with streaming, markdown rendering, and interactive permission prompts.

- :material-connection:{ .lg .middle } **Multi-Provider**

    ---

    Works with Xiaomi MiMo, Kimi, OpenAI, DeepSeek, Ollama, and any OpenAI-compatible API.

- :material-wrench:{ .lg .middle } **Tool Execution**

    ---

    File read/write/edit, bash commands, regex search, glob, web fetch, unified diff patches, and sub-agent spawning.

- :material-shield:{ .lg .middle } **Permission System**

    ---

    Tiered permission model: Bypass, AcceptEdits, Default, and Plan modes with per-tool classification.

</div>

## Quick Start

```bash
# Install and run (launches setup wizard on first run)
cargo install rusty
rusty

# Or build from source
git clone https://github.com/pdg-global/rusty.git
cd rusty
cargo build --release
./target/release/rusty --preset openai --api-key YOUR_KEY
```

## Run Modes

=== "TUI Mode"

    Full terminal UI with streaming, permission prompts, and slash commands (default).

    ```bash
    rusty
    ```

=== "Headless Mode"

    Single prompt, print response, save session.

    ```bash
    rusty --prompt "Explain this codebase"
    ```

=== "Stdin REPL"

    Interactive line-by-line REPL without TUI.

    ```bash
    rusty --headless
    ```

## Architecture

Rusty is a Cargo workspace with 6 crates:

| Crate | Purpose |
|-------|---------|
| `rusty-core` | Types, config, permissions, errors, credentials, setup wizard |
| `rusty-provider` | OpenAI-compatible HTTP/SSE streaming client |
| `rusty-tools` | All tool implementations |
| `rusty-agent` | Agent loop, compaction, sub-agent spawning |
| `rusty-tui` | Ratatui terminal UI |
| `rusty` (cli) | Binary entry point, CLI args, run modes |

## License

AGPL-3.0-or-later

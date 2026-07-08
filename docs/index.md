---
title: Introduction
---

# Rusty

A coding agent that never leaves your terminal. Rusty is a single 12&nbsp;MB
binary that pairs with you right where you work — fast, private, and entirely
yours. No telemetry. No cloud lock-in. No bloat.

```bash title="terminal"
curl -fsSL rustycli.com/install | sh
```

!!! note

    Rusty verifies the download with a checksum before it runs. Prefer to
    inspect first? Pipe the script to a file and read it — it's ~40 lines of
    plain shell.

## What you get

- **Context-aware edits** across your whole tree — file read/write/edit, unified diff patches
- **A sandbox with approvals** — tiered permission model with per-tool classification
- **Readable diffs** before any change lands
- **Bring your own model** — Xiaomi MiMo, Kimi, OpenAI, DeepSeek, Ollama, or any OpenAI-compatible API
- **Built-in tools** — bash, regex search, glob, web fetch, sub-agent spawning, task management
- **Auto-compaction** — three-tier system that keeps long conversations under the context window
- **Session persistence** — save, resume, and name sessions across restarts

## Run modes

=== "TUI Mode"

    Full terminal UI with streaming, markdown rendering, permission prompts, and slash commands. This is the default.

    ```bash
    rusty
    ```

=== "Headless Mode"

    Single prompt, print response, save session. Ideal for scripting.

    ```bash
    rusty --prompt "Explain this codebase"
    ```

=== "Stdin REPL"

    Interactive line-by-line REPL without the TUI. Supports slash commands.

    ```bash
    rusty --headless
    ```

## Where to next

<div class="grid cards" markdown>

- **[Installation](getting-started/installation.md)** — get the binary on your PATH
- **[Quickstart](getting-started/quickstart.md)** — ship your first change
- **[Settings](configuration/settings.md)** — configure models, keys, and preferences
- **[Permissions](configuration/permissions.md)** — how Rusty stays safe

</div>

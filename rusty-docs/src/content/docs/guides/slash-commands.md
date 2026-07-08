---
title: Slash Commands
description: Built-in commands for controlling Rusty during a session
---


## Overview

Slash commands are special commands you can type in the input box (both TUI and stdin REPL modes) to control Rusty without sending a message to the LLM. Type `/` to see available commands with tab completion.

## Command Reference

| Command | Aliases | Description |
|---------|---------|-------------|
| `/help` | `/h`, `/?` | List all available commands |
| `/init` | | Generate an AGENTS.md file for the current codebase |
| `/resume` | `/r` | Resume a saved session (interactive picker in TUI mode) |
| `/sessions` | `/s` | List all saved sessions |
| `/compact` | | Force conversation compaction |
| `/clear` | | Clear the screen |
| `/copy` | `/c` | Copy the last assistant response to the clipboard |
| `/model` | `/m` | Show the current model |
| `/rename` | | Rename the current session |
| `/permissions` | `/perms` | View or revoke always-approved tool permissions |
| `/settings` | | Open the model registry and general settings overlay (TUI only) |
| `/cost` | | Show token usage and estimated cost |
| `/quit` | `/exit`, `/q` | Exit (saves session) |

## Command Details

### /compact

Forces conversation compaction. Normally, compaction triggers automatically at 25%, 50%, and 75% of context window usage. Use this command to compact manually when you want to free up context space before hitting the automatic thresholds.

### /copy

Copies the last assistant response to the system clipboard. Useful for pasting code suggestions into your editor.

### /model

Displays the currently active model name, provider, and API base URL.

### /permissions

Displays the current permission state:

- **Session allowlist**: Tools approved for this session only
- **Permanent allowlist**: Tools approved permanently (persisted in `~/.rusty/settings.json`)

You can also revoke specific always-approved permissions from this view.

Permanent allowlist entries use the format `tool_name:exact_input`. For example:

```
bash:git status
bash:cargo check
file_read:src/main.rs
```

### /settings

Opens the settings overlay (TUI mode only). This provides a visual editor for:

- **Model registry**: Browse and switch between configured models, edit model entries
- **General settings**: Permission mode, thinking level, and other preferences

### /cost

Displays token usage statistics for the current session:

- Input tokens consumed
- Output tokens generated
- Total thinking tokens
- Estimated cost (based on per-model pricing configured in settings)

### /resume

In TUI mode, displays an interactive picker to browse and select from saved sessions. In stdin REPL mode, accepts a session ID as an argument.

### /init

Generates an AGENTS.md file tailored to the current codebase. This file helps the LLM understand the project structure, coding conventions, and common patterns. The generated file is placed in the project root.

### /rename

Renames the current session. Accepts a new name as an argument. Session names make it easier to find sessions later when using `/resume` or `/sessions`.

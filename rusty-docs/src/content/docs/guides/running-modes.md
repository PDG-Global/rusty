---
title: Running Modes
description: TUI, headless, and stdin REPL modes
---


## Overview

Rusty supports three running modes to suit different workflows and environments.

## TUI Mode (Default)

The default mode launches a full terminal UI built with Ratatui. This provides:

- Streaming responses with real-time display
- Interactive permission prompts
- Slash command support with tab completion
- Markdown rendering (bold, italic, code blocks, tables)
- Status bar showing model, permission mode, and token usage
- Scrollable message history
- Session save on exit
- Model registry picker via the sidebar (`/settings`)
- Structured task tracking with `--plan-with-tasks`

```bash
rusty
rusty --preset openai --api-key sk-...
rusty --plan-with-tasks
```

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Up/Down` | Scroll history |
| `Tab` | Autocomplete slash commands |
| `Ctrl+C` | Cancel current operation |
| `Ctrl+D` | Exit |

## Headless Mode

Headless mode runs a single prompt and prints the response. No interactive UI is displayed. Useful for scripting, CI pipelines, and non-interactive workflows.

```bash
rusty --prompt "Explain the error handling in this codebase"
rusty --preset xiaomi --prompt "List all public API functions"
```

Options available in headless mode:

| Flag | Description |
|------|-------------|
| `--prompt` | The prompt to send (required, triggers headless mode) |
| `--max-turns` | Maximum agent loop iterations |
| `--max-tokens` | Maximum tokens in the response |
| `--permissions` | Permission mode (use `bypass` for non-interactive) |

The session is saved automatically after the response completes.

## Stdin REPL Mode

Stdin mode provides an interactive line-by-line REPL without the TUI. Supports slash commands but does not have the full terminal UI rendering. Useful when the TUI is not available or when running in a simple terminal.

```bash
rusty --headless
```

Features:

- Line-by-line input
- Slash command support (`/help`, `/model`, `/sessions`, etc.)
- Streaming text output
- Session save on exit

## Choosing a Mode

| Use Case | Recommended Mode |
|----------|------------------|
| Interactive coding session | TUI |
| Scripting and automation | Headless |
| Simple terminal or SSH session | Stdin REPL |
| CI/CD pipelines | Headless with `--permissions bypass` |
| Quick one-off question | Headless |
| Long-running development work | TUI with session resume |
| Planning and task tracking | TUI with `--plan-with-tasks` |

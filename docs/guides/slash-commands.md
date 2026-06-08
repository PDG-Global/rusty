---
title: Slash Commands
description: Interactive commands available in TUI and REPL modes
---

## Overview

Slash commands are special commands typed in the input box, starting with `/`. They are available in both TUI mode and stdin REPL mode.

## Available Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `/help` | `/h`, `/?` | List all available commands |
| `/init` | | Generate an AGENTS.md file for the current codebase |
| `/resume` | `/r` | Resume a saved session (interactive picker in TUI) |
| `/sessions` | `/s` | List all saved sessions |
| `/compact` | | Force conversation compaction |
| `/clear` | | Clear the screen |
| `/copy` | `/c` | Copy the last assistant response to clipboard |
| `/model` | `/m` | Show the current model |
| `/rename` | | Rename the current session |
| `/quit` | `/exit`, `/q` | Exit Rusty (saves the session) |

## Command Details

### /help

Displays a list of all available commands with brief descriptions. Aliases: `/h`, `/?`.

### /init

Generates an `AGENTS.md` file tailored to the current codebase. This file provides context about the project structure, conventions, and key files to help the AI understand the project better.

### /resume

Opens an interactive session picker (in TUI mode) or lists sessions for selection (in stdin mode). Use this to continue a previous conversation. Alias: `/r`.

### /sessions

Lists all saved sessions with their IDs, models, and timestamps. Alias: `/s`.

### /compact

Forces conversation compaction. This summarizes older messages to reduce context window usage. Normally, compaction triggers automatically when the conversation exceeds approximately 80,000 tokens or 40 messages. Use this command to trigger it manually.

### /clear

Clears the terminal screen. The conversation history is preserved; only the display is cleared.

### /copy

Copies the last assistant response to the system clipboard. Alias: `/c`.

### /model

Displays the currently active model name and provider. Alias: `/m`.

### /rename

Renames the current session. Prompts for a new name in TUI mode.

### /quit

Exits Rusty. The current session is saved automatically before exiting. Aliases: `/exit`, `/q`.

## Tab Completion

In TUI mode, slash commands support tab completion. Type `/` and press `Tab` to cycle through available commands. Partial commands can also be completed:

- Type `/he` + `Tab` to complete to `/help`
- Type `/re` + `Tab` to complete to `/resume`

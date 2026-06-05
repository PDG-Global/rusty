---
title: Sessions
description: Save, resume, and manage conversation sessions
---

## Overview

Rusty automatically saves conversation sessions to `~/.rusty/sessions/` as JSON files. Each session stores the full message history, model information, and timestamps.

## Session Storage

Sessions are stored at:

```
~/.rusty/sessions/
├── <session-id>.json
├── <session-id>.json
└── ...
```

Each session file contains:

- **id**: Unique session identifier
- **messages**: Full conversation history
- **model**: Model used for the conversation
- **created_at**: Session start timestamp
- **updated_at**: Last activity timestamp

## Listing Sessions

View all saved sessions:

```bash
rusty --list-sessions
```

This displays a table of sessions with their IDs, models, and timestamps.

## Resuming a Session

Resume a previous session by its ID:

```bash
rusty --resume <session-id>
```

In TUI mode, use the `/resume` slash command for an interactive picker that lets you browse and select from saved sessions.

## Session Management

### Auto-Save

Sessions are saved automatically:

- In TUI mode: when you exit (via `/quit`, `Ctrl+D`, or closing the terminal)
- In headless mode: after the response completes
- In stdin REPL mode: when you exit

### Session Naming

Sessions can be renamed using the `/rename` slash command in TUI or stdin REPL mode.

### Session Limits

There is no hard limit on the number of saved sessions. Sessions accumulate in `~/.rusty/sessions/` until manually cleaned up.

## Workflow Examples

### Long-Running Development

Start a session, work on a feature, exit when done:

```bash
rusty
# ... work on the feature ...
# /quit to exit and save
```

Resume the next day:

```bash
rusty --resume <session-id>
# or in TUI: /resume
```

### Scripting with Context

Use headless mode to build up context across multiple calls:

```bash
# First call
rusty --prompt "Analyse the crate structure" --headless

# Resume for follow-up (session auto-saved from previous call)
rusty --resume <session-id> --prompt "Now suggest improvements"
```

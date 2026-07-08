---
title: Tools Overview
description: Overview of all tools available to Rusty
---


## Built-in Tools

Rusty provides a set of built-in tools that the LLM can invoke during a conversation. Each tool has a defined permission level and purpose.

| Tool | Permission | Description |
|------|------------|-------------|
| `file_read` | ReadOnly | Read file contents with optional offset and limit |
| `file_write` | Write | Create or overwrite files |
| `file_edit` | Write | Exact string match-and-replace editing |
| `apply_patch` | Write | Apply unified diff patches |
| `bash` | Classified | Execute shell commands |
| `grep` | ReadOnly | Regex search across files |
| `glob` | ReadOnly | File pattern matching |
| `web_fetch` | ReadOnly | Fetch content from URLs (with SSRF protection) |
| `todowrite` | None | Structured task list management |
| `note` | None | Session-scoped scratchpad for recording observations |
| `agent` | None | Spawn sub-agents for complex tasks |
| `memory` | None | Per-project persistent memory (save, search, list, delete) |

## Permission Levels

Tools operate under one of four permission levels:

- **None**: No special permissions required. Always allowed.
- **ReadOnly**: Can read data but cannot modify files or system state.
- **Write**: Can create or modify files in the working directory.
- **Execute**: Can run system commands. The bash tool is classified per-command.

## Path Sandboxing

All file tools enforce path sandboxing via `resolve_path()`. Paths are canonicalized (resolving symlinks and `..` components) and validated to ensure they remain within the working directory. Attempts to access files outside the sandbox are rejected.

The sandbox is hardened against TOCTOU symlink races:

- **Pre-write verification** via `verify_not_escaping_symlink()` checks before file creation.
- **Post-write re-verification** via `verify_no_symlink_escape()` confirms after write.
- Avoids `path.exists()` before `canonicalize()` to prevent race conditions.

The bash tool uses `check_bash_paths()` to validate path-like tokens and redirect targets in commands before execution.

## Tool Definitions

Each tool exposes:

- **Name**: Unique identifier
- **Description**: What the tool does
- **Input schema**: JSON Schema defining accepted parameters
- **Permission level**: What permissions the tool requires

The LLM receives tool definitions at the start of a conversation and can invoke them by name with structured arguments.

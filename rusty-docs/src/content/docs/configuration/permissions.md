---
title: Permissions
description: Understanding Rusty's tiered permission system
---


## Permission Modes

Rusty enforces a tiered permission system to control what tools can do. There are four modes:

| Mode | Description |
|------|-------------|
| `default` | Interactive prompts for write/execute operations |
| `accept-edits` | Auto-allow file writes, prompt for bash execution |
| `bypass` | Allow all operations without prompting |
| `plan` | Read-only mode, no write or execute operations |

Set the mode via CLI:

```bash
rusty --permissions bypass
```

Or in settings:

```json
{
  "permission_mode": "default"
}
```

## Permission Levels

Each tool has a permission level that determines what it can do:

| Level | Description | Examples |
|-------|-------------|----------|
| `None` | No special permissions needed | `todowrite`, `agent`, `note`, `memory` |
| `ReadOnly` | Can read but not modify | `file_read`, `grep`, `glob`, `web_fetch` |
| `Write` | Can create or modify files | `file_write`, `file_edit`, `apply_patch` |
| `Execute` | Can run system commands | `bash` (classified per-command) |

## Bash Command Classification

The bash tool uses `classify_bash_command()` to determine permission level. Commands are classified as either read-only or requiring write/execute permissions.

### Read-Only Commands

These commands bypass write permissions and are auto-allowed:

- `ls`, `cat`, `head`, `tail`, `wc`, `find`
- `git status`, `git log`, `git diff`, `git show`
- `cargo check`, `cargo test`, `cargo clippy`, `cargo build`
- `npm list`, `npm test`, `yarn test`
- Any command piped through read-only commands

### Write/Execute Commands

These commands require explicit permission (in `default` mode):

- `git commit`, `git push`, `git checkout`
- `rm`, `mv`, `cp`
- `npm install`, `cargo run`
- `docker`, `ssh`, `curl`

## Allowlists

### Permanent Allowlist

To permanently allow specific tools without prompting, add them to the `allowed_tools` array in `~/.rusty/settings.json`:

```json
{
  "allowed_tools": [
    "bash:git status",
    "bash:cargo check",
    "bash:npm test"
  ]
}
```

The format is `tool_name:exact_invocation_prefix`. The allowlist matches the beginning of the tool invocation.

### Session Allowlist

During a session, when you choose "Allow for this session" at a permission prompt, that tool is allowed for the remainder of the session. Session allowlist entries are not persisted across sessions.

## Permission Decision Flow

When a tool is invoked, Rusty checks permissions in this order:

1.  **Bypass mode** -- If permission mode is `bypass`, allow immediately.
2.  **Plan mode** -- If permission mode is `plan`, deny all write/execute operations.
3.  **Read-only or None level** -- Tools with `ReadOnly` or `None` permission levels are always allowed.
4.  **AcceptEdits + Write** -- If mode is `accept-edits` and tool level is `Write`, allow without prompting.
5.  **Permanent allowlist** -- Check if the tool invocation matches an entry in `allowed_tools`.
6.  **Session allowlist** -- Check if the user previously allowed this tool in the current session.
7.  **Interactive prompt** -- In TUI mode, prompt the user with options: Allow Once, Allow Session, Allow Always, or Deny.
8.  **Default deny** -- In headless mode without an interactive callback, deny.

### User-Facing Prompt Options

When an interactive prompt appears, the user can choose:

| Option | Description |
|--------|-------------|
| Allow Once | Allow this single invocation |
| Allow Session | Allow for the remainder of this session |
| Allow Always | Permanently add to `allowed_tools` in settings |
| Deny | Block this invocation |

## Plan Mode

Plan mode is designed for reviewing and planning without making changes:

```bash
rusty --permissions plan
```

In plan mode, Rusty can read files, search code, and browse the web, but cannot write files or execute commands. This is useful for code review and analysis tasks.

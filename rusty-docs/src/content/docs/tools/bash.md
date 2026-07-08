---
title: Bash
description: Execute shell commands with automatic classification
---


## Overview

The `bash` tool executes shell commands in the working directory. It uses an automatic classifier to determine whether a command is read-only or requires write/execute permissions.

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `command` | string | Yes | The shell command to execute |
| `timeout` | integer | No | Timeout in seconds (default: 120) |

## Command Classification

Rusty classifies bash commands to determine permission requirements automatically.

### Read-Only Commands

These commands are auto-allowed and bypass write permissions:

**File inspection:** `ls`, `cat`, `head`, `tail`, `wc`, `find`, `file`, `stat`, `du`

**Git read operations:** `git status`, `git log`, `git diff`, `git show`, `git branch`, `git remote`

**Build and test:** `cargo check`, `cargo test`, `cargo clippy`, `cargo build`, `npm test`, `yarn test`, `pytest`

**Package info:** `npm list`, `cargo tree`, `pip list`

**System info:** `uname`, `whoami`, `pwd`, `which`, `env`, `date`

### Write/Execute Commands

These commands require explicit permission in `default` mode:

**Git write operations:** `git commit`, `git push`, `git checkout`, `git merge`, `git rebase`, `git stash`

**File operations:** `rm`, `mv`, `cp`, `chmod`, `chown`, `mkdir`, `touch`

**Package management:** `npm install`, `pip install`, `cargo install`

**Execution:** `docker`, `ssh`, `curl`, `wget`, `python`, `node`

### Piped Commands

When commands are piped, the classifier examines the full pipeline. If all components are read-only, the command is classified as read-only:

```bash
# Read-only: ls piped to grep
ls -la | grep ".rs"

# Write: redirect to file
echo "hello" > output.txt
```

## Path Sandboxing

The bash tool performs path validation before executing commands via `check_bash_paths()`:

- **Path token extraction**: Parses the command for path-like tokens (absolute paths, `./`, `../`, `~` expansions).
- **Redirect target validation**: Extracts and validates redirect targets (`>`, `>>`, `2>`) against the working directory.
- **Boundary enforcement**: Rejects commands where any path token or redirect target resolves outside the working directory.

```bash
# Allowed: relative path within project
cat src/main.rs

# Blocked: absolute path outside sandbox
cat /etc/passwd

# Blocked: redirect target outside sandbox
echo "data" > /tmp/output.txt
```

:::note
Path sandboxing does not catch paths constructed via shell variables (`$VAR`), subshells, or commands that `cd` internally. Complex pipelines may have reduced coverage.
:::

## Examples

```json
{
  "command": "cargo check --workspace"
}
```

```json
{
  "command": "git diff --stat HEAD~5",
  "timeout": 30
}
```

```json
{
  "command": "python -m pytest tests/ -v",
  "timeout": 300
}
```

## Error Handling

- Commands that exit with a non-zero status return the stderr output along with the exit code.
- Commands exceeding the timeout are terminated and return a timeout error.
- The working directory is always the project root (or the directory specified with `--cwd`).

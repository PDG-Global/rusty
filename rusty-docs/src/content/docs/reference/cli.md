---
title: CLI Flags
---


# CLI Flags

All flags can be passed to the `rusty` binary. Flags override values from the settings file and environment variables.

```bash title="terminal"
rusty [OPTIONS] [PROMPT]
```

## General

| Flag | Description |
|------|-------------|
| `--setup` | Run the interactive setup wizard |
| `-v`, `--verbose` | Enable verbose logging |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Model & Provider

| Flag | Description |
|------|-------------|
| `--model <MODEL>` | Model to use (e.g. `mimo-v2.5-pro`, `gpt-4o`) |
| `--preset <PRESET>` | Provider preset: `xiaomi`, `kimi`, `openai`, `deepseek`, `ollama` |
| `--api-key <KEY>` | API key |
| `--api-base <URL>` | API base URL |
| `--max-tokens <N>` | Max tokens per response |
| `--temperature <F>` | Sampling temperature |
| `--thinking-budget <N>` | Reasoning token budget |

## Session

| Flag | Description |
|------|-------------|
| `--resume <ID>` | Resume a saved session by ID |
| `--list-sessions` | List saved sessions and exit |

## Run Mode

| Flag | Description |
|------|-------------|
| `--prompt <TEXT>` | Single prompt mode (headless). Prints response and exits. |
| `--headless` | Interactive REPL without TUI (stdin/stdout) |

## Permissions

| Flag | Description |
|------|-------------|
| `--permissions <MODE>` | Permission mode: `default`, `accept-edits`, `bypass`, `plan` |
| `--plan-with-tasks` | Plan mode with task tracking (implies `--permissions plan`) |

## Agent

| Flag | Description |
|------|-------------|
| `--max-turns <N>` | Max agent turns before stopping |
| `--cwd <DIR>` | Working directory |
| `--no-claude-md` | Disable discovery of AGENTS.md/CLAUDE.md context files |
| `--append-system-prompt <TEXT>` | Append text to the system prompt |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUSTY_API_KEY` | API key (highest priority) |
| `OPENAI_API_KEY` | API key (fallback) |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | Logging level (`debug`, `info`, `warn`) |

:::tip
Flags always take precedence over settings file values and environment
variables. Use `--preset` for quick provider switching without editing config.
:::

---
title: Settings
description: Rusty configuration files and runtime settings
---

## Settings File

Rusty stores its persistent configuration at `~/.rusty/settings.json`. This file is created by the setup wizard on first run.

```json
{
  "api_key": "sk-...",
  "api_base": "https://api.openai.com/v1",
  "default_model": "gpt-4o",
  "allowed_tools": [
    "bash:git status",
    "bash:cargo check"
  ],
  "credential_store": "keyring"
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `api_key` | string | API key for the LLM provider (stored if not using keyring) |
| `api_base` | string | Base URL for the API endpoint |
| `default_model` | string | Model identifier to use by default |
| `allowed_tools` | string[] | Permanently allowed tool invocations |
| `credential_store` | enum | `"keyring"` or `"settings_file"` |

## CLI Flags

All settings can be overridden at runtime via CLI flags:

| Flag | Description |
|------|-------------|
| `--model` | Override the model |
| `--api-key` | Override the API key |
| `--api-base` | Override the API base URL |
| `--preset` | Use a named preset (overrides api_base and model) |
| `--permissions` | Set permission mode: `default`, `accept-edits`, `bypass`, `plan` |
| `--max-turns` | Maximum agent loop iterations |
| `--max-tokens` | Maximum tokens in the response |
| `--temperature` | Sampling temperature |
| `--thinking-budget` | Token budget for reasoning/thinking content |
| `--plan-with-tasks` | Enable structured task tracking in responses |
| `--cwd` | Set the working directory |
| `--prompt` | Run in headless mode with a single prompt |
| `--headless` | Run in stdin REPL mode |
| `--resume` | Resume a saved session by ID |
| `--list-sessions` | List all saved sessions |
| `--verbose` | Enable verbose logging |
| `--setup` | Force the setup wizard to run |

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `OPENAI_API_KEY` | API key (lower priority than `RUSTY_API_KEY`) |
| `RUSTY_API_KEY` | API key (higher priority) |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | Logging level (`debug`, `info`, `warn`) |

## Resolution Order

Settings are resolved in this order (later wins):

1. Preset defaults
2. `~/.rusty/settings.json`
3. Environment variables
4. CLI flags

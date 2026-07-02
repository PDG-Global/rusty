---
title: Settings
description: Rusty configuration files and runtime settings
---

## Settings File

Rusty stores its persistent configuration at `~/.rusty/settings.json`. This file is created by the setup wizard on first run.

```json
{
  "api_key": "sk-...",
  "api_base": "https://api.xiaomimimo.com/v1",
  "default_model": "mimo-v2.5-pro",
  "models": [],
  "active_model": "mimo-v2.5-pro",
  "api_keys": {},
  "allowed_tools": [
    "bash:git status",
    "bash:cargo check"
  ],
  "credential_store": "keyring",
  "thinking_level": "normal",
  "permission_mode": "default",
  "permissions": {}
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `api_key` | string | API key for the LLM provider (stored if not using keyring) |
| `api_base` | string | Base URL for the API endpoint |
| `default_model` | string | Model identifier to use by default |
| `models` | array | Model registry entries (see Model Registry below) |
| `active_model` | string | Which model entry is currently active |
| `api_keys` | object | Per-model API keys (keyed by model ID) |
| `allowed_tools` | string[] | Permanently allowed tool invocations |
| `credential_store` | enum | `"keyring"` or `"settings_file"` |
| `thinking_level` | enum | `"minimal"`, `"normal"`, or `"deep"` |
| `permission_mode` | enum | `"default"`, `"accept-edits"`, `"bypass"`, or `"plan"` |
| `permissions` | object | Per-tool permission decisions |

## Model Registry

The `models` array defines available LLM models. Each entry has:

| Field | Type | Description |
|-------|------|-------------|
| `group` | string | Display grouping (e.g. "Xiaomi", "Anthropic") |
| `name` | string | Human-readable name |
| `provider` | enum | `OpenAI` or `Anthropic` (determines wire format) |
| `api_base` | string | API endpoint URL |
| `model` | string | Model ID sent to the API |
| `available_models` | string[] | Models to offer in the picker |
| `context_window` | number | Context window size in tokens |
| `thinking_budget` | number | Token budget for reasoning/thinking |
| `max_tokens` | number | Max output tokens |
| `temperature` | number | Sampling temperature |
| `extra_headers` | object | Additional HTTP headers |

The `active_model` field selects which model entry is used. The TUI `/settings` command provides a visual editor for the model registry.

## Thinking Levels

The `thinking_level` field controls the token budget allocated to reasoning:

| Level | Token Budget | Use Case |
|-------|-------------|----------|
| `minimal` | 1024 | Simple queries, quick responses |
| `normal` | 4096 | Standard multi-step tasks |
| `deep` | 16384 | Complex reasoning, architecture decisions |

**Dynamic adjustment**: The agent automatically adjusts the thinking level based on context:

- Multi-step tasks (2+ tool turns) are boosted from Minimal to Normal.
- Context usage above 70% triggers a one-level step-down.
- Context usage above 85% forces Minimal regardless of setting.

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
| `--no-claude-md` | Disable discovery of AGENTS.md/CLAUDE.md/RUSTY.md context files |
| `--append-system-prompt` | Append additional text to the system prompt |

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

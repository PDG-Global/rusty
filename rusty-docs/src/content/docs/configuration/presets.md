---
title: Presets
description: Pre-configured provider presets for quick setup
---


## Available Presets

Rusty ships with presets for popular LLM providers. Use `--preset` to select one:

| Preset | API Base | Default Model |
|--------|----------|---------------|
| `xiaomi` | `https://token-plan-cn.xiaomimimo.com/v1` | `mimo-v2.5-pro` |
| `kimi` | `https://api.kimi.com/coding/v1/` | `kimi-k2` |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` |
| `deepseek` | `https://api.deepseek.com` | `deepseek-v4-pro` |
| `ollama` | `http://localhost:11434/v1` | `llama3` |

:::note
The setup wizard may use slightly different default models for some providers (e.g. `kimi-k2`, `gpt-4.1`, `qwen3:8b` for Ollama). CLI presets take precedence when `--preset` is used.
:::

## Usage

```bash
# Use OpenAI with an API key
rusty --preset openai --api-key sk-...

# Use local Ollama
rusty --preset ollama

# Use DeepSeek
rusty --preset deepseek --api-key sk-...
```

## Custom Endpoints

For providers not listed as presets, set `--api-base` and `--model` directly:

```bash
rusty --api-base https://your-provider.com/v1 --api-key YOUR_KEY --model your-model
```

Any OpenAI-compatible API endpoint works, including:

- Azure OpenAI
- Together AI
- Groq
- Fireworks AI
- Self-hosted vLLM or text-generation-inference

## Presets vs Settings

When using `--preset`, the preset's `api_base` and `default_model` override what is in `~/.rusty/settings.json`. You can still override individual fields with CLI flags:

```bash
# Use Xiaomi preset but with a different model
rusty --preset xiaomi --model mimo-v2-lite --api-key YOUR_KEY
```

## Configuring a Default Provider

To set a default provider without using flags every time, edit `~/.rusty/settings.json`:

```json
{
  "api_base": "https://api.openai.com/v1",
  "default_model": "gpt-4o",
  "credential_store": "keyring"
}
```

Then run with just:

```bash
rusty
```

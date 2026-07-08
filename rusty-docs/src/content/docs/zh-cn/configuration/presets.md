---
title: 预设
description: 用于快速配置的预置提供方预设
---


## 可用预设

Rusty 内置了常见 LLM 提供方的预设。使用 `--preset` 选择其中之一：

| 预设 | API Base | 默认模型 |
|--------|----------|---------------|
| `xiaomi` | `https://token-plan-cn.xiaomimimo.com/v1` | `mimo-v2.5-pro` |
| `kimi` | `https://api.kimi.com/coding/v1/` | `kimi-k2` |
| `openai` | `https://api.openai.com/v1` | `gpt-4o` |
| `deepseek` | `https://api.deepseek.com` | `deepseek-v4-pro` |
| `ollama` | `http://localhost:11434/v1` | `llama3` |

:::note
对于某些提供方，设置向导可能使用略有不同的默认模型（例如 `kimi-k2`、`gpt-4.1`、Ollama 的 `qwen3:8b`）。使用 `--preset` 时，CLI 预设优先。
:::

## 用法

```bash
# 使用 OpenAI，并提供 API 密钥
rusty --preset openai --api-key sk-...

# 使用本地 Ollama
rusty --preset ollama

# 使用 DeepSeek
rusty --preset deepseek --api-key sk-...
```

## 自定义端点

对于未列为预设的提供方，直接设置 `--api-base` 与 `--model`：

```bash
rusty --api-base https://your-provider.com/v1 --api-key YOUR_KEY --model your-model
```

任何兼容 OpenAI 的 API 端点均可使用，包括：

- Azure OpenAI
- Together AI
- Groq
- Fireworks AI
- 自托管的 vLLM 或 text-generation-inference

## 预设与设置

使用 `--preset` 时，预设的 `api_base` 与 `default_model` 会覆盖 `~/.rusty/settings.json` 中的值。你仍可用 CLI 参数覆盖单个字段：

```bash
# 使用 Xiaomi 预设，但换一个模型
rusty --preset xiaomi --model mimo-v2-lite --api-key YOUR_KEY
```

## 配置默认提供方

若想不必每次都用参数就设定默认提供方，编辑 `~/.rusty/settings.json`：

```json
{
  "api_base": "https://api.openai.com/v1",
  "default_model": "gpt-4o",
  "credential_store": "keyring"
}
```

然后只需运行：

```bash
rusty
```

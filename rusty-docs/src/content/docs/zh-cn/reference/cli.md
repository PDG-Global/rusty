---
title: CLI 参数
---


# CLI 参数

所有参数都可传给 `rusty` 二进制文件。参数会覆盖设置文件与环境变量中的值。

```bash title="terminal"
rusty [OPTIONS] [PROMPT]
```

## 通用

| 参数 | 说明 |
|------|-------------|
| `--setup` | 运行交互式设置向导 |
| `-v`、`--verbose` | 启用详细日志 |
| `-h`、`--help` | 打印帮助 |
| `-V`、`--version` | 打印版本 |

## 模型与提供方

| 参数 | 说明 |
|------|-------------|
| `--model <MODEL>` | 要使用的模型（例如 `mimo-v2.5-pro`、`gpt-4o`） |
| `--preset <PRESET>` | 提供方预设：`xiaomi`、`kimi`、`openai`、`deepseek`、`ollama` |
| `--api-key <KEY>` | API 密钥 |
| `--api-base <URL>` | API base URL |
| `--max-tokens <N>` | 每次响应的最大 token 数 |
| `--temperature <F>` | 采样温度 |
| `--thinking-budget <N>` | 推理 token 预算 |

## 会话

| 参数 | 说明 |
|------|-------------|
| `--resume <ID>` | 按 ID 恢复已保存的会话 |
| `--list-sessions` | 列出已保存的会话并退出 |

## 运行模式

| 参数 | 说明 |
|------|-------------|
| `--prompt <TEXT>` | 单提示模式（无头）。打印响应后退出。 |
| `--headless` | 不带 TUI 的交互式 REPL（stdin/stdout） |

## 权限

| 参数 | 说明 |
|------|-------------|
| `--permissions <MODE>` | 权限模式：`default`、`accept-edits`、`bypass`、`plan` |
| `--plan-with-tasks` | 带任务跟踪的 plan 模式（隐含 `--permissions plan`） |

## 代理

| 参数 | 说明 |
|------|-------------|
| `--max-turns <N>` | 停止前的最大代理轮次 |
| `--cwd <DIR>` | 工作目录 |
| `--no-claude-md` | 禁用对 AGENTS.md/CLAUDE.md 上下文文件的发现 |
| `--append-system-prompt <TEXT>` | 向系统提示词追加文本 |

## 环境变量

| 变量 | 说明 |
|----------|-------------|
| `RUSTY_API_KEY` | API 密钥（最高优先级） |
| `OPENAI_API_KEY` | API 密钥（回退） |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | 日志级别（`debug`、`info`、`warn`） |

:::tip
参数始终优先于设置文件中的值与环境变量。使用 `--preset` 可在不编辑配置的情况下快速切换提供方。
:::

---
title: 设置
description: Rusty 的配置文件与运行时设置
---


## 设置文件

Rusty 将其持久化配置存储在 `~/.rusty/settings.json`。该文件由首次运行时的设置向导创建。

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

### 字段

| 字段 | 类型 | 说明 |
|-------|------|-------------|
| `api_key` | string | LLM 提供方的 API 密钥（不使用钥匙串时存储于此） |
| `api_base` | string | API 端点的 base URL |
| `default_model` | string | 默认使用的模型标识符 |
| `models` | array | 模型注册表条目（见下方「模型注册表」） |
| `active_model` | string | 当前活动的模型条目 |
| `api_keys` | object | 每模型的 API 密钥（以模型 ID 为键） |
| `allowed_tools` | string[] | 永久允许的工具调用 |
| `credential_store` | enum | `"keyring"` 或 `"settings_file"` |
| `thinking_level` | enum | `"minimal"`、`"normal"` 或 `"deep"` |
| `permission_mode` | enum | `"default"`、`"accept-edits"`、`"bypass"` 或 `"plan"` |
| `permissions` | object | 每工具的权限决策 |

## 模型注册表

`models` 数组定义可用的 LLM 模型。每个条目包含：

| 字段 | 类型 | 说明 |
|-------|------|-------------|
| `group` | string | 显示分组（例如 "Xiaomi"、"Anthropic"） |
| `name` | string | 人类可读的名称 |
| `provider` | enum | `OpenAI` 或 `Anthropic`（决定传输格式） |
| `api_base` | string | API 端点 URL |
| `model` | string | 发送给 API 的模型 ID |
| `available_models` | string[] | 选择器中可供选择的模型 |
| `context_window` | number | 上下文窗口大小（token） |
| `thinking_budget` | number | 推理/思考的 token 预算 |
| `max_tokens` | number | 最大输出 token 数 |
| `temperature` | number | 采样温度 |
| `extra_headers` | object | 额外的 HTTP 请求头 |

`active_model` 字段选择使用哪个模型条目。TUI 的 `/settings` 命令为模型注册表提供了可视化编辑器。

## 思考等级

`thinking_level` 字段控制分配给推理的 token 预算：

| 等级 | Token 预算 | 使用场景 |
|-------|-------------|----------|
| `minimal` | 1024 | 简单查询、快速响应 |
| `normal` | 4096 | 标准多步任务 |
| `deep` | 16384 | 复杂推理、架构决策 |

**动态调整**：代理会根据上下文自动调整思考等级：

- 多步任务（2 轮以上工具调用）从 Minimal 提升到 Normal。
- 上下文占用超过 70% 触发降一级。
- 上下文占用超过 85% 时，无论设置如何都强制为 Minimal。

## CLI 参数

所有设置都可在运行时通过 CLI 参数覆盖：

| 参数 | 说明 |
|------|-------------|
| `--model` | 覆盖模型 |
| `--api-key` | 覆盖 API 密钥 |
| `--api-base` | 覆盖 API base URL |
| `--preset` | 使用命名预设（覆盖 api_base 与 model） |
| `--permissions` | 设置权限模式：`default`、`accept-edits`、`bypass`、`plan` |
| `--max-turns` | 代理循环最大迭代次数 |
| `--max-tokens` | 响应的最大 token 数 |
| `--temperature` | 采样温度 |
| `--thinking-budget` | 推理/思考内容的 token 预算 |
| `--plan-with-tasks` | 在响应中启用结构化任务跟踪 |
| `--cwd` | 设置工作目录 |
| `--prompt` | 以无头模式运行单个提示 |
| `--headless` | 以标准输入 REPL 模式运行 |
| `--resume` | 按 ID 恢复已保存的会话 |
| `--list-sessions` | 列出所有已保存的会话 |
| `--verbose` | 启用详细日志 |
| `--setup` | 强制运行设置向导 |
| `--no-claude-md` | 禁用对 AGENTS.md/CLAUDE.md/RUSTY.md 上下文文件的发现 |
| `--append-system-prompt` | 向系统提示词追加额外文本 |

## 环境变量

| 变量 | 用途 |
|----------|---------|
| `OPENAI_API_KEY` | API 密钥（优先级低于 `RUSTY_API_KEY`） |
| `RUSTY_API_KEY` | API 密钥（优先级更高） |
| `OPENAI_BASE_URL` | API base URL |
| `RUST_LOG` | 日志级别（`debug`、`info`、`warn`） |

## 解析顺序

设置按以下顺序解析（后者优先）：

1. 预设默认值
2. `~/.rusty/settings.json`
3. 环境变量
4. CLI 参数

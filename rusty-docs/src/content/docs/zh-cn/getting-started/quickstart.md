---
title: 快速开始
description: 5 分钟内让 Rusty 跑起来
---


## 前置条件

- Rust 1.75+（2021 edition）
- 一个受支持的 LLM 提供方的 API 密钥

## 安装

### 从源码构建

```bash
git clone https://github.com/pdg-global/rusty.git
cd rusty
cargo build --release
```

二进制文件位于 `./target/release/rusty`。

### 通过 Cargo 安装

```bash
cargo install rusty
```

## 首次运行

首次运行 Rusty 时，设置向导会自动启动：

```bash
rusty
```

向导会引导你：

1. **选择提供方**——Xiaomi、Kimi、OpenAI、DeepSeek、Ollama，或自定义端点
2. **输入 API 密钥**（输入内容会被遮盖）
3. **选择凭据存储方式**（操作系统钥匙串或设置文件）
4. **为所选提供方选择模型**
5. **测试连通性**，确认配置可用

配置会保存到 `~/.rusty/settings.json`。

## 使用预设

直接指定预设与 API 密钥，可跳过向导：

```bash
rusty --preset openai --api-key sk-...
```

可用预设：`xiaomi`、`kimi`、`openai`、`deepseek`、`ollama`。

## 环境变量

也可以通过环境变量而非参数来设置 API 密钥：

```bash
export OPENAI_API_KEY=sk-...
rusty --preset openai
```

若两者都设置，`RUSTY_API_KEY` 的优先级高于 `OPENAI_API_KEY`。

## 常用参数

| 参数 | 说明 |
|------|-------------|
| `--model` | 覆盖默认模型 |
| `--permissions` | 设置权限模式：`default`、`accept-edits`、`bypass`、`plan` |
| `--plan-with-tasks` | 在响应中启用结构化任务跟踪 |
| `--thinking-budget` | 推理/思考内容的 token 预算 |
| `--cwd` | 设置工作目录 |
| `--no-claude-md` | 禁用对 AGENTS.md/CLAUDE.md/RUSTY.md 上下文文件的发现 |
| `--append-system-prompt` | 向系统提示词追加额外文本 |
| `--resume` | 按 ID 恢复已保存的会话 |

## 你的第一次对话

启动后，直接输入你的提示词并回车。Rusty 会实时流式返回响应，并在需要时请求执行工具。

不妨试着问：

- “这个目录里有哪些文件？”
- “读取 Cargo.toml 并解释其中的依赖”
- “用 Python 写一个 hello world 脚本”

输入 `/help` 查看可用的斜杠命令，或输入 `/quit` 退出（会话会自动保存）。

---
title: 运行模式
description: TUI、无头与标准输入 REPL 模式
---


## 概述

Rusty 支持三种运行模式，以适应不同的工作流与环境。

## TUI 模式（默认）

默认模式会启动一个用 Ratatui 构建的完整终端界面，提供：

- 实时显示的流式响应
- 交互式权限提示
- 带 tab 补全的斜杠命令支持
- Markdown 渲染（粗体、斜体、代码块、表格）
- 显示模型、权限模式与 token 用量的状态栏
- 可滚动的消息历史
- 退出时保存会话
- 通过侧边栏（`/settings`）选择模型注册表
- 通过 `--plan-with-tasks` 进行结构化任务跟踪

```bash
rusty
rusty --preset openai --api-key sk-...
rusty --plan-with-tasks
```

### 键盘快捷键

| 按键 | 动作 |
|-----|--------|
| `Enter` | 发送消息 |
| `Up/Down` | 滚动历史 |
| `Tab` | 自动补全斜杠命令 |
| `Ctrl+C` | 取消当前操作 |
| `Ctrl+D` | 退出 |

## 无头模式

无头模式运行单个提示并打印响应，不显示交互式界面。适合脚本、CI 流水线与非交互式工作流。

```bash
rusty --prompt "解释这个代码库中的错误处理"
rusty --preset xiaomi --prompt "列出所有公开的 API 函数"
```

无头模式下可用的选项：

| 参数 | 说明 |
|------|-------------|
| `--prompt` | 要发送的提示（必填，触发无头模式） |
| `--max-turns` | 代理循环最大迭代次数 |
| `--max-tokens` | 响应的最大 token 数 |
| `--permissions` | 权限模式（非交互场景使用 `bypass`） |

响应完成后会自动保存会话。

## 标准输入 REPL 模式

标准输入模式提供不带 TUI 的逐行交互式 REPL。支持斜杠命令，但没有完整的终端界面渲染。适用于 TUI 不可用或在简单终端中运行的场景。

```bash
rusty --headless
```

特性：

- 逐行输入
- 斜杠命令支持（`/help`、`/model`、`/sessions` 等）
- 流式文本输出
- 退出时保存会话

## 如何选择模式

| 使用场景 | 推荐模式 |
|----------|------------------|
| 交互式编码会话 | TUI |
| 脚本与自动化 | 无头 |
| 简单终端或 SSH 会话 | 标准输入 REPL |
| CI/CD 流水线 | 无头 + `--permissions bypass` |
| 快速一次性提问 | 无头 |
| 长时间的开发工作 | TUI + 会话恢复 |
| 规划与任务跟踪 | TUI + `--plan-with-tasks` |

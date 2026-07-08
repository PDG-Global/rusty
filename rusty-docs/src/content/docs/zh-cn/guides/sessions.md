---
title: 会话
description: 保存、恢复与管理对话会话
---


## 概述

Rusty 会自动将对话会话以 JSON 文件形式保存到 `~/.rusty/sessions/`。每个会话存储完整的消息历史、模型信息与时间戳。

## 会话存储

会话存储于：

```
~/.rusty/sessions/
├── <session-id>.json
├── <session-id>.notes.md
├── <session-id>.checkpoint.md
└── ...
```

每个会话文件包含：

- **id**：唯一会话标识符
- **messages**：完整的对话历史
- **model**：本次对话使用的模型
- **created_at**：会话开始时间戳
- **updated_at**：最近活动时间戳

### 附属文件

会话可以关联一些附属（sidecar）文件：

| 文件 | 用途 |
|------|---------|
| `<id>.notes.md` | 由 `note` 工具写入的会话级暂存区。内容在检查点提取时被处理，使用后清空。 |
| `<id>.checkpoint.md` | 在第二级压缩期间提取的结构化状态。当旧消息被总结时，保留关键上下文。 |

删除会话时，附属文件会被自动清理。

## 列出会话

查看所有已保存的会话：

```bash
rusty --list-sessions
```

这会显示一个包含会话 ID、模型与时间戳的表格。

## 恢复会话

按 ID 恢复之前的会话：

```bash
rusty --resume <session-id>
```

在 TUI 模式下，使用 `/resume` 斜杠命令可打开交互式选择器，浏览并选择已保存的会话。

## 会话管理

### 自动保存

会话会自动保存：

- TUI 模式：退出时（通过 `/quit`、`Ctrl+D` 或关闭终端）
- 无头模式：响应完成后
- 标准输入 REPL 模式：退出时

### 会话命名

在 TUI 或标准输入 REPL 模式下，可使用 `/rename` 斜杠命令重命名会话。

### 会话数量

已保存会话的数量没有硬性上限。会话会在 `~/.rusty/sessions/` 中累积，直到手动清理。

## 工作流示例

### 长时间开发

启动会话、开发某个功能、完成后退出：

```bash
rusty
# ……开发功能……
# /quit 退出并保存
```

第二天恢复：

```bash
rusty --resume <session-id>
# 或在 TUI 中：/resume
```

### 带上下文的脚本化

使用无头模式在多次调用间累积上下文：

```bash
# 第一次调用
rusty --prompt "分析这个 crate 的结构" --headless

# 恢复以进行后续（会话已由上次调用自动保存）
rusty --resume <session-id> --prompt "现在提出改进建议"
```

---
title: 权限
description: 理解 Rusty 的分级权限系统
---


## 权限模式

Rusty 采用分级权限系统来控制工具能做什么。共有四种模式：

| 模式 | 说明 |
|------|-------------|
| `default` | 对写入/执行操作进行交互式提示 |
| `accept-edits` | 自动允许文件写入，对 bash 执行进行提示 |
| `bypass` | 允许所有操作，不做提示 |
| `plan` | 只读模式，不进行写入或执行 |

通过 CLI 设置模式：

```bash
rusty --permissions bypass
```

或在设置中：

```json
{
  "permission_mode": "default"
}
```

## 权限级别

每个工具都有一个权限级别，决定它能做什么：

| 级别 | 说明 | 示例 |
|-------|-------------|----------|
| `None` | 无需特殊权限 | `todowrite`、`agent`、`note`、`memory` |
| `ReadOnly` | 可读但不可修改 | `file_read`、`grep`、`glob`、`web_fetch` |
| `Write` | 可创建或修改文件 | `file_write`、`file_edit`、`apply_patch` |
| `Execute` | 可运行系统命令 | `bash`（按命令逐一分类） |

## Bash 命令分类

bash 工具使用 `classify_bash_command()` 来判定权限级别。命令被分类为只读，或需要写入/执行权限。

### 只读命令

这些命令绕过写入权限并被自动允许：

- `ls`、`cat`、`head`、`tail`、`wc`、`find`
- `git status`、`git log`、`git diff`、`git show`
- `cargo check`、`cargo test`、`cargo clippy`、`cargo build`
- `npm list`、`npm test`、`yarn test`
- 任何通过只读命令进行管道处理的命令

### 写入/执行命令

这些命令（在 `default` 模式下）需要显式权限：

- `git commit`、`git push`、`git checkout`
- `rm`、`mv`、`cp`
- `npm install`、`cargo run`
- `docker`、`ssh`、`curl`

## 允许列表

### 永久允许列表

若要永久允许特定工具而不提示，将它们加入 `~/.rusty/settings.json` 中的 `allowed_tools` 数组：

```json
{
  "allowed_tools": [
    "bash:git status",
    "bash:cargo check",
    "bash:npm test"
  ]
}
```

格式为 `tool_name:exact_invocation_prefix`。允许列表匹配工具调用的开头部分。

### 会话允许列表

会话期间，当你在权限提示中选择「本次会话允许」时，该工具会在本次会话的剩余时间内被允许。会话允许列表条目不会跨会话持久化。

## 权限决策流程

工具被调用时，Rusty 按以下顺序检查权限：

1.  **Bypass 模式**——若权限模式为 `bypass`，立即允许。
2.  **Plan 模式**——若权限模式为 `plan`，拒绝所有写入/执行操作。
3.  **只读或 None 级别**——权限级别为 `ReadOnly` 或 `None` 的工具始终被允许。
4.  **AcceptEdits + Write**——若模式为 `accept-edits` 且工具级别为 `Write`，无需提示直接允许。
5.  **永久允许列表**——检查工具调用是否匹配 `allowed_tools` 中的条目。
6.  **会话允许列表**——检查用户此前是否在当前会话中允许过此工具。
7.  **交互式提示**——在 TUI 模式下，向用户提示选项：允许一次、允许本次会话、始终允许、拒绝。
8.  **默认拒绝**——在无交互回调的无头模式下，拒绝。

### 面向用户的提示选项

出现交互式提示时，用户可以选择：

| 选项 | 说明 |
|--------|-------------|
| 允许一次 | 允许这一次调用 |
| 允许本次会话 | 在本次会话剩余时间内允许 |
| 始终允许 | 永久加入设置中的 `allowed_tools` |
| 拒绝 | 阻止此次调用 |

## Plan 模式

Plan 模式让 Rusty 在做任何更改之前先进行调查。按 Shift+Tab 可在 default、accept-edits 和 plan 之间循环，或运行 `/plan`（后面可附带任务）。使用 `--permissions plan` 可直接以 plan 模式启动。

1. **调查**：允许只读工具、只读 shell 命令和 explore 子代理，其余一律拒绝，即使在 bypass 模式下也是如此。
2. **提出方案**：Rusty 调用 `exit_plan_mode` 并给出带编号的计划，计划会显示在对话记录中。
3. **批准**：回复 1（批准并自动接受编辑）、2（批准并在编辑前询问）或 3 / 任意文字（附上反馈继续规划）。规划前处于 bypass 模式的会话在批准后会回到 bypass。
4. **执行**：计划的顶层步骤会成为任务列表，Rusty 在同一次运行中实施。

使用 `--prompt` 时无人可询问，因此会打印计划并结束运行，仍处于 plan 模式。

---
title: Bash
description: 执行 shell 命令并自动分类
---


## 概述

`bash` 工具在工作目录中执行 shell 命令。它使用自动分类器来判定命令是只读，还是需要写入/执行权限。

## 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `command` | string | 是 | 要执行的 shell 命令 |
| `timeout` | integer | 否 | 超时时间（秒，默认 120） |

## 命令分类

Rusty 会对 bash 命令进行分类，以自动判定权限要求。

### 只读命令

这些命令被自动允许并绕过写入权限：

**文件检查：** `ls`、`cat`、`head`、`tail`、`wc`、`find`、`file`、`stat`、`du`

**Git 读操作：** `git status`、`git log`、`git diff`、`git show`、`git branch`、`git remote`

**构建与测试：** `cargo check`、`cargo test`、`cargo clippy`、`cargo build`、`npm test`、`yarn test`、`pytest`

**包信息：** `npm list`、`cargo tree`、`pip list`

**系统信息：** `uname`、`whoami`、`pwd`、`which`、`env`、`date`

### 写入/执行命令

这些命令在 `default` 模式下需要显式权限：

**Git 写操作：** `git commit`、`git push`、`git checkout`、`git merge`、`git rebase`、`git stash`

**文件操作：** `rm`、`mv`、`cp`、`chmod`、`chown`、`mkdir`、`touch`

**包管理：** `npm install`、`pip install`、`cargo install`

**执行：** `docker`、`ssh`、`curl`、`wget`、`python`、`node`

### 管道命令

当命令通过管道连接时，分类器会检查整条管道。若所有组成部分都是只读的，则该命令被分类为只读：

```bash
# 只读：ls 管道给 grep
ls -la | grep ".rs"

# 写入：重定向到文件
echo "hello" > output.txt
```

## 路径沙箱

bash 工具在执行命令前通过 `check_bash_paths()` 进行路径校验：

- **路径 token 提取**：解析命令中的类路径 token（绝对路径、`./`、`../`、`~` 展开）。
- **重定向目标校验**：提取并对照工作目录校验重定向目标（`>`、`>>`、`2>`）。
- **边界强制**：拒绝任何路径 token 或重定向目标解析到工作目录之外的命令。

```bash
# 允许：项目内的相对路径
cat src/main.rs

# 阻止：沙箱外的绝对路径
cat /etc/passwd

# 阻止：沙箱外的重定向目标
echo "data" > /tmp/output.txt
```

:::note
路径沙箱无法捕获通过 shell 变量（`$VAR`）、子 shell 或内部 `cd` 的命令所构造的路径。复杂管道的覆盖范围可能有所降低。
:::

## 示例

```json
{
  "command": "cargo check --workspace"
}
```

```json
{
  "command": "git diff --stat HEAD~5",
  "timeout": 30
}
```

```json
{
  "command": "python -m pytest tests/ -v",
  "timeout": 300
}
```

## 错误处理

- 以非零状态退出的命令会返回 stderr 输出以及退出码。
- 超过超时的命令会被终止并返回超时错误。
- 工作目录始终是项目根目录（或用 `--cwd` 指定的目录）。

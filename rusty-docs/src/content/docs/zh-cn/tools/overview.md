---
title: 工具概览
description: Rusty 可用的所有工具概览
---


## 内置工具

Rusty 提供一组内置工具，供 LLM 在对话中调用。每个工具都有既定的权限级别与用途。

| 工具 | 权限 | 说明 |
|------|------------|-------------|
| `file_read` | ReadOnly | 读取文件内容，可选偏移与行数上限 |
| `file_write` | Write | 创建或覆盖文件 |
| `file_edit` | Write | 精确字符串匹配替换编辑 |
| `apply_patch` | Write | 应用统一 diff 补丁 |
| `bash` | Classified | 执行 shell 命令 |
| `grep` | ReadOnly | 跨文件正则搜索 |
| `glob` | ReadOnly | 文件通配符匹配 |
| `web_fetch` | ReadOnly | 从 URL 获取内容（带 SSRF 防护） |
| `todowrite` | None | 结构化任务列表管理 |
| `note` | None | 会话级暂存区，用于记录观察 |
| `agent` | None | 为复杂任务派生子代理 |
| `memory` | None | 每项目的持久化记忆（保存、搜索、列出、删除） |

## 权限级别

工具在四种权限级别之一下运行：

- **None**：无需特殊权限。始终允许。
- **ReadOnly**：可读取数据，但不能修改文件或系统状态。
- **Write**：可在工作目录中创建或修改文件。
- **Execute**：可运行系统命令。bash 工具按命令逐一分类。

## 路径沙箱

所有文件工具都通过 `resolve_path()` 强制执行路径沙箱。路径会被规范化（解析符号链接与 `..` 组件）并经过校验，以确保它们始终位于工作目录内。访问沙箱外文件的尝试会被拒绝。

该沙箱针对 TOCTOU 符号链接竞态做了加固：

- **写入前校验**：通过 `verify_not_escaping_symlink()` 在创建文件前检查。
- **写入后再校验**：通过 `verify_no_symlink_escape()` 在写入后确认。
- 避免在 `canonicalize()` 之前调用 `path.exists()`，以防止竞态条件。

bash 工具使用 `check_bash_paths()` 在执行前校验命令中的类路径 token 与重定向目标。

## 工具定义

每个工具都暴露：

- **名称**：唯一标识符
- **描述**：工具的作用
- **输入 schema**：定义可接受参数的 JSON Schema
- **权限级别**：工具所需的权限

LLM 在对话开始时接收工具定义，并可通过名称配合结构化参数来调用它们。

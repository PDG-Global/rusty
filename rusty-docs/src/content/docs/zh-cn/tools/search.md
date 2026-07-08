---
title: 搜索
description: 用 grep 与 glob 查找文件并搜索内容
---


## grep

在项目文件中搜索正则表达式模式。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `pattern` | string | 是 | 要搜索的正则模式 |
| `path` | string | 否 | 要搜索的目录或文件（默认为工作目录） |
| `include` | string | 否 | 用于过滤结果的文件 glob（例如 `*.rs`、`*.ts`） |

### 特性

- 使用 Rust 正则语法
- 返回匹配行及其文件路径与行号
- 结果上限为 200 条匹配，以保持响应可控
- 跳过二进制文件与常见的非文本扩展名（`.png`、`.jpg`、`.exe`、`.dll` 等）

### 示例

搜索函数定义：

```json
{
  "pattern": "fn\\s+build_system_prompt",
  "include": "*.rs"
}
```

在指定目录中搜索 TODO 注释：

```json
{
  "pattern": "TODO|FIXME|HACK",
  "path": "src/"
}
```

搜索 import 语句：

```json
{
  "pattern": "^use\\s+crate::",
  "include": "*.rs"
}
```

---

## glob

查找匹配某个 glob 模式的文件。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `pattern` | string | 是 | 要匹配的 glob 模式 |
| `path` | string | 否 | 要搜索的目录（默认为工作目录） |

### 模式语法

| 模式 | 含义 |
|---------|---------|
| `*` | 匹配任意字符（`/` 除外） |
| `**` | 递归匹配任意目录 |
| `?` | 匹配任意单个字符 |
| `[abc]` | 匹配集合中的任意字符 |
| `[a-z]` | 匹配范围内的任意字符 |

### 示例

查找所有 Rust 源文件：

```json
{
  "pattern": "**/*.rs"
}
```

查找测试文件：

```json
{
  "pattern": "**/test*.rs"
}
```

在指定目录中查找文件：

```json
{
  "pattern": "*.toml",
  "path": "crates/"
}
```

查找配置文件：

```json
{
  "pattern": "**/*.{json,yaml,yml,toml}"
}
```

---
title: Search
description: Find files and search content with grep and glob
---


## grep

Search for regex patterns across files in the project.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | Yes | Regex pattern to search for |
| `path` | string | No | Directory or file to search in (defaults to working directory) |
| `include` | string | No | File glob to filter results (e.g. `*.rs`, `*.ts`) |

### Features

- Uses Rust regex syntax
- Returns matching lines with file paths and line numbers
- Caps results at 200 matches to keep responses manageable
- Skips binary files and common non-text extensions (`.png`, `.jpg`, `.exe`, `.dll`, etc.)

### Examples

Search for a function definition:

```json
{
  "pattern": "fn\\s+build_system_prompt",
  "include": "*.rs"
}
```

Search for TODO comments in a specific directory:

```json
{
  "pattern": "TODO|FIXME|HACK",
  "path": "src/"
}
```

Search for import statements:

```json
{
  "pattern": "^use\\s+crate::",
  "include": "*.rs"
}
```

---

## glob

Find files matching a glob pattern.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | Yes | Glob pattern to match |
| `path` | string | No | Directory to search in (defaults to working directory) |

### Pattern Syntax

| Pattern | Meaning |
|---------|---------|
| `*` | Match any characters (except `/`) |
| `**` | Match any directories recursively |
| `?` | Match any single character |
| `[abc]` | Match any character in the set |
| `[a-z]` | Match any character in the range |

### Examples

Find all Rust source files:

```json
{
  "pattern": "**/*.rs"
}
```

Find test files:

```json
{
  "pattern": "**/test*.rs"
}
```

Find files in a specific directory:

```json
{
  "pattern": "*.toml",
  "path": "crates/"
}
```

Find configuration files:

```json
{
  "pattern": "**/*.{json,yaml,yml,toml}"
}
```

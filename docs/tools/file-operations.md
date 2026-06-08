---
title: File Operations
description: Read, write, and edit files
---

## file_read

Read the contents of a file. Supports reading specific line ranges.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | Yes | Path to the file (relative to working directory) |
| `offset` | integer | No | Line number to start reading from (0-based) |
| `limit` | integer | No | Maximum number of lines to read |

### Example

Reading a file with a line range:

```json
{
  "path": "src/main.rs",
  "offset": 10,
  "limit": 20
}
```

This reads lines 11-30 of `src/main.rs`.

---

## file_write

Create or overwrite a file. Automatically creates parent directories if they do not exist.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | Yes | Path to the file |
| `content` | string | Yes | Content to write |

### Example

```json
{
  "path": "src/utils.rs",
  "content": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
}
```

!!! warning
    `file_write` overwrites the entire file. For targeted edits, use `file_edit` or `apply_patch` instead.

---

## file_edit

Edit a file by replacing an exact string match. The `old_string` must match exactly once in the file.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | Yes | Path to the file |
| `old_string` | string | Yes | Exact text to find (must be unique in the file) |
| `new_string` | string | Yes | Replacement text |

### Example

```json
{
  "path": "src/main.rs",
  "old_string": "fn main() {\n    println!(\"Hello\");\n}",
  "new_string": "fn main() {\n    println!(\"Hello, world!\");\n}"
}
```

If `old_string` matches multiple locations or is not found, the edit fails with an error.

---

## apply_patch

Apply a unified diff patch. Supports Claude Code-style patch format with fuzzy matching for context lines.

### Patch Format

```
*** Begin Patch
*** Add File: path/to/new_file.rs
+line 1
+line 2
*** Update File: path/to/existing.rs
 context line
-old line
+new line
 context line
*** Delete File: path/to/old_file.rs
*** End Patch
```

### Patch Sections

| Section | Description |
|---------|-------------|
| `*** Add File` | Create a new file with the specified content |
| `*** Update File` | Modify an existing file using diff hunks |
| `*** Delete File` | Remove a file |

### Fuzzy Matching

When applying updates, the patch tool uses fuzzy matching for context lines. If the exact context is not found, it searches within a 3-line window to find the best match. This makes patches more resilient to minor whitespace or formatting differences.

### Example

```
*** Begin Patch
*** Update File: src/lib.rs
 use std::io;

-pub fn read_input() -> String {
+pub fn read_input() -> Result<String, io::Error> {
     let mut input = String::new();
-    std::io::stdin().read_line(&mut input).unwrap();
-    input
+    std::io::stdin().read_line(&mut input)?;
+    Ok(input)
 }
*** End Patch
```

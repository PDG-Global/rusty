---
title: 文件操作
description: 读取、写入与编辑文件
---


## file_read

读取文件内容。支持读取指定的行范围。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `path` | string | 是 | 文件路径（相对于工作目录） |
| `offset` | integer | 否 | 开始读取的行号（从 0 计） |
| `limit` | integer | 否 | 最多读取的行数 |

### 示例

按行范围读取文件：

```json
{
  "path": "src/main.rs",
  "offset": 10,
  "limit": 20
}
```

这会读取 `src/main.rs` 的第 11–30 行。

---

## file_write

创建或覆盖文件。若父目录不存在会自动创建。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `path` | string | 是 | 文件路径 |
| `content` | string | 是 | 要写入的内容 |

### 示例

```json
{
  "path": "src/utils.rs",
  "content": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
}
```

:::caution
`file_write` 会覆盖整个文件。若需定点编辑，请改用 `file_edit` 或 `apply_patch`。
:::

---

## file_edit

通过替换精确匹配的字符串来编辑文件。`old_string` 必须在文件中恰好匹配一次。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|-----------|------|----------|-------------|
| `path` | string | 是 | 文件路径 |
| `old_string` | string | 是 | 要查找的精确文本（在文件中必须唯一） |
| `new_string` | string | 是 | 替换文本 |

### 示例

```json
{
  "path": "src/main.rs",
  "old_string": "fn main() {\n    println!(\"Hello\");\n}",
  "new_string": "fn main() {\n    println!(\"Hello, world!\");\n}"
}
```

若 `old_string` 匹配到多处或未找到，编辑将以错误告终。

---

## apply_patch

应用统一 diff 补丁。支持 Claude Code 风格的补丁格式，对上下文行进行模糊匹配。

### 补丁格式

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

### 补丁段

| 段 | 说明 |
|---------|-------------|
| `*** Add File` | 以指定内容创建新文件 |
| `*** Update File` | 使用 diff hunk 修改现有文件 |
| `*** Delete File` | 删除文件 |

### 模糊匹配

应用更新时，补丁工具会对上下文行进行模糊匹配。若找不到精确的上下文，它会在 3 行的窗口内搜索最佳匹配。这让补丁对细微的空白或格式差异更具鲁棒性。

### 示例

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

---

## 路径沙箱

所有文件工具都通过 `resolve_path()` 强制执行路径沙箱：

- 路径会被规范化，解析符号链接与 `..` 组件。
- 任何逃逸出工作目录的路径都会被拒绝。
- **TOCTOU 加固**：实现避免在 `canonicalize()` 之前检查 `path.exists()`，以防止符号链接竞态。写入前校验（`verify_not_escaping_symlink()`）在创建文件前运行，写入后再校验（`verify_no_symlink_escape()`）在写入后运行，以防范符号链接攻击。

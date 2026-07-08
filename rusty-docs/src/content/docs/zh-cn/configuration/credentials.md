---
title: 凭据
description: API 密钥管理与凭据存储
---


## 凭据如何解析

Rusty 通过一条分级链来解析 API 密钥，按顺序检查每个来源。使用找到的第一个非空值。

### 解析顺序

1.  **环境变量**

    先检查 `RUSTY_API_KEY`，再检查 `OPENAI_API_KEY`。空字符串视为不存在。

2.  **操作系统钥匙串**

    若设置中 `credential_store` 为 `"keyring"`，Rusty 从系统钥匙串读取：

    - **macOS**：钥匙串访问（Keychain Access）
    - **Windows**：凭据管理器（Credential Manager）
    - **Linux**：Secret Service（GNOME Keyring、KWallet）

3.  **设置文件**

    最终回退使用 `~/.rusty/settings.json` 中的 `api_key` 字段。

## 凭据存储选项

### 钥匙串（推荐）

将 API 密钥存储在操作系统的安全凭据库中。这是默认且推荐的选项。

```json
{
  "credential_store": "keyring"
}
```

钥匙串由设置向导自动管理。你也可以以编程方式管理：

```bash
# 检查钥匙串是否可用
rusty --setup
```

### 设置文件

将 API 密钥以明文形式存储在 `~/.rusty/settings.json` 中。若你的平台没有可用的钥匙串（例如无桌面环境的无头 Linux），可使用此方式。

```json
{
  "credential_store": "settings_file",
  "api_key": "sk-..."
}
```

:::caution
使用 `settings_file` 模式时，API 密钥以明文存储。请确保对 `~/.rusty/settings.json` 设置了合适的文件权限。
:::

## 每模型 API 密钥

若你使用多个提供方，可在 `api_keys` 字段中为每个模型存储独立的 API 密钥：

```json
{
  "api_keys": {
    "mimo-v2.5-pro": "sk-mimo-...",
    "gpt-4o": "sk-openai-..."
  }
}
```

当匹配的模型处于活动状态时，`api_keys` 中的每模型密钥优先于顶层的 `api_key` 字段。

## 多提供方配置

若你在多个提供方之间切换，可用环境变量存储不同的凭据：

```bash
export OPENAI_API_KEY=sk-openai-...
export RUSTY_API_KEY=sk-other-...
```

`RUSTY_API_KEY` 始终优先于 `OPENAI_API_KEY`。

## 编程访问

`rusty-core` 中的 `CredentialManager` 提供了一些辅助函数：

| 函数 | 说明 |
|----------|-------------|
| `resolve_api_key(settings)` | 完整的分级解析（环境变量、钥匙串、设置） |
| `store_in_keyring(key)` | 在操作系统钥匙串中存储密钥 |
| `delete_from_keyring()` | 从操作系统钥匙串中删除密钥 |
| `is_keyring_available()` | 检查操作系统钥匙串是否可访问 |

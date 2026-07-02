---
title: Credentials
description: API key management and credential storage
---

## How Credentials Are Resolved

Rusty resolves API keys through a tiered chain, checking each source in order. The first non-empty value found is used.

### Resolution Order

1.  **Environment Variables**

    `RUSTY_API_KEY` is checked first, then `OPENAI_API_KEY`. Empty strings are treated as absent.

2.  **OS Keyring**

    If `credential_store` is set to `"keyring"` in settings, Rusty reads from the system keyring:

    - **macOS**: Keychain Access
    - **Windows**: Credential Manager
    - **Linux**: Secret Service (GNOME Keyring, KWallet)

3.  **Settings File**

    The `api_key` field in `~/.rusty/settings.json` is used as a final fallback.

## Credential Store Options

### Keyring (Recommended)

Stores the API key in the operating system's secure credential store. This is the default and recommended option.

```json
{
  "credential_store": "keyring"
}
```

The keyring is managed automatically by the setup wizard. You can also manage it programmatically:

```bash
# Check if keyring is available
rusty --setup
```

### Settings File

Stores the API key in plaintext in `~/.rusty/settings.json`. Use this if your platform does not have a keyring available (e.g., headless Linux without a desktop environment).

```json
{
  "credential_store": "settings_file",
  "api_key": "sk-..."
}
```

!!! warning
    When using `settings_file` mode, the API key is stored in plaintext. Ensure appropriate file permissions on `~/.rusty/settings.json`.

## Per-Model API Keys

If you use multiple providers, you can store separate API keys per model in the `api_keys` field:

```json
{
  "api_keys": {
    "mimo-v2.5-pro": "sk-mimo-...",
    "gpt-4o": "sk-openai-..."
  }
}
```

Per-model keys in `api_keys` take precedence over the top-level `api_key` field when the matching model is active.

## Multi-Provider Setup

If you switch between providers, you can store separate credentials using environment variables:

```bash
export OPENAI_API_KEY=sk-openai-...
export RUSTY_API_KEY=sk-other-...
```

`RUSTY_API_KEY` always takes precedence over `OPENAI_API_KEY`.

## Programmatic Access

The `CredentialManager` in `rusty-core` provides helper functions:

| Function | Description |
|----------|-------------|
| `resolve_api_key(settings)` | Full tiered resolution (env, keyring, settings) |
| `store_in_keyring(key)` | Store a key in the OS keyring |
| `delete_from_keyring()` | Remove the key from the OS keyring |
| `is_keyring_available()` | Check if the OS keyring is accessible |

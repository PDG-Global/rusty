---
title: Quickstart
description: Get Rusty running in under 5 minutes
---

## Prerequisites

- Rust 1.75+ (2021 edition)
- An API key for a supported LLM provider

## Install

### From Source

```bash
git clone https://github.com/pdg-global/rusty.git
cd rusty
cargo build --release
```

The binary will be at `./target/release/rusty`.

### Via Cargo Install

```bash
cargo install rusty
```

## First Run

When you run Rusty for the first time, the setup wizard launches automatically:

```bash
rusty
```

The wizard will:

1. **Select a provider** from Xiaomi, Kimi, OpenAI, DeepSeek, Ollama, or a custom endpoint
2. **Enter your API key** (masked input)
3. **Choose credential storage** (OS keyring or settings file)
4. **Select a model** for the chosen provider
5. **Test connectivity** to verify the setup works

Configuration is saved to `~/.rusty/settings.json`.

## Using a Preset

Skip the wizard by specifying a preset and API key directly:

```bash
rusty --preset openai --api-key sk-...
```

Available presets: `xiaomi`, `kimi`, `openai`, `deepseek`, `ollama`.

## Environment Variables

Set your API key via environment variables instead of flags:

```bash
export OPENAI_API_KEY=sk-...
rusty --preset openai
```

`RUSTY_API_KEY` takes precedence over `OPENAI_API_KEY` if both are set.

## Your First Conversation

Once running, just type your prompt and press Enter. Rusty will stream the response in real-time and offer to execute tools when needed.

Try asking:
- "What files are in this directory?"
- "Read the Cargo.toml and explain the dependencies"
- "Create a hello world script in Python"

Use `/help` to see available slash commands, or `/quit` to exit (session is saved automatically).

---
title: Installation
description: All the ways to install Rusty
---

## From Source

```bash
git clone https://github.com/pdg-global/rusty.git
cd rusty
cargo build --release
```

The binary is produced at `./target/release/rusty`. Move it somewhere on your `PATH`:

```bash
# macOS / Linux
sudo cp ./target/release/rusty /usr/local/bin/

# Or add the target directory to your PATH
export PATH="$PWD/target/release:$PATH"
```

## Pre-built Binaries

Pre-built binaries are available on the [GitHub releases page](https://github.com/PDG-Global/rusty/releases). Download the appropriate binary for your platform and add it to your `PATH`.

## Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| macOS | aarch64 (Apple Silicon) | Fully supported |
| macOS | x86_64 (Intel) | Fully supported |
| macOS | Universal (arm64 + x86_64) | Fully supported |
| Linux | x86_64 (GNU libc) | Fully supported |
| Linux | aarch64 (GNU libc) | Fully supported |
| Linux | armv7 (GNU libc) | Fully supported |
| Linux | x86_64 (musl, static) | Fully supported |
| Linux | aarch64 (musl, static) | Fully supported |
| FreeBSD | x86_64 | Fully supported |

!!! note
    macOS binaries are code-signed and notarised. Static Linux (musl) builds have no glibc dependency and work on minimal containers.

## Dependencies

Rusty builds as a single statically compiled binary with no runtime dependencies. All native dependencies (OpenSSL, etc.) are vendored via the Rust crate ecosystem.

### Build Dependencies

- Rust toolchain 1.75+ (edition 2021)
- A C compiler (for some vendored C libraries on Linux)

Install Rust via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Verifying the Installation

After installing, verify Rusty works:

```bash
rusty --help
```

This displays all available CLI flags and run modes.

## Shell Completions

Rusty supports shell completions via clap. Generate them for your shell:

```bash
# Bash
rusty --completions bash > ~/.bash_completion.d/rusty

# Zsh
rusty --completions zsh > ~/.zfunc/_rusty

# Fish
rusty --completions fish > ~/.config/fish/completions/rusty.fish
```

!!! note
    If shell completions are not yet wired up, you can request them as a feature on GitHub.

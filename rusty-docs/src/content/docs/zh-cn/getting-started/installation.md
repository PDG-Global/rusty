---
title: 安装
description: 安装 Rusty 的所有方式
---


## 从源码构建

```bash
git clone https://github.com/pdg-global/rusty.git
cd rusty
cargo build --release
```

构建产物位于 `./target/release/rusty`。将它移动到 `PATH` 中的某个目录：

```bash
# macOS / Linux
sudo cp ./target/release/rusty /usr/local/bin/

# 或者把 target 目录加入 PATH
export PATH="$PWD/target/release:$PATH"
```

## 预编译二进制文件

预编译的二进制文件可在 [GitHub releases 页面](https://github.com/PDG-Global/rusty/releases) 获取。下载适合你平台的版本并加入 `PATH` 即可。

## 平台支持

| 平台 | 架构 | 状态 |
|----------|-------------|--------|
| macOS | aarch64（Apple Silicon） | 完整支持 |
| macOS | x86_64（Intel） | 完整支持 |
| macOS | 通用（arm64 + x86_64） | 完整支持 |
| Linux | x86_64（GNU libc） | 完整支持 |
| Linux | aarch64（GNU libc） | 完整支持 |
| Linux | armv7（GNU libc） | 完整支持 |
| Linux | x86_64（musl，静态） | 完整支持 |
| Linux | aarch64（musl，静态） | 完整支持 |
| FreeBSD | x86_64 | 完整支持 |

:::note
macOS 二进制文件经过代码签名与公证。静态 Linux（musl）构建不依赖 glibc，可在最小化容器中运行。
:::

## 依赖

Rusty 编译为单个静态链接的二进制文件，没有运行时依赖。所有原生依赖（OpenSSL 等）都通过 Rust crate 生态内联打包。

### 构建依赖

- Rust 工具链 1.75+（2021 edition）
- 一个 C 编译器（Linux 上部分内联 C 库需要）

通过 [rustup](https://rustup.rs/) 安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## 验证安装

安装完成后，验证 Rusty 是否可用：

```bash
rusty --help
```

这会显示所有可用的 CLI 参数与运行模式。

## Shell 补全

Rusty 通过 clap 支持 shell 补全。为你的 shell 生成补全脚本：

```bash
# Bash
rusty --completions bash > ~/.bash_completion.d/rusty

# Zsh
rusty --completions zsh > ~/.zfunc/_rusty

# Fish
rusty --completions fish > ~/.config/fish/completions/rusty.fish
```

:::note
如果 shell 补全尚未接入，你可以在 GitHub 上作为功能请求提出。
:::

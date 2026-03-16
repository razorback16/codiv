---
title: Installation
description: Install Codiv from source and verify it is working.
section: Getting Started
order: 1
---

## Prerequisites

Before installing Codiv, make sure you have:

- **An LLM API key** from at least one supported provider:
  - [Anthropic](https://console.anthropic.com/) (Claude)
  - [OpenAI](https://platform.openai.com/) (GPT)
  - [Google](https://aistudio.google.com/) (Gemini)

## Homebrew (macOS / Linux)

The easiest way to install Codiv:

```bash
brew tap razorback16/codiv https://github.com/razorback16/codiv.git
brew install codiv
```

This installs both `codiv` (TUI client) and `codivd` (daemon).

## Cargo Install (from Git)

If you have the [Rust toolchain](https://rustup.rs/) installed:

```bash
cargo install --git https://github.com/razorback16/codiv.git codiv
cargo install --git https://github.com/razorback16/codiv.git codivd
```

Both commands are needed — `codiv` is the TUI client and `codivd` is the daemon it connects to.

## Build from Source

Clone the repository and build all workspace crates:

```bash
git clone https://github.com/razorback16/codiv.git
cd codiv
cargo build --workspace --release
ln -sf "$(pwd)/target/release/codiv" /usr/local/bin/codiv
ln -sf "$(pwd)/target/release/codivd" /usr/local/bin/codivd
```

This compiles the `codiv` client, `codivd` daemon, and all shared library crates, then symlinks both binaries to your `PATH`.

## Verify

Confirm the installation worked:

```bash
codiv --help
```

You should see the Codiv help output listing available subcommands and flags. If you see a "command not found" error, make sure `/usr/local/bin` is on your `PATH`.

## Next Steps

Head to the [Quick Start](/docs/getting-started/quick-start/) guide to launch Codiv and try your first commands.

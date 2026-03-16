---
title: Installation
description: Install Codiv from source and verify it is working.
section: Getting Started
order: 1
---

## Prerequisites

Before installing Codiv, make sure you have:

- **Rust toolchain** (stable) — install via [rustup](https://rustup.rs/)
- **An LLM API key** from at least one supported provider:
  - [Anthropic](https://console.anthropic.com/) (Claude)
  - [OpenAI](https://platform.openai.com/) (GPT)
  - [Google](https://aistudio.google.com/) (Gemini)

## Build from Source

Clone the repository and build all workspace crates:

```bash
git clone https://github.com/razorback16/slate-agent.git
cd slate-agent
cargo build --workspace
```

This compiles the `codiv` client, `codivd` daemon, and all shared library crates.

## Install

For a release build with optimizations, run:

```bash
cargo build --workspace --release
ln -sf "$(pwd)/target/release/codiv" /usr/local/bin/codiv
```

This creates a symlink so `codiv` is available on your `PATH`.

## Verify

Confirm the installation worked:

```bash
codiv --help
```

You should see the Codiv help output listing available subcommands and flags. If you see a "command not found" error, make sure `/usr/local/bin` is on your `PATH`.

## Next Steps

Head to the [Quick Start](/docs/getting-started/quick-start/) guide to launch Codiv and try your first commands.

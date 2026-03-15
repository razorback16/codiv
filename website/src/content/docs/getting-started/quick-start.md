---
title: Quick Start
description: Launch Codiv and try your first shell and AI commands.
section: Getting Started
order: 2
---

## Launch Codiv

Start the Codiv TUI by running:

```bash
codiv
```

This opens an interactive terminal that replaces your shell. Commands you know work instantly; everything else routes to the AI agent.

## Try a Shell Command

Type a standard shell command and press Enter:

```bash
ls -la
```

Codiv recognizes this as a known command and executes it with near-zero overhead — no AI round-trip needed. The output appears inline just like a regular terminal.

## Try an AI Query

Type a natural language request:

```
find all Rust files that contain "async fn" and list them
```

Codiv classifies this as an AI query and routes it to the agent. The agent reasons about the task, calls tools (like `grep` and `glob`), and streams the response back to your terminal.

## Try CLI Tools

Codiv's built-in tools are also accessible as subcommands:

```bash
# Read a file with line numbers
codiv read --path ./src/main.rs

# Search with regex
codiv grep --pattern "fn main" --path ./src

# Find files by glob
codiv glob --pattern "**/*.rs"

# Run a shell command through the tool interface
codiv bash --command "cargo test"
```

## JSON Mode

For programmatic use, pipe JSON in and out:

```bash
echo '{"file_path":"./Cargo.toml"}' | codiv read --json-in --json-out
```

## Next Steps

See the [Configuration](/slate-agent/docs/getting-started/configuration/) guide to set up your API keys and customize Codiv's behavior.

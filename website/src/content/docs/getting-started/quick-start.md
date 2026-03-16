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

This opens an interactive terminal with two explicit modes. **AI Mode** is active by default — you'll see a cyan `>` gutter on the input line. Press **Tab on an empty input** to toggle into **Command Mode** (white `$` gutter), which sends input directly to a bash co-process. Press Tab again to switch back.

## Try an AI Query

AI Mode is the default. With the `>` gutter showing, type a natural language request:

```
find all Rust files that contain "async fn" and list them
```

The agent reasons about the task, calls tools (like `grep` and `glob`), and streams the response back to your terminal.

## Switch to Command Mode

Press **Tab** on an empty input line. The gutter changes from `>` (cyan) to `$` (white), indicating you are now in Command Mode. All input is sent directly to the bash co-process.

## Try a Shell Command

With the `$` gutter showing, type a standard shell command and press Enter:

```bash
ls -la
```

The command executes with near-zero overhead — no AI round-trip needed. The output appears inline just like a regular terminal.

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

See the [Configuration](/docs/getting-started/configuration/) guide to set up your API keys and customize Codiv's behavior.

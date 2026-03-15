---
title: Tools
description: Built-in tools available to both the AI agent and CLI users.
section: Usage
order: 3
---

## Overview

Codiv ships with six built-in tools. Each tool is available in two ways:

- **Agent use** — the AI agent calls tools during its reasoning loop
- **CLI use** — you run tools directly as `codiv <tool>` subcommands

## Common Flags

Every tool supports these flags:

| Flag | Purpose |
|------|---------|
| `--json-in` | Read JSON input from stdin |
| `--json-out` | Output result as JSON (`{"result": ..., "error": ...}`) |
| `--agent-guide` | Print the agent usage guide for this tool |
| `--help` | Print help text |

## Tool Reference

### Bash

Execute a shell command:

```bash
codiv bash --command "cargo test --workspace"
codiv bash --command "ls -la" --timeout 5000
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--command` | Yes | The shell command to execute |
| `--timeout` | No | Timeout in milliseconds |

### Read

Read file contents with line numbers:

```bash
codiv read --path ./src/main.rs
codiv read --path ./src/main.rs --offset 10 --limit 50
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--path` | Yes | File path to read |
| `--offset` | No | Line number to start from |
| `--limit` | No | Maximum number of lines |

### Write

Write content to a file (creates or overwrites):

```bash
codiv write --path ./output.txt --content "Hello, world!"
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--path` | Yes | File path to write |
| `--content` | Yes | Content to write |

### Edit

Find and replace a unique string in a file:

```bash
codiv edit --path ./src/main.rs --old-string "fn old_name" --new-string "fn new_name"
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--path` | Yes | File path to edit |
| `--old-string` | Yes | Text to find (must be unique in the file) |
| `--new-string` | Yes | Replacement text |

### Glob

Find files matching a pattern:

```bash
codiv glob --pattern "**/*.rs"
codiv glob --pattern "src/**/*.toml" --path ./my-project
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--pattern` | Yes | Glob pattern |
| `--path` | No | Directory to search in (defaults to cwd) |

### Grep

Search file contents with regex:

```bash
codiv grep --pattern "fn main" --path ./src
codiv grep --pattern "TODO" --include "*.rs"
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `--pattern` | Yes | Regex pattern to search for |
| `--path` | No | File or directory to search in |
| `--include` | No | Glob to filter files |

## JSON Mode

For programmatic or scripted use, combine `--json-in` and `--json-out`:

```bash
echo '{"file_path":"./Cargo.toml"}' | codiv read --json-in --json-out
```

Output:

```json
{
  "result": "     1\t[workspace]\n     2\tmembers = [\"codiv\", \"codivd\", ...]\n",
  "error": null
}
```

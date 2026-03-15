---
title: Configuration
description: Set up API keys, models, and safety settings in the Codiv config file.
section: Getting Started
order: 3
---

## Config File Location

Codiv reads its configuration from:

```
~/.codiv/config.toml
```

If this file does not exist, Codiv uses sensible defaults but you will need to provide at least one API key for AI features to work.

## API Key Setup

Add your API key for one or more providers:

```toml
[providers.anthropic]
api_key = "sk-ant-..."

[providers.openai]
api_key = "sk-..."

[providers.google]
api_key = "AIza..."
```

You can also set keys via environment variables:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-..."
export GOOGLE_API_KEY="AIza..."
```

Environment variables take precedence over the config file.

## Model Selection

Choose which model to use for AI queries:

```toml
[model]
provider = "anthropic"
name = "claude-sonnet-4-20250514"
```

Supported providers: `anthropic`, `openai`, `google`.

## Safety Settings

Control which commands require confirmation before execution:

```toml
[safety]
# Commands that always run without confirmation
allowlist = [
  "ls", "cat", "echo", "pwd", "head", "tail", "wc",
  "grep", "find", "which", "whoami", "date",
]

# Commands that are always blocked
denylist = [
  "rm -rf /",
  "mkfs",
  "dd if=/dev/zero",
]
```

Commands not on either list are classified by the risk engine into four tiers: low, medium, high, and critical. Medium and above require user confirmation.

## Debug Mode

Enable debug logging to troubleshoot issues:

```bash
# Default debug level
codiv --debug

# Trace-level logging (very verbose)
codiv --debug=trace
```

Debug logs are written to `/tmp/codiv-debug.log`.

## Config Hot-Reload

Codiv watches the config file for changes. When you edit `~/.codiv/config.toml`, most settings take effect immediately without restarting — including model selection and safety lists.

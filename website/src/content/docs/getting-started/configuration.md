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

## Authentication

The recommended way to configure provider credentials is the interactive login wizard:

```bash
# Interactive provider selection
codiv login

# Login to a specific provider
codiv login anthropic
codiv login openai
```

The wizard supports both API key entry and OAuth code flows (for Claude Code and Codex). Credentials are stored in `~/.codiv/config.toml`.

For OAuth providers, tokens are stored under `[auth.tokens.{provider_id}]` in the config file and are automatically refreshed by the daemon in the background, so you never need to re-authenticate unless a refresh token is revoked.

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

If you already have API keys set as environment variables and want to migrate them into `config.toml`, run:

```bash
codiv migrate-env
```

This detects `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, and `GOOGLE_API_KEY` in your environment and writes them to the config file.

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

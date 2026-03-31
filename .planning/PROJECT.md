# Codiv Auth — AI Provider Login System

## What This Is

A unified authentication system for Codiv that adds a `codiv login` CLI command with an interactive wizard for configuring AI provider credentials. Ported and adapted from the forgecode project's auth infrastructure, it supports API key entry and OAuth code flows for Anthropic (Claude Code), OpenAI (Codex), and other providers. Credentials are stored in `~/.codiv/config.toml` alongside existing configuration.

## Core Value

Users can authenticate with any supported AI provider through a single `codiv login` command — no manual config file editing, no hunting for environment variable names, no expired tokens breaking sessions.

## Requirements

### Validated

- Existing multi-provider support (Anthropic, OpenAI, Google, OpenAI-compatible) via config.toml and env vars
- Daemon-based architecture with codivd handling API calls
- aisdk library for unified LLM interface

### Active

- [ ] `codiv login` CLI command with interactive provider wizard
- [ ] API key auth method (paste key, validate, store)
- [ ] OAuth code flow auth method (browser redirect, code exchange)
- [ ] Provider registry with auth method metadata (Anthropic, OpenAI, Claude Code OAuth, Codex OAuth)
- [ ] Credential storage merged into `~/.codiv/config.toml`
- [ ] Token refresh handled automatically by codivd daemon
- [ ] Interactive prompts for provider/method selection (dialoguer style)
- [ ] `codiv login <provider>` shortcut to skip provider selection
- [ ] Credential migration from env vars to config.toml

### Out of Scope

- OAuth device flow (GitHub Copilot style) — not needed for v1 providers
- Google ADC — defer until Vertex AI demand exists
- TUI-inline login — CLI subcommand is sufficient for v1
- fzf-based selection — using dialoguer/inquire for interactive prompts instead
- Shared auth crate between codiv and forgecode — port and adapt, not share

## Context

**Source project:** forgecode at `/Users/subhagato/Development/github/forgecode`
- Auth crates: `forge_infra/src/auth/`, `forge_domain/src/auth/`, `forge_services/src/provider_auth.rs`, `forge_repo/src/provider/`
- Provider catalog: `forge_repo/src/provider/provider.json`
- Login UX: `forge_main/src/ui.rs` (`handle_provider_login`, `handle_code_flow`, `handle_api_key_input`)
- Credential storage: `~/.forge/.credentials.json` (JSON format — codiv will use TOML)

**Codiv architecture:**
- Two-process: codiv (TUI) + codivd (daemon)
- Config: `~/.codiv/config.toml` (TOML, hot-reloads)
- API calls go through codivd → aisdk
- Token refresh will live in codivd since it already owns the API call lifecycle

**Approach:** Port relevant Rust modules from forgecode's auth system, adapt to codiv's architecture (TOML config, daemon-based refresh, IPC protocol). The wizard UX will use dialoguer for interactive prompts rather than forgecode's fzf approach.

## Constraints

- **Config format**: TOML (not JSON like forgecode) — must merge auth into existing config.toml
- **Architecture**: Token refresh must be in codivd daemon, not client
- **UX**: CLI subcommand (`codiv login`), not TUI inline
- **Dependencies**: Minimize new dependencies — prefer reusing what's already in the workspace
- **Security**: API keys and tokens must not be logged or exposed in debug output

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| Port & adapt from forgecode | Proven auth flows, avoid reinventing OAuth | — Pending |
| Merge auth into config.toml | Single config file, hot-reload support already exists | — Pending |
| Daemon-side token refresh | Daemon owns API calls, can refresh proactively | — Pending |
| dialoguer for interactive prompts | Standard Rust CLI prompt library, no external deps like fzf | — Pending |
| API Key + OAuth Code Flow only | Covers Anthropic, OpenAI, Claude Code, Codex — expand later | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd:transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd:complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-03-31 after initialization*

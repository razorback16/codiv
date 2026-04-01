---
phase: 03-cli-wizard-daemon-refresh
plan: "01"
subsystem: cli/login
tags: [cli, ux, dialoguer, wizard, auth]
dependency_graph:
  requires: []
  provides: [interactive-login-wizard]
  affects: [crates/codiv/src/cli/login.rs]
tech_stack:
  added: []
  patterns: [dialoguer::Select for interactive provider selection]
key_files:
  created: []
  modified:
    - crates/codiv/src/cli/login.rs
decisions:
  - "Display auth method before running flow — single println! before each match arm, no UX overhead"
  - "registry.into_iter().nth(selection) used to retrieve selected ProviderEntry — avoids clone and index re-lookup"
metrics:
  duration: "45s"
  completed: "2026-04-01"
  tasks: 1
  files: 1
requirements:
  - CLI-01
  - CLI-02
  - CLI-03
  - CLI-04
---

# Phase 03 Plan 01: Interactive Login Wizard Summary

Interactive `codiv login` wizard using dialoguer::Select for arrow-key provider navigation with auth method display before flow execution.

## What Was Built

Upgraded `crates/codiv/src/cli/login.rs` from a static text list to a full interactive dialoguer wizard:

- `codiv login` (no args): presents `Select::new().with_prompt("Select a provider")` arrow-key menu listing all four providers (Anthropic, OpenAI, Claude Code, Codex)
- `codiv login <provider>`: skips wizard, resolves provider via `provider_by_id()`, continues directly to auth
- Before each auth flow, prints one of:
  - `Auth method: API key` (Anthropic, OpenAI)
  - `Auth method: OAuth (browser redirect)` (Claude Code, Codex)
- `run_api_key_flow` and `run_oauth_flow` are unchanged — only the routing logic in `run()` was modified

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Upgrade provider selection to dialoguer Select wizard | 1fa19cd | crates/codiv/src/cli/login.rs |

## Deviations from Plan

None - plan executed exactly as written.

## Known Stubs

None.

## Self-Check: PASSED

- File exists: crates/codiv/src/cli/login.rs — FOUND
- Commit 1fa19cd — FOUND (git log confirms)
- `Select::new()` present in login.rs — CONFIRMED
- Two `Auth method:` println lines present — CONFIRMED
- `cargo build -p codiv` exits 0 — CONFIRMED

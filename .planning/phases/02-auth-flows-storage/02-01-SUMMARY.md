---
phase: 02-auth-flows-storage
plan: "01"
subsystem: codiv-common/auth
tags: [credential-storage, toml, oauth, types]
dependency_graph:
  requires: [01-auth-foundation/01-01-SUMMARY.md, 01-auth-foundation/01-02-SUMMARY.md]
  provides: [write_api_key_to_config, write_oauth_tokens_to_config, read_oauth_tokens_from_config, OAuthTokenResponse]
  affects: [crates/codiv-common, crates/codiv]
tech_stack:
  added: [toml_edit=0.22, toml=0.8, oauth2=5.0.0, open=5.3.3, reqwest=0.12, anyhow=1, tokio=1, dialoguer=0.12]
  patterns: [TOML format-preserving merge, HOME env isolation with Mutex for tests]
key_files:
  created:
    - crates/codiv-common/src/auth/storage.rs
  modified:
    - crates/codiv-common/src/auth/types.rs
    - crates/codiv-common/src/auth/mod.rs
    - crates/codiv-common/Cargo.toml
    - crates/codiv/Cargo.toml
decisions:
  - "Used static Mutex in tests to serialize HOME env var mutations across threads, enabling safe parallel test execution without requiring --test-threads=1"
  - "Added oauth2, open, reqwest, anyhow, tokio, toml_edit, toml to codiv-common so later plans can build auth flows against these deps without re-adding them"
metrics:
  duration: "3m 10s"
  completed: "2026-04-01"
  tasks_completed: 2
  files_created: 1
  files_modified: 4
---

# Phase 02 Plan 01: Credential Storage Foundation Summary

**One-liner:** TOML format-preserving credential storage with OAuthTokenResponse type and new auth deps (oauth2, open, reqwest, dialoguer) added to workspace.

## What Was Built

Task 1 added `OAuthTokenResponse` (6-field struct: access_token, refresh_token, expires_in, expires_at, token_type, scope) to `types.rs` and added all dependencies needed by later auth plans (oauth2, open, reqwest, anyhow, tokio) to `codiv-common/Cargo.toml`, plus `dialoguer` and `anyhow` to `codiv/Cargo.toml`.

Task 2 created `crates/codiv-common/src/auth/storage.rs` with three credential storage functions that use `toml_edit` for format-preserving TOML merges — the same pattern as `add_permission_to_config()` in `codivd`. All functions exported from `codiv-common::auth`.

## Storage Functions

- `write_api_key_to_config(provider_id, api_key)` — writes `[providers.{id}] api_key = "..."` without touching other sections
- `write_oauth_tokens_to_config(provider_id, tokens)` — writes `[auth.tokens.{id}]` with access_token, refresh_token (if Some), expires_at as RFC3339; also mirrors access_token to `[providers.{id}] api_key` for daemon hot-reload
- `read_oauth_tokens_from_config(provider_id)` — returns `Ok(Some(OAuthTokens))` if stored, `Ok(None)` if missing

## Tests

All 44 tests pass (39 existing + 5 new storage tests). The `with_temp_config` helper uses a static `Mutex` to serialize tests that mutate the `HOME` env var, enabling safe parallel test execution.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Test thread-safety via Mutex instead of test-threads=1**
- **Found during:** Task 2 verification
- **Issue:** 3 storage tests failed when run in parallel because multiple threads were mutating the `HOME` env var simultaneously via `std::env::set_var`
- **Fix:** Added `static HOME_LOCK: Mutex<()>` to serialize the storage tests that mutate HOME, allowing `cargo test --package codiv-common` to pass without `--test-threads=1`
- **Files modified:** `crates/codiv-common/src/auth/storage.rs`
- **Commit:** 15d951d

## Commits

| Task | Commit | Message |
|------|--------|---------|
| Task 1 | cc039b6 | feat(02-01): add OAuthTokenResponse type and new dependencies |
| Task 2 | 15d951d | feat(02-01): implement credential storage with TOML merge writes |

## Self-Check: PASSED

- [x] `crates/codiv-common/src/auth/storage.rs` exists
- [x] `OAuthTokenResponse` in `types.rs` with all 6 fields
- [x] Storage functions exported from `codiv-common::auth` (mod.rs line 11)
- [x] `cargo test --package codiv-common` exits 0 (44 tests pass)
- [x] `cargo build --workspace` exits 0

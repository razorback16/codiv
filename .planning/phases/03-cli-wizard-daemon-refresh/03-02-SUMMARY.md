---
phase: 03-cli-wizard-daemon-refresh
plan: 02
subsystem: auth
tags: [oauth, token-refresh, daemon, background-task, reqwest]

# Dependency graph
requires:
  - phase: 02-auth-flows-storage
    provides: OAuthTokens types, read/write_oauth_tokens_from_config, provider registry with OAuthConfig

provides:
  - refresh_oauth_token() async fn in codiv-common/auth/flows.rs — form-encoded POST for token refresh
  - maybe_refresh_stored_token() in codiv-common — reads stored tokens, checks 5min expiry, refreshes and writes back
  - spawn_token_refresh_task() in codivd — background task checking claude_code and codex every 4 minutes
  - _token_refresh_task JoinHandle on Daemon struct to keep background task alive

affects: [codivd, codiv-common, daemon-startup, oauth-providers]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - Background tokio task owned by Daemon struct via JoinHandle field to prevent early drop
    - Graceful degradation pattern — token refresh failures logged as warnings, never abort API call

key-files:
  created: []
  modified:
    - crates/codiv-common/src/auth/flows.rs
    - crates/codiv-common/src/auth/mod.rs
    - crates/codivd/src/agent/config.rs
    - crates/codivd/src/daemon.rs

key-decisions:
  - "eprintln! used instead of tracing::warn! in codiv-common (tracing not a dep of codiv-common)"
  - "Background refresh task polls every 4 minutes with 5-minute expiry buffer, giving 1-minute overlap for slow refreshes"
  - "refresh_oauth_token uses form-encoded body (standard RFC 6749) — token_refresh_url falls back to token_url since both claude_code and codex have token_refresh_url = None"

patterns-established:
  - "Token refresh: check needs_refresh(5min) -> refresh via HTTP -> write new tokens to config.toml -> return new access token"
  - "Daemon background tasks: stored as JoinHandle field with _ prefix to prevent unused-variable warnings while keeping task alive"

requirements-completed: [STOR-02]

# Metrics
duration: 2min
completed: 2026-04-01
---

# Phase 03 Plan 02: Daemon OAuth Token Refresh Summary

**Background OAuth token refresh for codivd using form-encoded grant_type=refresh_token POST with 5-minute proactive expiry buffer**

## Performance

- **Duration:** 2 min
- **Started:** 2026-04-01T17:56:23Z
- **Completed:** 2026-04-01T17:57:55Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments

- Added `refresh_oauth_token()` async function in codiv-common that sends a form-encoded POST with `grant_type=refresh_token` to the provider's token endpoint
- Added `maybe_refresh_stored_token()` that checks stored tokens with a 5-minute buffer and writes new tokens to config.toml on success
- Added `spawn_token_refresh_task()` in codivd that loops every 4 minutes checking claude_code and codex OAuth tokens for upcoming expiry
- Wired the background task into `Daemon::new()` storing the JoinHandle as `_token_refresh_task` to keep it alive for the daemon lifetime

## Task Commits

Each task was committed atomically:

1. **Task 1: Add refresh_oauth_token() to codiv-common flows** - `624c0cf` (feat)
2. **Task 2: Call maybe_refresh_stored_token from codivd before API calls** - `573122b` (feat)

**Plan metadata:** (docs commit follows)

## Files Created/Modified

- `crates/codiv-common/src/auth/flows.rs` - Added refresh_oauth_token() and maybe_refresh_stored_token()
- `crates/codiv-common/src/auth/mod.rs` - Exported both new functions from auth module
- `crates/codivd/src/agent/config.rs` - Added spawn_token_refresh_task() background task function
- `crates/codivd/src/daemon.rs` - Added _token_refresh_task field and spawn call in Daemon::new()

## Decisions Made

- Used `eprintln!` instead of `tracing::warn!` in codiv-common because tracing is not a dependency of codiv-common (would require adding a new dep). This follows the plan's instruction: "If tracing is not a dep of codiv-common, use eprintln! instead."
- Background task approach chosen over per-call refresh (the plan's "REVISED APPROACH") — cleaner separation, avoids sync/async complexity in model builder functions, tokens are already written to config.toml so hot-reload picks them up automatically.

## Deviations from Plan

None - plan executed exactly as written (including the "REVISED APPROACH" for Task 2).

## Issues Encountered

None - both tasks compiled and passed verification on the first attempt.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- OAuth token refresh infrastructure is complete and operational in codivd
- Tokens for claude_code and codex will be proactively refreshed before expiry
- Ready for Phase 03 Plan 03 (the login CLI wizard that exercises the full auth flow end-to-end)

---
*Phase: 03-cli-wizard-daemon-refresh*
*Completed: 2026-04-01*

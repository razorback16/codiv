---
phase: 01-auth-foundation
plan: 01
subsystem: auth
tags: [rust, serde, chrono, url, oauth, api-key, newtypes]

# Dependency graph
requires: []
provides:
  - "OAuthTokens struct with access_token, optional refresh_token, expires_at (DateTime<Utc>)"
  - "OAuthConfig struct for OAuth 2.0 flow configuration with PKCE support"
  - "AuthMethod enum with ApiKey and OAuthCode(OAuthConfig) variants"
  - "ApiKey newtype with safe Display truncation for logging"
  - "AccessToken and RefreshToken newtypes with serde transparent serialization"
  - "codiv-common/src/auth module wired into lib.rs"
affects: [01-auth-foundation/01-02, provider-registry, login-flow, token-refresh]

# Tech tracking
tech-stack:
  added:
    - "chrono 0.4 (serde feature) — DateTime<Utc> for token expiry"
    - "url 2 (serde feature) — Url type for OAuth endpoint URLs"
  patterns:
    - "Newtype pattern for security-sensitive strings (ApiKey, AccessToken, RefreshToken)"
    - "Serde transparent serialization on newtypes"
    - "Tests co-located in types.rs #[cfg(test)] mod tests"
    - "Auth module in codiv-common/src/auth/ with mod.rs re-exports"

key-files:
  created:
    - "crates/codiv-common/src/auth/types.rs"
    - "crates/codiv-common/src/auth/mod.rs"
  modified:
    - "crates/codiv-common/Cargo.toml"
    - "crates/codiv-common/src/lib.rs"

key-decisions:
  - "ApiKey Display truncates at 20 chars: shows first 13 + '...' + last 4 for safe debug logging"
  - "OAuthConfig includes optional token_refresh_url, custom_headers, extra_auth_params for provider extensibility"
  - "AuthMethod v1 subset: ApiKey and OAuthCode only (OAuthDevice, CodexDevice, GoogleAdc deferred)"
  - "No ClientId newtype — plain String for client_id simplifies v1 provider config"

patterns-established:
  - "Security newtypes: wrap sensitive strings in newtypes, implement Display with truncation"
  - "Auth module layout: crates/codiv-common/src/auth/{mod.rs, types.rs} with pub use re-exports in mod.rs"

requirements-completed: [AUTH-03]

# Metrics
duration: 8min
completed: 2026-03-31
---

# Phase 01 Plan 01: Auth Types Foundation Summary

**Core OAuth and API key auth types (OAuthTokens, OAuthConfig, AuthMethod, ApiKey, AccessToken, RefreshToken) added to codiv-common with chrono/url dependencies and 7 passing unit tests**

## Performance

- **Duration:** ~8 min
- **Started:** 2026-03-31T16:38:00Z
- **Completed:** 2026-03-31T16:40:08Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments

- Added `chrono` (serde feature) and `url` (serde feature) dependencies to codiv-common
- Created `crates/codiv-common/src/auth/types.rs` with all required types and 7 unit tests
- Wired `pub mod auth` into `codiv-common/src/lib.rs`
- `cargo check --workspace` passes; `cargo test -p codiv-common` passes (29/29 tests)

## Task Commits

Each task was committed atomically:

1. **Task 1: Add auth dependencies to codiv-common** - `8ca3355` (chore)
2. **Task 2: Create auth types module** - `0578264` (feat)

## Files Created/Modified

- `crates/codiv-common/Cargo.toml` - Added chrono 0.4 and url 2 with serde features
- `crates/codiv-common/src/auth/types.rs` - All core auth types with unit tests
- `crates/codiv-common/src/auth/mod.rs` - Auth module re-exports
- `crates/codiv-common/src/lib.rs` - Added `pub mod auth`

## Decisions Made

- ApiKey Display truncates keys > 20 chars: first 13 chars + "..." + last 4 chars, preventing accidental key exposure in logs
- OAuthConfig includes `token_refresh_url`, `custom_headers`, and `extra_auth_params` (optional) for provider-specific OAuth needs without code changes
- AuthMethod limited to `ApiKey` and `OAuthCode` variants for v1 (covers Anthropic, OpenAI, Claude Code, Codex)
- `client_id` kept as plain `String` (no ClientId newtype) — simpler for v1 without sacrificing correctness

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- `codiv_common::auth` module is ready for plan 02 (provider registry) which will use `AuthMethod` and `OAuthConfig` to declare how each provider authenticates
- Phase 2 OAuth flow implementations can import `OAuthTokens`, `AccessToken`, `RefreshToken` from `codiv_common::auth`
- All types implement `Serialize`/`Deserialize` — ready for TOML config storage integration

## Self-Check: PASSED

- FOUND: crates/codiv-common/src/auth/types.rs
- FOUND: crates/codiv-common/src/auth/mod.rs
- FOUND: commit 8ca3355 (chore: add chrono and url dependencies)
- FOUND: commit 0578264 (feat: create auth types module)

---
*Phase: 01-auth-foundation*
*Completed: 2026-03-31*

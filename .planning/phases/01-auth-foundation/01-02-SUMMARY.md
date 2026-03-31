---
phase: 01-auth-foundation
plan: 02
subsystem: auth
tags: [rust, provider-registry, oauth, api-key, codiv-common]

# Dependency graph
requires:
  - phase: 01-auth-foundation/01-01
    provides: AuthMethod, OAuthConfig, ApiKey, AccessToken, RefreshToken types in codiv-common::auth::types
provides:
  - ProviderEntry struct with id, display_name, auth_methods fields
  - provider_registry() function returning 4 provider entries (anthropic, openai, claude_code, codex)
  - provider_by_id() helper for lookup by stable string ID
  - Re-exports in codiv-common::auth module
affects: [02-auth-flows, 03-login-wizard, codivd-token-refresh]

# Tech tracking
tech-stack:
  added: []
  patterns: [hardcoded provider catalog as pure Rust Vec (no JSON loading), &'static str for provider IDs]

key-files:
  created:
    - crates/codiv-common/src/auth/provider_registry.rs
  modified:
    - crates/codiv-common/src/auth/mod.rs

key-decisions:
  - "Provider registry is hardcoded Rust Vec — no JSON file loading for v1 simplicity"
  - "&'static str used for id and display_name fields — zero allocation, safe for enum-like provider keys"

patterns-established:
  - "Provider lookup pattern: provider_by_id() iterates registry, returns Option<ProviderEntry>"
  - "TDD RED/GREEN commit sequence: test commit first with stub, then feat commit with implementation"

requirements-completed: [PROV-05]

# Metrics
duration: 3min
completed: 2026-03-31
---

# Phase 01 Plan 02: Provider Registry Summary

**Hardcoded provider catalog for Anthropic (ApiKey), OpenAI (ApiKey), Claude Code (PKCE OAuth), and Codex (OAuth) with lookup by stable string ID**

## Performance

- **Duration:** 3 min
- **Started:** 2026-03-31T16:42:38Z
- **Completed:** 2026-03-31T16:45:27Z
- **Tasks:** 1 (TDD: RED + GREEN commits)
- **Files modified:** 2

## Accomplishments

- Created `provider_registry.rs` with `ProviderEntry` struct and `provider_registry()` returning 4 v1 providers
- Wired correct OAuth configs: Claude Code uses PKCE + client_id `9d1c250a-e61b-44d9-88ed-5944d1962f5e`; Codex uses no PKCE + client_id `app_EMoamEEZ73f0CkXaXp7hrann`
- Added `provider_by_id()` helper; updated `mod.rs` to re-export all public items
- 9 new tests pass alongside 30 existing tests (39 total in codiv-common)

## Task Commits

Each task was committed atomically (TDD flow):

1. **Task 1 RED: Provider registry stub + tests** - `e333684` (test)
2. **Task 1 GREEN: Full provider registry implementation** - `7e90408` (feat)

**Plan metadata:** (docs commit follows)

_Note: TDD tasks have multiple commits (test RED → feat GREEN)_

## Files Created/Modified

- `crates/codiv-common/src/auth/provider_registry.rs` - ProviderEntry struct, provider_registry() fn, provider_by_id() fn, 9 tests
- `crates/codiv-common/src/auth/mod.rs` - Added `pub mod provider_registry` and re-exports for ProviderEntry, provider_by_id, provider_registry

## Decisions Made

- Provider registry is a hardcoded `Vec<ProviderEntry>` — no JSON file loading needed for v1, simpler and zero-allocation
- `&'static str` for `id` and `display_name` fields — avoids heap allocation for enum-like constants, safe to clone
- OAuth configs ported directly from forgecode's `provider.json` values without modification

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- `codiv-common::auth` now exports both the auth types (plan 01-01) and provider registry (plan 01-02)
- Phase 2 auth flows can call `provider_by_id("claude_code")` to get OAuth config for the login flow
- Phase 3 CLI wizard can call `provider_registry()` to iterate all providers for the selection prompt
- No blockers.

---
*Phase: 01-auth-foundation*
*Completed: 2026-03-31*

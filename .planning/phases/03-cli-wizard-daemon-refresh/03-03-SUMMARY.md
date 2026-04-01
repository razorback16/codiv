---
phase: 03-cli-wizard-daemon-refresh
plan: "03"
subsystem: auth
tags: [clap, cli, env-migration, config-toml, api-keys]

# Dependency graph
requires:
  - phase: 02-auth-flows-storage
    provides: write_api_key_to_config function and ApiKey Display newtype
provides:
  - codiv migrate-env CLI subcommand
  - Env var to config.toml migration for ANTHROPIC_API_KEY, OPENAI_API_KEY, GOOGLE_API_KEY
affects: [03-cli-wizard-daemon-refresh]

# Tech tracking
tech-stack:
  added: []
  patterns: [clap kebab-case subcommand dispatch, ApiKey Display for safe output]

key-files:
  created:
    - crates/codiv/src/cli/migrate.rs
  modified:
    - crates/codiv/src/cli/mod.rs
    - crates/codiv/src/cli/tools.rs

key-decisions:
  - "Fixed non-exhaustive match in cli/tools.rs by adding MigrateEnv arm (Rule 3 - blocking issue)"

patterns-established:
  - "New CLI subcommands must be added to both mod.rs dispatch AND tools.rs handle match to avoid non-exhaustive pattern errors"

requirements-completed: [STOR-03]

# Metrics
duration: 2min
completed: 2026-03-31
---

# Phase 03 Plan 03: migrate-env Subcommand Summary

**`codiv migrate-env` detects ANTHROPIC_API_KEY, OPENAI_API_KEY, GOOGLE_API_KEY env vars and writes them to `~/.codiv/config.toml` using safe truncated ApiKey Display output**

## Performance

- **Duration:** ~2 min
- **Started:** 2026-03-31T09:16:14Z
- **Completed:** 2026-03-31T09:17:34Z
- **Tasks:** 1
- **Files modified:** 3 (1 created, 2 modified)

## Accomplishments
- Created `crates/codiv/src/cli/migrate.rs` with `MigrateEnvArgs` and `handle_migrate_env`
- Wired `MigrateEnv` into the `Commands` enum and `dispatch` in `cli/mod.rs`
- All three provider env vars (Anthropic, OpenAI, Google) mapped to their config provider IDs
- API keys never printed raw — `ApiKey::new(value)` Display truncation enforced

## Task Commits

Each task was committed atomically:

1. **Task 1: Create migrate.rs and wire MigrateEnv into CLI** - `8ae0329` (feat)

**Plan metadata:** (docs commit — see below)

## Files Created/Modified
- `crates/codiv/src/cli/migrate.rs` - MigrateEnvArgs, handle_migrate_env, run() logic for 3 env vars
- `crates/codiv/src/cli/mod.rs` - Added `pub mod migrate;`, `MigrateEnv` variant, dispatch arm
- `crates/codiv/src/cli/tools.rs` - Added `Commands::MigrateEnv` arm to fix non-exhaustive match

## Decisions Made
None beyond plan specification — plan was detailed and complete.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed non-exhaustive match in cli/tools.rs**
- **Found during:** Task 1 (build verification)
- **Issue:** `crates/codiv/src/cli/tools.rs` has its own `handle(cmd: Commands)` match that also covers all `Commands` variants. Adding `MigrateEnv` to the enum without updating `tools.rs` caused a compile error `E0004: non-exhaustive patterns`.
- **Fix:** Added `Commands::MigrateEnv(args) => super::migrate::handle_migrate_env(args)` arm to the `tools::handle` match.
- **Files modified:** `crates/codiv/src/cli/tools.rs`
- **Verification:** `cargo build -p codiv` exits 0 after fix.
- **Committed in:** `8ae0329` (included in task 1 commit)

---

**Total deviations:** 1 auto-fixed (Rule 3 - blocking)
**Impact on plan:** Required fix for compilation; no scope creep. Pattern is now documented: all new subcommands need an arm in both `mod.rs` dispatch and `tools.rs` handle.

## Issues Encountered
None beyond the auto-fixed blocking issue above.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- `codiv migrate-env` is fully functional — users can run it to migrate env var credentials to config.toml
- Phase 03 plans 01 and 02 complete the `codiv login` wizard and daemon refresh; this plan completes the migration UX
- No blockers

---
*Phase: 03-cli-wizard-daemon-refresh*
*Completed: 2026-03-31*

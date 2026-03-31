# Coding Conventions

**Analysis Date:** 2026-03-31

## Naming Patterns

**Files:**
- `snake_case.rs` everywhere: `bash_coprocess.rs`, `permission_evaluator.rs`, `ast_classifier.rs`
- Modules mirror directory names: `crates/codiv/src/ui/terminal/render.rs` → `crate::ui::terminal::render`
- No barrel-file aliasing — each module is individually imported

**Structs / Enums:**
- `PascalCase`: `BashCoprocess`, `InputLine`, `ClientMessage`, `DaemonMessage`, `ApiErrorKind`
- Enum variants are `PascalCase`: `RiskLevel::High`, `PermissionDecision::LlmEvaluate`

**Functions / Methods:**
- `snake_case`: `classify_error`, `generate_sentinel`, `truncate_output`, `frame_message`
- Boolean predicates prefixed with `is_`: `is_retryable()`, `is_readonly_bash()`, `is_char_boundary()`
- Constructor convention: `fn new(...)` for primary constructors, `fn open(...)` for resource openers (`SessionStore::open`)
- Builder-style helpers: `fn spawn(...)` for processes, `fn build_tools(...)` for factory functions

**Constants:**
- `SCREAMING_SNAKE_CASE`: `MAX_TOOL_OUTPUT_BYTES`, `MAX_HISTORY_EVENTS`, `PASTE_COLLAPSE_THRESHOLD`, `INITIAL_BACKOFF_MS`
- Module-level constants declared immediately before the code that uses them

**Type Aliases:**
- Sparse use: only `type DynError = Box<dyn std::error::Error + Send + Sync>` found in `crates/codivd/src/agent/config.rs`

## Code Style

**Formatting:**
- Standard `rustfmt` (no custom config detected)
- Edition 2021 across all crates

**Linting:**
- `#[allow(dead_code)]` used to suppress warnings for fields intentionally unused (e.g., `blocks.rs`, `state.rs`) — prefer this over removing fields that serve documentation purposes
- `#[allow(clippy::too_many_arguments)]` used in `event_loop.rs` and `input_keys.rs` for large event-handler functions
- `#[allow(clippy::module_inception)]` in `crates/codivd/src/agent/mod.rs` where module and inner file share a name

## Import Organization

**Order (observed):**
1. Crate-local (`use crate::...`) or sibling (`use super::...`)
2. Workspace crates (`use codiv_common::...`, `use codiv_tools::...`)
3. Third-party (`use tokio::...`, `use serde::...`, `use tracing::...`)
4. Standard library (`use std::...`)

No enforced auto-grouping tool (no `rustfmt` import grouping config detected). Imports are grouped by hand, typically with a blank line between `crate::` and external crates.

**Path Aliases:**
- None — fully qualified paths only

## Section Separators

Large files use dashed separator comments to divide logical sections:

```rust
// ---------------------------------------------------------------------------
// Section Name
// ---------------------------------------------------------------------------
```

Shorter sections in test modules use `// -- Section name --`.

## Error Handling

**Return type patterns:**
- `Result<T, String>` — used in tool execute functions (`read::execute`, `bash::execute`) where the error is a human-readable message
- `Result<T, rusqlite::Error>` — typed errors for database operations in `store.rs`
- `Result<T, DynError>` (where `DynError = Box<dyn Error + Send + Sync>`) — async agent/IPC pipeline in `config.rs`
- `std::io::Result<T>` — process/socket/PTY operations

**Error propagation:**
- `?` operator throughout async code
- `map_err(|e| format!("context: {e}"))` for converting typed errors to `String` errors at tool boundaries
- No `anyhow` or `thiserror` — either raw `Box<dyn Error>` or explicit `Result<T, SomeError>`

**Error categorization:**
- `ApiErrorKind` enum in `crates/codivd/src/agent/error.rs` provides structured classification of API errors (Server, RateLimited, ContextOverflow, AuthError, Other) with `is_retryable()` helper

**Non-fatal errors:**
- `tracing::error!(...)` / `tracing::warn!(...)` for soft failures in the daemon
- `log::debug!()` / `log::info!()` in the TUI client (uses `log` crate, not `tracing`)
- `.ok()` for explicitly discarding errors where failure is acceptable (e.g., cleanup operations)

## Logging

**Two logging frameworks in use:**
- **Daemon (`codivd`):** `tracing` crate — `tracing::info!`, `tracing::debug!`, `tracing::warn!`, `tracing::error!`
  - Structured fields: `tracing::debug!(command = %input.command, timeout_ms = ..., "bash")`
  - Outputs to `~/.codiv/codivd.log` via `tracing_subscriber::fmt`
- **TUI client (`codiv`):** `log` crate — `log::debug!`, `log::info!`
  - Outputs to `/tmp/codiv-debug.log` (only when `--debug` flag is set)

**What gets logged:**
- `debug`: Per-command execution details, PTY/completion events, stream details
- `info`: Major lifecycle events (spawn, connect, session create, config reload)
- `warn`: Recoverable failures (timeout, channel full, LLM evaluator fallback)
- `error`: Failed DB writes, stream failures, spawn failures

## Comments

**Module-level docs (`//!`):**
Used consistently on files with non-obvious scope:
```rust
//! `PipeIO`: `ShellIO` implementation over stdin/stdout/stderr pipes.
//!
//! Spawns no process itself — the caller provides `ChildStdin`, ...
```

**Item docs (`///`):**
Used on all public structs, enums, traits, and non-trivial public functions. Inline field docs on struct fields explain invariants. Single-line docs preferred; multi-line only for complex explanations.

**Inline comments (`//`):**
- Used within function bodies to explain non-obvious logic: protocol details, edge cases, why something is done a certain way
- Section separators (`// ---`) used inside large `impl` blocks to group related methods

## Function Design

**Size:** Most functions fit on screen. Large event loops (`event_loop.rs`, `daemon.rs`) are the exception; they use `#[allow(clippy::too_many_arguments)]` where needed.

**Parameters:** Prefer taking `&str` over `&String`, `&[T]` over `&Vec<T>`. Shared mutable state passed as `Arc<RwLock<T>>`.

**Return Values:** `Option<T>` for "maybe absent", `Result<T, E>` for "might fail". No panic-on-None in production paths — panics reserved for truly invariant conditions (e.g., `expect("stdout piped")` after explicit `Stdio::piped()`).

## Module Design

**Exports:**
- `pub` for cross-crate API surface (in `codiv-common`)
- `pub(crate)` for intra-crate sharing between modules (common in `codiv/src/ui/terminal/`)
- `pub(super)` for constants and helpers shared between parent and child module only
- Private by default for implementation details

**Barrel Files:**
- No barrel `lib.rs` re-exports in `codiv` or `codivd` (binary crates)
- `codiv-common` and `codiv-tools` use `lib.rs` with explicit `pub mod` declarations
- `mod.rs` files in subdirectories declare and optionally re-export their children

---

*Convention analysis: 2026-03-31*

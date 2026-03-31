# Testing Patterns

**Analysis Date:** 2026-03-31

## Test Framework

**Runner:**
- Rust's built-in `cargo test` — no additional test runner
- `tokio::test` (`#[tokio::test]`) for async tests in `codivd` and E2E tests

**Assertion Library:**
- Standard Rust `assert!`, `assert_eq!`, `assert_ne!`
- `matches!` macro for pattern matching: `assert!(matches!(msg, ClientMessage::Heartbeat { .. }))`
- No third-party assertion libraries (no `pretty_assertions`, `proptest`, etc.)

**Run Commands:**
```bash
make test              # cargo test --workspace
cargo test             # Run all tests
cargo test -p codivd   # Run tests for a single crate
cargo test -- --nocapture  # Show stdout during tests
```

## Test File Organization

**Location:**
Two patterns used:

1. **Inline test modules** (most common) — co-located at the bottom of the source file:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       // tests here
   }
   ```
   Used in 25 files across all crates.

2. **Integration test files** — in `tests/` directory at crate root:
   - `crates/codiv/tests/e2e_paste.rs` — E2E tests for the paste feature
   - `crates/codivd/tests/ipc_roundtrip.rs` — IPC protocol roundtrip test

**Naming:**
- Unit test functions: descriptive `snake_case` without `test_` prefix in most cases (e.g., `readonly_commands_are_low`, `backspace_at_start_is_noop`, `short_output_unchanged`)
- Some legacy/tool-level tests use `test_` prefix: `test_basic_echo`, `test_server_errors`
- Test module is always named `tests`

## Test Structure

**Suite Organization:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Optional: grouped with comments like "// -- Auto mode tests --"
    #[test]
    fn descriptive_behavior_name() {
        // Arrange
        let mut input = InputLine::new();
        // Act
        input.insert('a');
        // Assert
        assert_eq!(input.content(), "a");
    }
}
```

**Grouping within test modules:**
Section comments group related tests inside the module:
```rust
// -----------------------------------------------------------------------
// Read-only / Low risk
// -----------------------------------------------------------------------
#[test]
fn readonly_commands_are_low() { ... }

// -----------------------------------------------------------------------
// Medium risk
// -----------------------------------------------------------------------
```

**Async tests:**
```rust
#[tokio::test]
async fn short_paste_appears_inline() {
    let term = spawn_codiv().await;
    // ...
    term.kill().await.ok();
}
```

## Mocking

**Framework:** None — no mocking library used (no `mockall`, `mockito`, etc.)

**Approach:**
- Behavior is tested through real implementations, not mocks
- Test helper functions construct real instances with controlled inputs:
  ```rust
  fn run(cmd: &str, timeout_ms: u64) -> Result<String, String> {
      let value = json!({ "command": cmd, "timeout_ms": timeout_ms });
      let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
      execute(value, &cwd, &[])
  }
  ```
- IPC tests spin up real Unix sockets with `tokio::net::UnixListener`
- E2E tests spawn the actual compiled `codiv` binary via `termwright`

**What to Mock:**
- Nothing is mocked — rely on real process execution and real socket communication

**What NOT to Mock:**
- Do not mock file I/O, shell execution, or IPC — use real implementations with controlled inputs

## Fixtures and Factories

**Test Data:**
- Constructed inline using struct literals or `serde_json::json!` macro:
  ```rust
  let args = serde_json::json!({"command": "cargo build"});
  assert_eq!(classify_risk("bash", &args), RiskLevel::Low);
  ```
- InputLine tests build state by calling methods sequentially (no fixtures):
  ```rust
  let mut input = InputLine::new();
  input.insert('l');
  input.insert('s');
  input.submit();
  ```

**Location:**
- No separate fixture files — all test data is constructed inline

**Helper functions in test modules:**
Some test modules define private helpers:
```rust
// crates/codiv/tests/e2e_paste.rs
fn bracketed_paste(text: &str) -> Vec<u8> { ... }
async fn spawn_codiv() -> Terminal { ... }
async fn save_screenshot(term: &Terminal, name: &str) { ... }

// crates/codiv/src/ui/terminal/utils.rs tests
fn screen_line(parser: &vt100::Parser, row: u16, cols: u16) -> String { ... }
```

## Coverage

**Requirements:** No enforced coverage threshold — no `.codecov.yml` or coverage configuration detected.

**View Coverage:**
```bash
cargo tarpaulin --workspace  # If tarpaulin is installed
```

## Test Types

**Unit Tests:**
- The majority of tests; co-located inline with source using `#[cfg(test)]`
- Scope: single function or method in isolation
- Examples:
  - `crates/codiv-common/src/truncate.rs` — tests `truncate_output` and `truncate_tool_output`
  - `crates/codivd/src/agent/error.rs` — tests `classify_error` and `ApiErrorKind::is_retryable`
  - `crates/codivd/src/agent/ast_classifier.rs` — extensive tests for bash command risk classification
  - `crates/codiv/src/ui/input.rs` — comprehensive tests for `InputLine` cursor, history, paste behavior
  - `crates/codiv/src/ui/diff.rs` — tests for `generate_unified_diff`

**Integration Tests:**
- `crates/codivd/tests/ipc_roundtrip.rs` — spins up a real Unix socket server, sends a `ClientMessage::Heartbeat`, and verifies `DaemonMessage::Heartbeat` response
- Tests rely on `bincode` serialization correctness end-to-end

**E2E Tests:**
- `crates/codiv/tests/e2e_paste.rs` — uses `termwright` (dev-dependency) to spawn the compiled `codiv` binary in a pseudo-terminal
- Sends bracketed paste sequences and asserts on screen contents and screenshots
- Screenshots saved to `crates/codiv/tests/screenshots/`
- Requires the binary to be compiled (`CARGO_BIN_EXE_codiv`)

## Common Patterns

**Async Testing:**
```rust
#[tokio::test]
async fn test_heartbeat_roundtrip() {
    let socket_path = format!("/tmp/codivd-test-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);
    // setup...
    let _ = std::fs::remove_file(&socket_path); // cleanup
}
```
- Process ID used in socket paths to avoid test collisions
- Manual cleanup of sockets in both setup and teardown

**Error Testing:**
```rust
#[test]
fn test_context_overflow_takes_priority_over_500() {
    assert_eq!(
        classify_error("500 Internal Server Error: context_length_exceeded"),
        ApiErrorKind::ContextOverflow,
    );
}
```

**Boundary Testing:**
```rust
#[test]
fn boundary_three_vs_four_lines() {
    // Tests both sides of the threshold in one test function
    let term3 = spawn_codiv().await;
    // assert 3 lines is inline...
    let term4 = spawn_codiv().await;
    // assert 4 lines is collapsed...
}
```

**Screen/Output Assertion:**
```rust
let screen = term.screen().await;
let text = screen.text();
assert!(
    text.contains("[Pasted text"),
    "long paste should show collapsed indicator, got:\n{text}"
);
```
- Error messages always include the actual value: `"got:\n{text}"`

**PTY/Parser Tests (render module):**
```rust
fn screen_line(parser: &vt100::Parser, row: u16, cols: u16) -> String {
    let mut line = String::new();
    for col in 0..cols {
        if let Some(cell) = parser.screen().cell(row, col) {
            line.push(cell.contents().chars().next().unwrap_or(' '));
        }
    }
    line.trim_end().to_string()
}
```
Tests construct a `vt100::Parser` directly and assert on rendered cell contents.

---

*Testing analysis: 2026-03-31*

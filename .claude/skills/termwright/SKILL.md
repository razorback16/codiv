---
name: termwright
description: >
  Guide for using termwright ("Playwright for TUIs") to end-to-end test terminal applications.
  Use this skill whenever the user wants to write, run, or debug E2E tests for a terminal/TUI app,
  automate terminal interactions, capture terminal snapshots, or mentions termwright, tui testing,
  terminal testing, or terminal automation. Also trigger when the user asks about testing codiv's
  TUI client end-to-end.
---

# Termwright — Terminal UI Testing

Termwright is an end-to-end testing and automation framework for terminal applications — think "Playwright, but for TUIs." Written in Rust, it lets you spawn a terminal session in a PTY, interact with it (type, press keys, send raw bytes, click with mouse), wait for expected output, query screen state, and take screenshots.

## When to reach for termwright

- Writing E2E tests for any CLI or TUI program (including codiv itself)
- Automating interactive terminal workflows (setup wizards, REPLs, SSH sessions)
- Capturing terminal screenshots / text snapshots for regression testing
- Debugging flaky terminal behavior by scripting reproducible steps
- Enabling AI agents to observe and interact with terminal UIs via JSON output

## Installation

```toml
# As a Rust dependency in Cargo.toml
[dependencies]
termwright = "0.2"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```bash
# CLI installation
cargo install termwright
```

## CLI Reference

### `termwright run`

Run a command and capture its output.

```bash
termwright run [OPTIONS] -- <COMMAND> [ARGS]...
```

Options: `--cols`, `--rows`, `--wait-for <TEXT>`, `--delay <MS>`, `--format <text|json|json-compact>`, `--timeout <SECS>`

### `termwright screenshot`

Take a PNG screenshot of a terminal application.

```bash
termwright screenshot [OPTIONS] -- <COMMAND> [ARGS]...
```

Options: `--cols`, `--rows`, `--wait-for <TEXT>`, `--delay <MS>`, `-o <PATH>`, `--font <NAME>`, `--font-size <SIZE>`, `--timeout <SECS>`

### `termwright daemon`

Run a single TUI session and expose it over a Unix socket for incremental automation.

```bash
termwright daemon [OPTIONS] -- <COMMAND> [ARGS]...
```

Options: `--cols`, `--rows`, `--socket <PATH>`, `--background`

### `termwright exec`

Execute a single daemon request and print the response.

```bash
termwright exec --socket <PATH> --method <NAME> [--params <JSON>]
```

### `termwright run-steps`

Run a YAML or JSON steps file for scripted E2E testing.

```bash
termwright run-steps [OPTIONS] <FILE>
```

### CLI example

```bash
# Capture terminal output as text
termwright run -- ls -la

# Take a screenshot of a TUI application
termwright screenshot --wait-for "VIM" -o vim.png -- vim test.txt

# Get JSON output for AI processing
termwright run --format json -- htop
```

## Library API (Rust)

For programmatic tests (e.g. in `#[tokio::test]` integration tests):

```rust
use std::time::Duration;
use termwright::prelude::*;

#[tokio::test]
async fn test_my_tui() -> Result<()> {
    // Spawn a terminal application
    let term = Terminal::builder()
        .size(120, 36)
        .working_dir("/path/to/project")
        .env("MY_VAR", "value")
        .spawn("./target/debug/my-app", &["--flag"])
        .await?;

    // Wait for the application to be ready
    term.expect("Ready")
        .timeout(Duration::from_secs(10))
        .await?;

    // Type text and press keys
    term.type_str("hello world").await?;
    term.send_key(Key::Enter).await?;
    term.enter().await?;  // Shorthand for Enter

    // Send raw bytes (useful for escape sequences, bracketed paste, etc.)
    term.send_raw(b"\x1b[200~pasted text\x1b[201~").await?;

    // Query screen state
    let screen = term.screen().await;
    assert!(screen.contains("hello world"));

    let text = screen.text();
    let line = screen.line(0);

    // Find text on screen
    if let Some(pos) = screen.find_text("error") {
        println!("Found at row={}, col={}", pos.row, pos.col);
    }

    // Cursor position
    let cursor = screen.cursor();

    // Wait conditions
    term.expect("expected text")
        .timeout(Duration::from_secs(5))
        .await?;

    term.wait_idle(Duration::from_millis(500))
        .timeout(Duration::from_secs(5))
        .await?;

    // Take a screenshot
    term.screenshot().await.save("output.png")?;

    // Clean up
    term.kill().await?;

    Ok(())
}
```

### Key API methods

| Method | Purpose |
|---|---|
| `Terminal::builder()` | Create a builder. Chain `.size()`, `.env()`, `.working_dir()`, `.timeout()`, then `.spawn(cmd, args)` |
| `term.type_str(text)` | Type text into the terminal |
| `term.send_key(key)` | Send a key press (`Key::Enter`, `Key::Ctrl('c')`, `Key::Tab`, etc.) |
| `term.enter()` | Shorthand for `send_key(Key::Enter)` |
| `term.escape()` | Shorthand for `send_key(Key::Escape)` |
| `term.send_raw(bytes)` | Send raw bytes (escape sequences, bracketed paste, etc.) |
| `term.screen()` | Get current `Screen` snapshot |
| `term.expect(text)` | Wait for text to appear (returns builder, chain `.timeout()` then `.await`) |
| `term.expect_gone(text)` | Wait for text to disappear |
| `term.expect_pattern(regex)` | Wait for regex pattern match |
| `term.wait_idle(duration)` | Wait for screen to stabilize |
| `term.wait_exit()` | Wait for the process to exit |
| `term.kill()` | Kill the terminal process |
| `term.resize(cols, rows)` | Resize the PTY |
| `term.screenshot()` | Take a PNG screenshot |

### Screen API

| Method | Purpose |
|---|---|
| `screen.text()` | Get all screen text |
| `screen.line(n)` | Get text of row `n` |
| `screen.contains(text)` | Check if text appears anywhere |
| `screen.find_text(text)` | Find position of text (returns `Option<TextMatch>`) |
| `screen.cell(row, col)` | Get individual cell (char, fg, bg, attributes) |
| `screen.cursor()` | Get cursor position |
| `screen.region(rows, cols)` | Extract a rectangular region |
| `screen.detect_boxes()` | Detect UI boundaries using box-drawing characters |
| `screen.to_json()` | Pretty JSON output |
| `screen.to_json_compact()` | Compact JSON output |

### Key types

```rust
Key::Char('a')       // Regular characters
Key::Enter           // Enter/Return
Key::Tab             // Tab
Key::Escape          // Escape
Key::Backspace       // Backspace
Key::Up / Down / Left / Right  // Arrow keys
Key::Home / End / PageUp / PageDown
Key::Insert / Delete
Key::F(1)..Key::F(12)  // Function keys
Key::Ctrl('c')       // Ctrl combinations
Key::Alt('x')        // Alt combinations
```

## Daemon mode (from Rust)

For long-lived sessions controlled over a Unix socket:

```rust
use termwright::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let client = DaemonClient::connect_unix("/tmp/termwright-12345.sock").await?;

    let info = client.handshake().await?;
    client.wait_for_text("Ready", Some(Duration::from_secs(5))).await?;

    let screen_text = client.screen_text().await?;
    client.press("Enter").await?;
    client.r#type("hello").await?;
    client.hotkey_ctrl('c').await?;

    client.close().await?;
    Ok(())
}
```

## Best practices

These patterns make terminal tests more reliable:

1. **Wait before asserting.** Terminal output is asynchronous. Never assume the screen has updated after typing or pressing a key — always use `expect()` or `wait_idle()` before reading screen state.

2. **Use `wait_idle` for unpredictable timing** (compilation, network requests, file I/O). It waits until the terminal stops producing output, which is more robust than a fixed timeout.

3. **Use `send_raw` for escape sequences.** When testing features like bracketed paste, mouse input, or other raw terminal protocols, use `send_raw()` to send exact byte sequences.

4. **Handle interactive prompts.** Many CLI tools ask for confirmation, login, or first-run setup. Your test should anticipate these and respond (or configure the tool to skip them via env vars / flags).

5. **Debug with screen snapshots.** When a test fails, `screen.text()` at the point of failure is the most useful artifact. Print it in your assertion messages.

6. **Set appropriate timeouts.** Use `.timeout()` on expect builders. Default is 30 seconds which may be too long for fast tests or too short for slow startups.

## Testing codiv with termwright

Since codiv is a ratatui-based TUI, termwright is a natural fit for E2E testing. Example pattern:

```rust
use std::time::Duration;
use termwright::prelude::*;

#[tokio::test]
async fn test_codiv_prompt() -> Result<()> {
    let term = Terminal::builder()
        .size(120, 36)
        .env("CODIV_TEST", "1")
        .spawn("./target/debug/codiv", &["--debug"])
        .await?;

    // Wait for the TUI to render (AI mode prompt)
    term.expect(">")
        .timeout(Duration::from_secs(10))
        .await?;

    // Type a command
    term.type_str("echo hello world").await?;
    term.enter().await?;

    // Verify output appeared
    term.expect("hello world")
        .timeout(Duration::from_secs(5))
        .await?;

    let screen = term.screen().await;
    assert!(screen.contains("hello world"));

    term.kill().await?;
    Ok(())
}
```

### Simulating bracketed paste

To test paste behavior, use `send_raw` with bracketed paste escape sequences:

```rust
// Bracketed paste: \x1b[200~ ... \x1b[201~
// Use \r for line endings (as real terminals do)
let paste = b"\x1b[200~line1\rline2\rline3\x1b[201~";
term.send_raw(paste).await?;
```

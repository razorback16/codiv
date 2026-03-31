//! End-to-end tests for the paste feature in codiv.
//!
//! These tests use termwright to spawn the codiv TUI binary and simulate
//! bracketed paste events, verifying inline vs collapsed behavior.
//! Screenshots are saved to `crates/codiv/tests/screenshots/`.

use std::path::PathBuf;
use std::time::Duration;
use termwright::prelude::*;

/// Bracketed paste start sequence.
const PASTE_START: &[u8] = b"\x1b[200~";
/// Bracketed paste end sequence.
const PASTE_END: &[u8] = b"\x1b[201~";

/// Helper: build a bracketed paste payload with `\r` line endings (as real
/// terminals send).
fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(PASTE_START);
    buf.extend_from_slice(text.replace('\n', "\r").as_bytes());
    buf.extend_from_slice(PASTE_END);
    buf
}

/// Return the path to the screenshots directory, creating it if needed.
fn screenshots_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/screenshots");
    std::fs::create_dir_all(&dir).expect("failed to create screenshots dir");
    dir
}

/// Save a screenshot with the given name.
async fn save_screenshot(term: &Terminal, name: &str) {
    let path = screenshots_dir().join(format!("{name}.png"));
    term.screenshot().await.save(&path).expect("failed to save screenshot");
}

/// Spawn the codiv binary and wait for the AI-mode prompt (`>`).
async fn spawn_codiv() -> Terminal {
    let bin = env!("CARGO_BIN_EXE_codiv");
    let term = Terminal::builder()
        .size(120, 36)
        .spawn(bin, &["--debug"])
        .await
        .expect("failed to spawn codiv");

    term.expect(">")
        .timeout(Duration::from_secs(10))
        .await
        .expect("codiv prompt did not appear");

    term
}

/// Short paste (2 lines) should be inserted inline into the prompt.
#[tokio::test]
async fn short_paste_appears_inline() {
    let term = spawn_codiv().await;

    let paste = bracketed_paste("hello\nworld");
    term.send_raw(&paste).await.expect("failed to send paste");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term, "01_short_paste_inline").await;

    let screen = term.screen().await;
    let text = screen.text();

    assert!(
        text.contains("hello"),
        "expected 'hello' inline in prompt, got:\n{text}"
    );
    assert!(
        text.contains("world"),
        "expected 'world' inline in prompt, got:\n{text}"
    );
    assert!(
        !text.contains("[Pasted text"),
        "short paste should NOT be collapsed, got:\n{text}"
    );

    term.kill().await.ok();
}

/// Long paste (5 lines) should show as a collapsed block.
#[tokio::test]
async fn long_paste_shows_collapsed() {
    let term = spawn_codiv().await;

    let paste = bracketed_paste("line one\nline two\nline three\nline four\nline five");
    term.send_raw(&paste).await.expect("failed to send paste");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term, "02_long_paste_collapsed").await;

    let screen = term.screen().await;
    let text = screen.text();

    assert!(
        text.contains("[Pasted text"),
        "long paste should show collapsed indicator, got:\n{text}"
    );
    assert!(
        text.contains("5 lines"),
        "collapsed indicator should mention 5 lines, got:\n{text}"
    );
    assert!(
        !text.contains("line four"),
        "collapsed paste should not show full text, got:\n{text}"
    );

    term.kill().await.ok();
}

/// Backspace on a collapsed paste block removes the whole block.
#[tokio::test]
async fn backspace_removes_collapsed_paste() {
    let term = spawn_codiv().await;

    let paste = bracketed_paste("aaa\nbbb\nccc\nddd\neee");
    term.send_raw(&paste).await.expect("failed to send paste");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term, "03_before_backspace").await;

    let screen = term.screen().await;
    assert!(
        screen.text().contains("[Pasted text"),
        "paste should be collapsed before backspace"
    );

    term.send_key(Key::Backspace)
        .await
        .expect("failed to send backspace");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term, "04_after_backspace").await;

    let screen = term.screen().await;
    let text = screen.text();
    assert!(
        !text.contains("[Pasted text"),
        "backspace should remove collapsed paste, got:\n{text}"
    );

    term.kill().await.ok();
}

/// Boundary test: exactly 3 lines should be inline, 4 lines should be collapsed.
#[tokio::test]
async fn boundary_three_vs_four_lines() {
    // --- 3 lines: should be inline ---
    let term3 = spawn_codiv().await;

    let paste3 = bracketed_paste("one\ntwo\nthree");
    term3.send_raw(&paste3).await.expect("failed to send 3-line paste");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term3, "05_boundary_3lines_inline").await;

    let screen3 = term3.screen().await;
    let text3 = screen3.text();
    assert!(
        !text3.contains("[Pasted text"),
        "3-line paste should be inline, got:\n{text3}"
    );
    assert!(
        text3.contains("one"),
        "3-line paste text should be visible, got:\n{text3}"
    );

    term3.kill().await.ok();

    // --- 4 lines: should be collapsed ---
    let term4 = spawn_codiv().await;

    let paste4 = bracketed_paste("one\ntwo\nthree\nfour");
    term4.send_raw(&paste4).await.expect("failed to send 4-line paste");
    tokio::time::sleep(Duration::from_millis(300)).await;

    save_screenshot(&term4, "06_boundary_4lines_collapsed").await;

    let screen4 = term4.screen().await;
    let text4 = screen4.text();
    assert!(
        text4.contains("[Pasted text"),
        "4-line paste should be collapsed, got:\n{text4}"
    );
    assert!(
        text4.contains("4 lines"),
        "collapsed indicator should mention 4 lines, got:\n{text4}"
    );

    term4.kill().await.ok();
}

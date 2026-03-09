# Tick-Based Animation System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a tick-based animation system to the ratatui TUI that powers cursor blink, loading spinners, and status bar animations.

**Architecture:** A dedicated tick channel fires every 80ms and is added as a new arm in the existing `crossbeam_channel::select!` loop. An `AnimationState` struct tracks tick count and animation visibility flags. Rendering consults this state to show/hide cursors, cycle spinner frames, and pulse status indicators. The tick channel only triggers re-renders when animations are actually active.

**Tech Stack:** Rust, ratatui 0.30, crossterm 0.28, crossbeam-channel 0.5, vt100

---

### Task 1: Create AnimationState struct

**Files:**
- Create: `crates/slate/src/ui/terminal/animation.rs`
- Modify: `crates/slate/src/ui/terminal/mod.rs:8-14` (add `mod animation;`)

**Step 1: Create the animation module with AnimationState**

```rust
// crates/slate/src/ui/terminal/animation.rs
use std::time::{Duration, Instant};

/// Spinner frame characters (Braille pattern).
const SPINNER_FRAMES: &[char] = &['\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}'];

/// Tick interval for the animation timer.
pub(crate) const TICK_INTERVAL: Duration = Duration::from_millis(80);

/// Number of ticks between cursor blink toggles (~480ms at 80ms ticks).
const CURSOR_BLINK_TICKS: u64 = 6;

/// Tracks animation state across ticks.
pub(crate) struct AnimationState {
    tick_count: u64,
    cursor_visible: bool,
    /// Whether any animation is currently active (controls whether ticks trigger renders).
    has_active_animations: bool,
}

impl AnimationState {
    pub(crate) fn new() -> Self {
        Self {
            tick_count: 0,
            cursor_visible: true,
            has_active_animations: false,
        }
    }

    /// Advance one tick. Returns true if a re-render is needed.
    pub(crate) fn tick(&mut self) -> bool {
        self.tick_count = self.tick_count.wrapping_add(1);

        if !self.has_active_animations {
            return false;
        }

        // Toggle cursor blink
        if self.tick_count % CURSOR_BLINK_TICKS == 0 {
            self.cursor_visible = !self.cursor_visible;
        }

        true
    }

    /// Update which animations are active based on current UI state.
    pub(crate) fn update_active(
        &mut self,
        prompt_is_live: bool,
        is_executing: bool,
        agent_streaming: bool,
    ) {
        self.has_active_animations = prompt_is_live || is_executing || agent_streaming;

        // Reset cursor to visible when prompt becomes live (avoids starting invisible).
        if prompt_is_live && self.tick_count % CURSOR_BLINK_TICKS == 0 {
            // Handled by tick() toggle
        }
    }

    /// Whether the cursor should be visible right now (for blinking).
    pub(crate) fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Get the current spinner character.
    pub(crate) fn spinner_char(&self) -> char {
        let idx = (self.tick_count as usize) % SPINNER_FRAMES.len();
        SPINNER_FRAMES[idx]
    }

    /// Whether any animations are active (used to decide if tick should trigger render).
    pub(crate) fn is_active(&self) -> bool {
        self.has_active_animations
    }
}
```

**Step 2: Register the module**

In `crates/slate/src/ui/terminal/mod.rs`, add `mod animation;` after line 8 (after `mod daemon;`):

```rust
mod animation;
mod daemon;
mod event_loop;
// ... rest unchanged
```

**Step 3: Verify it compiles**

Run: `cargo check -p slate`
Expected: compiles with no errors (unused warnings are fine at this stage)

**Step 4: Commit**

```bash
git add crates/slate/src/ui/terminal/animation.rs crates/slate/src/ui/terminal/mod.rs
git commit -m "feat: add AnimationState struct for tick-based animations"
```

---

### Task 2: Add tick channel to the event loop

**Files:**
- Modify: `crates/slate/src/ui/terminal/event_loop.rs`
- Modify: `crates/slate/src/ui/terminal/io.rs`

**Step 1: Add spawn_tick_channel to io.rs**

Append this function to `crates/slate/src/ui/terminal/io.rs`:

```rust
/// Spawn a background thread that sends tick events at a fixed interval.
/// The channel is unbounded so ticks are never blocked by a slow consumer;
/// stale ticks are harmless (they just trigger a render check).
pub(crate) fn spawn_tick_channel(interval: Duration) -> crossbeam_channel::Receiver<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("tick-timer".into())
        .spawn(move || loop {
            std::thread::sleep(interval);
            if tx.send(()).is_err() {
                break;
            }
        })
        .expect("spawn tick timer");
    rx
}
```

**Step 2: Wire tick channel into event_loop.rs**

In `event_loop()` in `event_loop.rs`:

a) Add import at the top (after existing imports from `super`):
```rust
use super::animation::AnimationState;
```

b) After line 63 (`let mut tool_result_modal = ToolResultModal::new();`), add:
```rust
let mut anim = AnimationState::new();
```

c) After line 70 (`let crossterm_rx = terminal_io::spawn_crossterm_reader();`), add:
```rust
let tick_rx = terminal_io::spawn_tick_channel(super::animation::TICK_INTERVAL);
```

d) Inside the `crossbeam_channel::select!` block (after the `default(timeout)` arm at line 625-628), add a new arm **before** `default`:
```rust
recv(tick_rx) -> _ => {
    anim.update_active(*prompt_is_live, pending_command.is_some(), agent_streaming);
    if anim.tick() {
        needs_render = true;
    }
}
```

**Step 3: Verify it compiles**

Run: `cargo check -p slate`
Expected: compiles (anim may warn as unused in render — that's fine)

**Step 4: Commit**

```bash
git add crates/slate/src/ui/terminal/io.rs crates/slate/src/ui/terminal/event_loop.rs
git commit -m "feat: add tick channel to event loop for animation timing"
```

---

### Task 3: Implement cursor blink in render

**Files:**
- Modify: `crates/slate/src/ui/terminal/render.rs`
- Modify: `crates/slate/src/ui/terminal/event_loop.rs`

**Step 1: Add anim parameter to render_frame**

In `render.rs`, add `anim: &AnimationState` parameter to `render_frame()` (after `tool_result_modal: &ToolResultModal`):

```rust
pub(crate) fn render_frame(
    // ... existing params ...
    tool_result_modal: &ToolResultModal,
    anim: &super::animation::AnimationState,
) -> Result<(), Box<dyn std::error::Error>> {
```

**Step 2: Use anim.cursor_visible() for cursor visibility**

Replace line 104 in render.rs:
```rust
// Old:
let cursor_visible = scroll_offset == 0;
// New:
let cursor_visible = scroll_offset == 0 && anim.cursor_visible();
```

**Step 3: Update the render_frame call in event_loop.rs**

In `event_loop.rs`, update the `render_frame()` call (around line 94) to pass `&anim` as the last argument:

```rust
render_frame(
    // ... existing args ...
    &tool_result_modal,
    &anim,
)?;
```

**Step 4: Verify it compiles and cursor blinks**

Run: `cargo check -p slate`
Expected: compiles with no errors

**Step 5: Commit**

```bash
git add crates/slate/src/ui/terminal/render.rs crates/slate/src/ui/terminal/event_loop.rs
git commit -m "feat: implement cursor blink using animation tick"
```

---

### Task 4: Add spinner to status bar during execution/streaming

**Files:**
- Modify: `crates/slate/src/ui/terminal/render.rs`

**Step 1: Add anim parameter to render_status_bar**

Update `render_status_bar()` signature to accept the animation state:

```rust
pub(crate) fn render_status_bar(
    // ... existing params ...
    context_usage: (usize, usize),
    anim: &super::animation::AnimationState,
) {
```

**Step 2: Replace running_indicator with spinner**

Replace line 323:
```rust
// Old:
let running_indicator = if is_executing { " [running]" } else { "" };
// New:
let running_indicator = if is_executing {
    format!(" {} running", anim.spinner_char())
} else {
    String::new()
};
```

**Step 3: Update format string to use running_indicator as String**

The `running_indicator` is now a `String` instead of `&str`. Update the format usage — it should already work since `format!` accepts both, but verify the `left` construction still compiles. The existing code at lines 335-352 uses `running_indicator` in a `format!()` call which handles both `&str` and `String`.

**Step 4: Pass anim to render_status_bar call**

In `render_frame()`, update the `render_status_bar()` call (around line 183) to pass `anim`:

```rust
render_status_bar(
    // ... existing args ...
    context_usage,
    anim,
);
```

**Step 5: Verify it compiles**

Run: `cargo check -p slate`
Expected: compiles with no errors

**Step 6: Commit**

```bash
git add crates/slate/src/ui/terminal/render.rs
git commit -m "feat: add animated spinner to status bar during command execution"
```

---

### Task 5: Add spinner to gutter during agent streaming

**Files:**
- Modify: `crates/slate/src/ui/terminal/render.rs`

**Step 1: Animate the live prompt gutter marker during streaming**

In `render_frame()`, find the live prompt gutter rendering (around line 140-149). After the existing `if *prompt_is_live && scroll_offset == 0` block, add a new block for the streaming spinner in the gutter. Replace the entire block:

```rust
// Live prompt: draw `>` at the current cursor row.
if *prompt_is_live && scroll_offset == 0 {
    let (cursor_row, _) = parser.screen().cursor_position();
    let row = term_area.top() + cursor_row;
    if row < term_area.bottom() {
        buf[(term_area.left(), row)]
            .set_char('>')
            .set_fg(Color::Cyan);
    }
} else if agent_streaming && scroll_offset == 0 {
    // Show animated spinner in gutter while agent is streaming.
    let (cursor_row, _) = parser.screen().cursor_position();
    let row = term_area.top() + cursor_row;
    if row < term_area.bottom() {
        buf[(term_area.left(), row)]
            .set_char(anim.spinner_char())
            .set_fg(Color::Yellow);
    }
}
```

Note: `agent_streaming` is already a parameter of `render_frame` — it's passed but currently only used for the prompt-is-live guard. We need to make sure it's accessible in this scope. Looking at the current code, `agent_streaming` is a parameter of `render_frame()` — yes, it is (line 41).

**Step 2: Verify it compiles**

Run: `cargo check -p slate`
Expected: compiles with no errors

**Step 3: Commit**

```bash
git add crates/slate/src/ui/terminal/render.rs
git commit -m "feat: add animated spinner gutter marker during agent streaming"
```

---

### Task 6: Add pulsing daemon status indicator in status bar

**Files:**
- Modify: `crates/slate/src/ui/terminal/render.rs`

**Step 1: Add pulsing dot to daemon status**

In `render_status_bar()`, update the daemon_status construction to include a pulsing dot. Replace the daemon_status block (lines 306-321):

```rust
let daemon_dot = if daemon_connected {
    // Alternate between bright and dim dot every ~6 ticks
    if anim.cursor_visible() { "\u{25CF}" } else { "\u{25CB}" }
} else {
    "\u{25CB}" // always hollow when disconnected
};

let daemon_status = if daemon_connected && daemon_timestamp > 0 {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_s = now_ms.saturating_sub(daemon_timestamp) / 1000;
    if age_s < 30 {
        format!("{} connected", daemon_dot)
    } else {
        format!("{} stale", daemon_dot)
    }
} else if daemon_connected {
    format!("{} connected", daemon_dot)
} else {
    format!("{} offline", daemon_dot)
};
```

**Step 2: Update the right-side format string**

The `daemon_status` is now a `String`. Update line 334 to remove the `daemon:` prefix since it's now implicit:

```rust
let right = format!(" {}{} | {} ", model_part, daemon_status, VERSION);
```

This should work without changes since `format!` handles `String` via `Display`.

**Step 3: Verify it compiles**

Run: `cargo check -p slate`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add crates/slate/src/ui/terminal/render.rs
git commit -m "feat: add pulsing daemon status indicator in status bar"
```

---

### Task 7: Add unit tests for AnimationState

**Files:**
- Modify: `crates/slate/src/ui/terminal/animation.rs`

**Step 1: Add tests module**

Append to `animation.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_cursor_visible() {
        let anim = AnimationState::new();
        assert!(anim.cursor_visible());
    }

    #[test]
    fn tick_without_active_animations_returns_false() {
        let mut anim = AnimationState::new();
        assert!(!anim.tick());
    }

    #[test]
    fn tick_with_active_animations_returns_true() {
        let mut anim = AnimationState::new();
        anim.update_active(true, false, false);
        assert!(anim.tick());
    }

    #[test]
    fn cursor_blinks_after_threshold_ticks() {
        let mut anim = AnimationState::new();
        anim.update_active(true, false, false);
        assert!(anim.cursor_visible());

        // Tick 6 times to trigger first blink toggle
        for _ in 0..6 {
            anim.tick();
        }
        assert!(!anim.cursor_visible());

        // Tick 6 more to toggle back
        for _ in 0..6 {
            anim.tick();
        }
        assert!(anim.cursor_visible());
    }

    #[test]
    fn spinner_cycles_through_frames() {
        let mut anim = AnimationState::new();
        let first = anim.spinner_char();
        anim.tick_count = 1;
        let second = anim.spinner_char();
        assert_ne!(first, second);

        // After a full cycle it wraps
        anim.tick_count = 8;
        assert_eq!(anim.spinner_char(), first);
    }

    #[test]
    fn update_active_reflects_state() {
        let mut anim = AnimationState::new();
        assert!(!anim.is_active());

        anim.update_active(false, true, false);
        assert!(anim.is_active());

        anim.update_active(false, false, true);
        assert!(anim.is_active());

        anim.update_active(false, false, false);
        assert!(!anim.is_active());
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p slate -- animation`
Expected: all 5 tests pass

**Step 3: Commit**

```bash
git add crates/slate/src/ui/terminal/animation.rs
git commit -m "test: add unit tests for AnimationState"
```

---

### Task 8: Final integration and manual verification

**Step 1: Full build check**

Run: `cargo build -p slate`
Expected: compiles with no errors

**Step 2: Run all tests**

Run: `cargo test -p slate`
Expected: all tests pass

**Step 3: Commit any remaining changes**

```bash
git add -A
git commit -m "feat: tick-based animation system with cursor blink, spinners, and status pulsing"
```

---
name: codiv-ratatui
description: >
  Activate this skill whenever someone asks about the **codiv** Rust TUI client and their
  question involves the terminal interface. "codiv" (not "codivd", not "streamdown-rs") is a
  terminal app built with ratatui, crossterm, tui-term, and vt100. Any question about codiv
  that touches rendering, layout, widgets, the event loop, channels, resize, flickering,
  scrolling, popups, overlays, the status bar, the input bar, DaemonMessage handling in the
  UI, panic recovery, raw mode, alternate screen, or terminal lifecycle belongs here. The user
  will rarely say "ratatui" — they'll say "codiv" plus describe a visual, interactive, or
  terminal problem. That combination is your trigger. Do NOT use for: codivd daemon internals,
  AI provider config, markdown/streamdown-rs parsing, or tab-completion logic (unless it's
  about the popup widget's rendering/positioning).
---

# Codiv TUI — ratatui Guide

This skill covers ratatui usage exclusively in the **codiv** crate (the TUI client).
The codivd daemon has no ratatui code — see the `codiv-aisdk` skill for daemon work.

## Codiv Source Layout

All UI source lives under `crates/codiv/src/`:

```
ui/
├── terminal/
│   ├── mod.rs           ← Terminal::run() entry, setup/teardown, constants
│   ├── event_loop.rs    ← crossbeam select! loop, event dispatch
│   ├── render.rs        ← frame.draw() callback, layout split, widget rendering
│   ├── daemon.rs        ← handle_daemon_message(), StreamChunk processing
│   ├── io.rs            ← background threads: crossterm poller, tick timer
│   ├── animation.rs     ← AnimationState, tick-gated re-render control
│   ├── utils.rs         ← scroll helpers (true_scrollback_len, scroll_to_focused)
│   └── state.rs         ← AppState, MAX_SCROLLBACK, mode tracking
├── input.rs             ← InputLine: buffer, cursor, history
├── completion_popup.rs  ← CompletionPopup overlay widget
├── blocks.rs            ← InputMode enum (Command / Ai), BlockRegistry
└── selection.rs         ← mouse text selection + clipboard (arboard)
```

## Key Dependencies

| Crate | Version | Role |
|---|---|---|
| `ratatui` | 0.30 | Layout, Terminal, Frame, widgets |
| `crossterm` | 0.28 | Backend, event reading, raw mode, alternate screen |
| `tui-term` | 0.3 | `PseudoTerminal` widget wrapping vt100 screen |
| `vt100` | 0.16 | VT100 parser — the single content store for all output |
| `crossbeam-channel` | 0.5 | Multi-producer channels for `select!` multiplexing |
| `arboard` | 3 | Clipboard for text selection copy |

---

## 1. Layout Architecture

The current layout is a **two-zone ratatui split**, not three. The input line lives
inside the vt100 parser buffer (injected via ANSI escapes), so ratatui sees only two regions:

```rust
// render.rs
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Min(1),                     // PTY area (content + embedded input line)
        Constraint::Length(STATUS_BAR_HEIGHT),   // Status bar (1 row)
    ])
    .split(area);
```

A 2-column gutter is carved out manually for the prompt character (`>` or `$`):

```rust
let content_area = Rect {
    x: term_area.x + PROMPT_GUTTER_WIDTH,     // PROMPT_GUTTER_WIDTH = 2
    width: term_area.width.saturating_sub(PROMPT_GUTTER_WIDTH),
    ..term_area
};
```

**Constants** (mod.rs):
- `STATUS_BAR_HEIGHT = 1`
- `PROMPT_GUTTER_WIDTH = 2`
- `MAX_SCROLLBACK = 10_000` (state.rs)

**Alt-screen mode**: When the user runs a fullscreen app (vim, htop), the layout
collapses — `PseudoTerminal` fills the entire `area` with no status bar or gutter.

### If you're adding a third zone

To split the AI output from the PTY shell as separate ratatui regions, use three
constraints. `Fill(1)` takes remaining space after fixed zones are satisfied:

```rust
let [ai_area, pty_area, input_area] = area.layout(&Layout::vertical([
    Constraint::Fill(1),      // AI output: expands to fill
    Constraint::Min(5),       // PTY shell: at least 5 rows
    Constraint::Length(3),    // Input bar: fixed 3 rows (border + content)
]));
```

Constraint priority: Min > Max > Length > Percentage > Ratio > Fill.

---

## 2. Event Loop

The event loop uses **synchronous `crossbeam_channel::select!`** (not tokio).
Four channels plus a timeout are multiplexed:

```rust
// event_loop.rs (simplified)
let mut needs_render = true;

loop {
    if needs_render {
        term.draw(|frame| render(frame, &parser, &state, ...))?;
        needs_render = false;
    }

    crossbeam_channel::select! {
        recv(crossterm_rx) -> event => {
            // keyboard, mouse, resize events
            needs_render = true;
        }
        recv(pty_rx) -> bytes => {
            parser.process(&bytes);
            needs_render = true;
        }
        recv(daemon_rx) -> msg => {
            handle_daemon_message(msg, &mut parser, &mut state, ...);
            // Batch-drain remaining messages before rendering
            while let Ok(msg2) = daemon_rx.try_recv() {
                handle_daemon_message(msg2, ...);
            }
            needs_render = true;
        }
        recv(tick_rx) -> _ => {
            if anim.tick() {  // only true when has_active_animations
                needs_render = true;
            }
        }
        default(timeout) => {
            needs_render = true;  // heartbeat / activity timeout
        }
    }

    // Post-select PTY drain
    while let Ok(extra) = pty_rx.try_recv() {
        parser.process(&extra);
    }
}
```

**Background threads** (io.rs):
- Crossterm poller: `crossterm::event::poll(100ms)` in a loop → sends `Event` to `crossterm_rx`
- Tick timer: sleeps `TICK_INTERVAL` (80ms) → sends `()` to `tick_rx`

**Render cadence**: At most one `term.draw()` per `select!` iteration.
The `needs_render` dirty flag prevents redundant frames. Batch-draining daemon
messages (and PTY bytes) after the initial recv merges N messages into one frame.

**Selective channel gating**: When no command is pending, `pty_rx` is swapped to
`crossbeam_channel::never()` to avoid blocking the completion engine.

**Adaptive timeout**: `compute_next_timeout()` returns the minimum of:
completion init (50ms), activity deadline (300s), heartbeat (10s), or fallback (60s).

---

## 3. Input Bar

The input bar is **not a ratatui widget**. It's injected directly into the vt100
parser's screen buffer using ANSI escape sequences before `term.draw()`:

```rust
// render.rs — before drawing frame
// Clear line, write input text
parser.process(format!("\r\x1b[K{}", input_text).as_bytes());

// Position cursor via CSI
let target_col = input.cursor_position();          // chars, not bytes
let current_col = input_text.chars().count();
if current_col > target_col {
    parser.process(format!("\x1b[{}D", current_col - target_col).as_bytes());
}
```

`InputLine` (input.rs) tracks:
- `buffer: String` — the raw input text
- `cursor: usize` — byte offset into buffer
- `cursor_position() -> usize` — character count up to cursor (handles multibyte)

**Mode switching**: `InputMode` enum has `Command` and `Ai` variants. Tab on empty
input toggles the mode. The gutter character changes (`>` / `$`) and the color
changes (Cyan / White).

### If you're making the input bar a ratatui widget

Use `Paragraph` inside a bordered `Block`, with `frame.set_cursor_position()`:

```rust
let input_widget = Paragraph::new(self.input.as_str())
    .style(Style::default().fg(if ai_mode { Color::Cyan } else { Color::White }))
    .block(Block::bordered().title(if ai_mode { "AI" } else { "Shell" }));
frame.render_widget(input_widget, input_area);

// Cursor: +1 for left border
frame.set_cursor_position(Position::new(
    input_area.x + self.character_index as u16 + 1,
    input_area.y + 1,
));
```

The byte-index-to-char-index conversion matters for multibyte characters:
```rust
fn byte_index(&self) -> usize {
    self.input.char_indices()
        .map(|(i, _)| i)
        .nth(self.character_index)
        .unwrap_or(self.input.len())
}
```

---

## 4. Completion Popup Overlay

The popup uses the standard ratatui overlay pattern (completion_popup.rs):

```rust
// 1. Compute geometry from cursor anchor
let anchor_x = term_area.x + PROMPT_GUTTER_WIDTH + cursor_col as u16;
let anchor_y = cursor_row;

// 2. Position: below cursor if room, above if not
let popup_rect = if space_below >= needed { below } else { above };

// 3. Clear background, then render List
frame.render_widget(Clear, popup_rect);

let items: Vec<ListItem> = candidates.iter().enumerate().map(|(i, c)| {
    let style = if i == selected {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else {
        Style::default().bg(Color::DarkGray).fg(Color::White)
    };
    ListItem::new(c.as_str()).style(style)
}).collect();

let list = List::new(items).block(
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray))
        .style(Style::default().bg(Color::DarkGray)),
);
frame.render_widget(list, popup_rect);
```

Max visible: 10 rows. The popup manages its own scroll offset (not `ListState`)
to keep the selected item visible.

For a centered popup instead, use `Rect::centered()`:
```rust
let popup_area = frame.area().centered(
    Constraint::Percentage(60),
    Constraint::Percentage(40),
);
frame.render_widget(Clear, popup_area);
frame.render_widget(my_popup_widget, popup_area);
```

---

## 5. Terminal Resize

Resize is handled manually — `ratatui::init()`'s autoresize is not used:

```rust
// event_loop.rs
Event::Resize(cols, rows) => {
    let parser_rows = parser_rows_from_term_height(rows);
    let parser_cols = if was_alt_screen {
        cols.max(1)       // full width in alt-screen
    } else {
        parser_cols_from_term_width(cols)  // subtract gutter
    };
    parser.screen_mut().set_size(parser_rows, parser_cols);
    bash.resize(parser_rows, parser_cols);
    md_stream.set_width(parser_cols);     // reflow markdown
}
```

Helpers (mod.rs):
```rust
fn parser_rows_from_term_height(h: u16) -> u16 { h - STATUS_BAR_HEIGHT }
fn parser_cols_from_term_width(w: u16) -> u16  { w - PROMPT_GUTTER_WIDTH }
```

Alt-screen transitions also trigger a resize (different width calculation).

---

## 6. IPC → State Updates

`DaemonMessage` variants (from codiv-common) and their render effects:

| Message | Handler | Visual Effect |
|---|---|---|
| `AgentStreamChunk { chunk: StreamChunk::Text(t) }` | Feeds text through `MarkdownStream` → ANSI bytes → `parser.process()` | AI text appears in PTY area |
| `StreamChunk::Reasoning(t)` | Writes dim "Thinking..." placeholder, accumulates in buffer | Animated thinking dots |
| `StreamChunk::ToolCallDelta { tool_name, .. }` | Flushes markdown, writes bold yellow tool header | Tool call appears |
| `StreamChunk::ToolCall { name, arguments }` | Updates header in-place (cursor-up + clear + rewrite) | Full tool args shown |
| `StreamChunk::ToolResult { name, result }` | Writes 2-line summary (bold green header + colored body) | Tool result block |
| `AgentComplete { summary }` | Flushes final markdown, closes AI block, resets `agent_streaming` | Streaming stops |
| `AgentMeta { model_alias, tokens, .. }` | Updates status bar display state | Status bar refreshes |
| `Error { message }` | Prints in red, resets streaming state | Error shown inline |
| `Heartbeat { timestamp }` | Updates `last_daemon_timestamp` | Status bar staleness |

All content goes through `parser.process(bytes)` — the vt100 parser is the single
source of truth for everything rendered in the PTY area.

---

## 7. PTY Rendering via tui-term

The `PseudoTerminal` widget from `tui-term` renders a `vt100::Screen`:

```rust
// render.rs
let pseudo_term = PseudoTerminal::new(parser.screen())
    .cursor(PtCursor::default().visibility(cursor_visible));
frame.render_widget(pseudo_term, content_area);
```

`cursor_visible` is `scroll_offset == 0` — the cursor hides when scrolled up.

Parser initialization (mod.rs):
```rust
let parser = vt100::Parser::new(parser_rows, parser_cols, MAX_SCROLLBACK);
// MAX_SCROLLBACK = 10_000 lines
```

Everything visible in the PTY zone — user commands, AI output, tool results,
the input line — is written as bytes into this single parser. The `PseudoTerminal`
widget just projects the current screen state into a ratatui `Rect`.

---

## 8. Scroll Behavior

Scrolling is driven by `scroll_offset: usize` passed to vt100:

```rust
parser.screen_mut().set_scrollback(scroll_offset);
```

Event mapping:
```rust
MouseEventKind::ScrollUp   => scroll_offset = scroll_offset.saturating_add(3),
MouseEventKind::ScrollDown => scroll_offset = scroll_offset.saturating_sub(3),
KeyCode::PageUp            => scroll_offset = scroll_offset.saturating_add(10),
KeyCode::PageDown          => scroll_offset = scroll_offset.saturating_sub(10),
```

Enter and Ctrl+C reset `scroll_offset` to 0 (snap to bottom).

**Block navigation** — `scroll_to_focused()` (utils.rs) centers a target block:
```rust
fn scroll_to_focused(parser: &mut vt100::Parser, target_scrollback_line: usize) {
    let screen_rows = parser.screen().size().0 as usize;
    let total = true_scrollback_len(parser);  // measures actual VecDeque length
    let desired_top = target_scrollback_line.saturating_sub(screen_rows / 2);
    let natural_top = total;
    scroll_offset = natural_top.saturating_sub(desired_top);
    parser.screen_mut().set_scrollback(scroll_offset);
}
```

### Adding a Scrollbar widget

If you want a visible scrollbar track alongside the vt100 area:

```rust
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

let total_lines = true_scrollback_len(&parser) + parser.screen().size().0 as usize;
let mut scrollbar_state = ScrollbarState::new(total_lines).position(scroll_offset);

frame.render_stateful_widget(
    Scrollbar::new(ScrollbarOrientation::VerticalRight),
    content_area,
    &mut scrollbar_state,
);
```

---

## 9. Frame Budget & Performance

Patterns that keep rendering efficient:

1. **Dirty flag**: `needs_render: bool` — only call `term.draw()` when something changed.
2. **Batch draining**: After a daemon message, drain all queued messages with `try_recv()`
   before rendering. Same for PTY bytes. This merges N events into one frame.
3. **Tick-gated animation**: The 80ms tick only triggers a render when
   `has_active_animations` is set (executing, streaming, thinking, or pending tool).
   Idle state produces zero tick-driven frames.
4. **Selective channel gating**: Swap `pty_rx` to `crossbeam_channel::never()` when
   idle to prevent blocking other channels.
5. **ratatui double-buffering**: `term.draw()` diffs the new buffer against the
   previous one internally and only writes changed cells to the terminal.

If you notice frame drops during heavy streaming, the likely bottleneck is
`parser.process()` calls or markdown rendering — profile those, not ratatui.

---

## 10. Terminal Setup & Teardown

Current pattern (mod.rs) — manual setup and finally-style cleanup:

```rust
pub fn run(...) -> Result<()> {
    // Setup
    terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    // Run event loop, capture result
    let result = event_loop(&mut term, ...);

    // Teardown (always runs)
    terminal::disable_raw_mode()?;
    execute!(term.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
    term.show_cursor()?;

    result
}
```

**No panic hook is installed.** If the process panics mid-run, the terminal will
be left in raw mode / alternate screen until the OS cleans up on exit.

### Adding a panic hook

Use ratatui 0.30's built-in `set_panic_hook()` pattern:

```rust
fn set_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        hook(info);
    }));
}
```

Call this right after `enable_raw_mode()`. Or use `ratatui::init()` which installs
the hook automatically — but that changes the Terminal construction pattern.

---

## Quick Reference: ratatui 0.30 API

For full ratatui API patterns and examples, see `references/ratatui-api.md`.

Key things to know:
- `Layout::vertical([...])` / `Layout::horizontal([...])`  — shorthand constructors
- `area.layout(&layout)` — compile-time array destructure: `let [a, b, c] = area.layout(...)`
- `frame.render_widget(widget, area)` — stateless
- `frame.render_stateful_widget(widget, area, &mut state)` — stateful (List, Scrollbar, Table)
- `frame.set_cursor_position(Position::new(x, y))` — show blinking cursor
- `frame.render_widget(Clear, area)` — erase background before overlay
- `Rect::centered(h_constraint, v_constraint)` — center a popup
- Only handle `KeyEventKind::Press`, not Release/Repeat

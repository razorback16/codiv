# ratatui 0.30 API Reference

Comprehensive reference for ratatui patterns used in or useful for the codiv TUI.
Read this file when working on new ratatui features or debugging widget behavior.

## Table of Contents

1. [Terminal Lifecycle](#terminal-lifecycle)
2. [Layout System](#layout-system)
3. [Widget Trait](#widget-trait)
4. [Common Widgets](#common-widgets)
5. [Styling](#styling)
6. [Rect Helpers](#rect-helpers)
7. [Input Handling with crossterm](#input-handling)
8. [PseudoTerminal (tui-term)](#pseudoterminal)

---

## Terminal Lifecycle

### Quick init (auto panic hook)

```rust
fn main() -> Result<()> {
    let mut terminal = ratatui::init();  // raw mode + alt screen + panic hook
    let result = app.run(&mut terminal);
    ratatui::restore();                  // teardown (also called by panic hook)
    result
}
```

`ratatui::init()` calls `set_panic_hook()` internally, which wraps the previous
panic hook so `restore()` runs before the panic message prints.

### Manual init (full control, what codiv currently uses)

```rust
use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use crossterm::execute;
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};

// Setup
enable_raw_mode()?;
execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
let backend = CrosstermBackend::new(stdout());
let mut terminal = Terminal::new(backend)?;

// Install panic hook manually
let hook = std::panic::take_hook();
std::panic::set_hook(Box::new(move |info| {
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    hook(info);
}));

// ... event loop ...

// Teardown
disable_raw_mode()?;
execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
terminal.show_cursor()?;
```

### Type alias

```rust
pub type DefaultTerminal = Terminal<CrosstermBackend<Stdout>>;
```

---

## Layout System

### Constructors

```rust
Layout::vertical([...])              // Direction::Vertical shorthand
Layout::horizontal([...])            // Direction::Horizontal shorthand
Layout::new(Direction::Vertical, [Constraint::Length(3), Constraint::Fill(1)])
```

### Constraint variants (resolution priority order)

| Constraint | Priority | Meaning |
|---|---|---|
| `Min(n)` | Highest | At least n cells |
| `Max(n)` | High | At most n cells |
| `Length(n)` | Medium | Exactly n cells |
| `Percentage(n)` | Low | n% of total |
| `Ratio(a, b)` | Low | a/b of total |
| `Fill(w)` | Lowest | Proportional fill of remaining space (weight w) |

`u16` auto-converts to `Constraint::Length`, so `Layout::vertical([1, 3])` works.

### Applying layouts

```rust
// Compile-time checked (preferred when count is known):
let [top, middle, bottom] = area.layout(&Layout::vertical([
    Constraint::Length(1),
    Constraint::Fill(1),
    Constraint::Length(3),
]));

// Runtime (returns Rc<[Rect]>):
let chunks = Layout::vertical([...]).split(area);
let top = chunks[0];
```

### Flex

```rust
Layout::vertical([Constraint::Length(10)])
    .flex(Flex::Center)     // centers the single chunk
    .split(area);
```

Flex options: `Start` (default), `Center`, `End`, `SpaceAround`, `SpaceBetween`.

---

## Widget Trait

### Stateless widget

```rust
pub trait Widget {
    fn render(self, area: Rect, buf: &mut Buffer);
}

// Implement for reference to allow reuse:
impl Widget for &MyWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // draw into buf
    }
}
```

### Stateful widget

```rust
pub trait StatefulWidget {
    type State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}

// Usage:
frame.render_stateful_widget(list, area, &mut list_state);
```

### Frame rendering methods

```rust
frame.area()                                          // full terminal Rect
frame.render_widget(widget, area)                     // stateless
frame.render_stateful_widget(widget, area, &mut st)   // stateful
frame.set_cursor_position(Position::new(x, y))        // show cursor
frame.render_widget(Clear, area)                      // erase for overlay
```

---

## Common Widgets

### Block (border container)

```rust
Block::bordered()                     // all four borders
Block::bordered().title("Title")
Block::default().borders(Borders::TOP | Borders::BOTTOM)
Block::bordered().border_style(Style::default().fg(Color::Gray))
block.inner(area)                     // Rect inside the borders
```

### Paragraph (text display)

```rust
Paragraph::new("simple text")
Paragraph::new(Line::from(vec!["bold".bold(), " normal".into()]))
Paragraph::new(text)
    .block(Block::bordered())
    .scroll((vertical_offset, 0))     // manual scroll
    .wrap(Wrap { trim: true })        // enable wrapping
    .alignment(Alignment::Center)
```

### List (item list with optional selection)

```rust
// Stateless
let items = vec![ListItem::new("a"), ListItem::new("b")];
frame.render_widget(List::new(items).block(Block::bordered()), area);

// Stateful (with selection tracking)
let mut state = ListState::default();
state.select(Some(0));
frame.render_stateful_widget(
    List::new(items)
        .highlight_style(Style::new().reversed())
        .highlight_symbol(">> "),
    area,
    &mut state,
);

// Navigation
state.select_next();
state.select_previous();
state.select_first();
state.select_last();
```

### Scrollbar

```rust
let mut scroll_state = ScrollbarState::new(content_length).position(offset);
frame.render_stateful_widget(
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(Some("↑"))
        .end_symbol(Some("↓")),
    area.inner(Margin { vertical: 1, horizontal: 0 }),
    &mut scroll_state,
);
```

### Clear (overlay eraser)

```rust
frame.render_widget(Clear, popup_area);  // always before overlay widget
```

---

## Styling

```rust
use ratatui::style::{Color, Modifier, Style, Stylize};

// Builder pattern
Style::default().fg(Color::Cyan).bg(Color::Black).add_modifier(Modifier::BOLD)

// Stylize trait (chainable on any widget/text type)
"text".bold().cyan().on_black()
Line::from("hello").centered().bold()

// Common colors
Color::Red, Color::Green, Color::Yellow, Color::Blue, Color::Cyan,
Color::White, Color::Gray, Color::DarkGray, Color::Black,
Color::Rgb(r, g, b), Color::Indexed(n),  // 256-color
Color::Reset  // terminal default
```

---

## Rect Helpers

```rust
// Centering (for popups)
let popup = area.centered(
    Constraint::Percentage(60),   // width
    Constraint::Percentage(40),   // height
);

// Sub-rects
area.inner(Margin { vertical: 1, horizontal: 2 })  // shrink by margin
Rect::new(x, y, width, height)                     // absolute position

// Manual carving
Rect {
    x: area.x + offset,
    width: area.width.saturating_sub(offset),
    ..area   // keep y and height
}
```

---

## Input Handling

### crossterm event reading (background thread pattern)

```rust
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};

// In a background thread:
loop {
    if event::poll(Duration::from_millis(100))? {
        let event = event::read()?;
        tx.send(event)?;
    }
}

// In main loop, only handle Press events:
if let Event::Key(key) = event {
    if key.kind == KeyEventKind::Press {
        match key.code {
            KeyCode::Char(c) => { ... }
            KeyCode::Enter => { ... }
            KeyCode::Backspace => { ... }
            KeyCode::Left | KeyCode::Right => { ... }
            KeyCode::Up | KeyCode::Down => { ... }
            KeyCode::PageUp | KeyCode::PageDown => { ... }
            KeyCode::Tab => { ... }
            KeyCode::Esc => { ... }
            _ => {}
        }
    }
}

// Mouse events:
if let Event::Mouse(mouse) = event {
    match mouse.kind {
        MouseEventKind::ScrollUp => { ... }
        MouseEventKind::ScrollDown => { ... }
        MouseEventKind::Down(MouseButton::Left) => { ... }
        MouseEventKind::Drag(MouseButton::Left) => { ... }
        MouseEventKind::Up(MouseButton::Left) => { ... }
        _ => {}
    }
}

// Resize:
if let Event::Resize(cols, rows) = event {
    // Update parser size, PTY size, markdown width
}
```

### Modifier keys

```rust
if key.modifiers.contains(KeyModifiers::CONTROL) {
    match key.code {
        KeyCode::Char('c') => { /* interrupt */ }
        KeyCode::Char('l') => { /* clear */ }
        KeyCode::Char('a') => { /* home */ }
        KeyCode::Char('e') => { /* end */ }
        _ => {}
    }
}
```

---

## PseudoTerminal (tui-term 0.3 + vt100 0.16)

### Setup

```rust
use vt100::Parser;
use tui_term::widget::PseudoTerminal;

let mut parser = vt100::Parser::new(rows, cols, scrollback_limit);
```

### Processing output

```rust
// PTY bytes, ANSI-rendered markdown, tool headers — all go here:
parser.process(bytes);
```

### Rendering

```rust
let pt = PseudoTerminal::new(parser.screen())
    .cursor(PtCursor::default().visibility(show_cursor));
frame.render_widget(pt, content_area);
```

### Scrollback

```rust
// Set scroll offset (0 = bottom, higher = further back)
parser.screen_mut().set_scrollback(offset);

// Measure total scrollback
parser.screen_mut().set_scrollback(usize::MAX);  // vt100 clamps to real length
let total = parser.screen().scrollback();         // read clamped value
parser.screen_mut().set_scrollback(original);     // restore
```

### Resize

```rust
parser.screen_mut().set_size(new_rows, new_cols);
```

### Writing ANSI sequences directly

```rust
// Clear current line and write text:
parser.process(b"\r\x1b[K");
parser.process(text.as_bytes());

// Move cursor back N columns:
parser.process(format!("\x1b[{}D", n).as_bytes());

// Dim text:
parser.process(b"\x1b[90m");
parser.process(text.as_bytes());
parser.process(b"\x1b[0m");

// Bold yellow:
parser.process(b"\x1b[1;33m");

// Cursor up + clear line (for in-place updates):
parser.process(b"\x1b[A\x1b[2K");
```

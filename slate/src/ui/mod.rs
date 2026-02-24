//! UI module — ratatui-based linear scroll-down terminal interface.
//!
//! This module contains the terminal UI components:
//! - [`scroll_buffer`]: Scrollback buffer with styled lines and viewport windowing.
//! - [`input`]: Line editor with history and tab-completion.
//! - [`terminal`]: Main event loop, rendering, and command dispatch.

pub mod input;
pub mod scroll_buffer;
pub mod terminal;

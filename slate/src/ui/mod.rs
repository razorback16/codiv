//! UI module — ratatui-based linear scroll-down terminal interface.
//!
//! This module contains the terminal UI components:
//! - [`input`]: Line editor with history and tab-completion.
//! - [`terminal`]: Main event loop, rendering, and command dispatch.

pub mod input;
pub mod terminal;

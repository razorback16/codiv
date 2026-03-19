//! UI module — ratatui-based linear scroll-down terminal interface.
//!
//! This module contains the terminal UI components:
//! - [`input`]: Line editor with history and tab-completion.
//! - [`terminal`]: Main event loop, rendering, and command dispatch.

pub mod blocks;
pub mod color_downgrade;
pub mod completion_popup;
pub mod diff;
pub mod input;
pub mod terminal;
pub mod theme;
pub mod tool_modal;

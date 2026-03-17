//! Key event handling, split into focused submodules.

mod block_nav;
mod completion;
mod confirmation;
mod input_keys;
mod mouse;
mod session_picker;

pub(crate) use block_nav::handle_block_nav;
pub(crate) use completion::handle_completion_popup;
pub(crate) use confirmation::handle_confirmation;
pub(crate) use input_keys::{handle_input_keys, InputKeyResult};
pub(crate) use mouse::handle_mouse;
pub(crate) use session_picker::handle_session_picker;

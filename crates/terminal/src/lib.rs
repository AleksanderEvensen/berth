//! Embedded terminal for exec sessions: `alacritty_terminal` for the grid,
//! a gpui element to paint it, and keystroke encoding.
//!
//! Engine-agnostic: the owner feeds process output into
//! [`TerminalView::feed`] and forwards [`TerminalEvent`]s to the process.

mod emulator;
mod input;
mod view;

pub use emulator::{Emulator, GridSize};
pub use view::{TerminalEvent, TerminalView};

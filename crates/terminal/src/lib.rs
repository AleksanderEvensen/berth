//! Embedded terminal for exec sessions: `alacritty_terminal` for the grid,
//! a gpui element to paint it, and keystroke encoding.
//!
//! Engine-agnostic: the owner feeds process output into
//! [`TerminalView::feed`] and forwards [`TerminalEvent`]s to the process.

mod emulator;
mod input;
mod view;

/// The ANSI parser the emulator uses, for other views that parse escape
/// sequences (log lines) without pinning a second `vte` version.
pub use alacritty_terminal::vte;
pub use emulator::{Emulator, GridSize, sgr_color};
pub use view::{TerminalEvent, TerminalView};

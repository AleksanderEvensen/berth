//! Window commands, as the platform's own titlebar performs them.
//!
//! gpui's window methods cover most of this, but not uniformly: its
//! Windows backend can't start a move or open the window menu, and its
//! zoom there only maximizes. These fill the gaps, so a custom titlebar
//! can call the same function everywhere.

use gpui::{Pixels, Point, Window};

/// Minimize the window.
pub fn minimize(window: &Window) {
    window.minimize_window();
}

/// Maximize the window, or restore it if it is maximized.
pub fn toggle_maximize(window: &Window) {
    #[cfg(windows)]
    crate::win32::toggle_maximize(window);
    #[cfg(not(windows))]
    window.zoom_window();
}

/// Close the window.
pub fn close(window: &mut Window) {
    window.remove_window();
}

/// Start moving the window with the pointer, as when dragging a titlebar.
/// Call from a mouse-move with the button held, not from the press, so a
/// double-click still arrives (see [`crate::drag_region`]).
pub fn start_move(window: &Window) {
    #[cfg(windows)]
    crate::win32::start_move(window);
    #[cfg(not(windows))]
    window.start_window_move();
}

/// Open the window menu (move, resize, always on top, …) at `position`, in
/// window coordinates. On Windows it opens at the pointer, as the system's
/// own titlebar does.
pub fn show_window_menu(window: &Window, position: Point<Pixels>) {
    #[cfg(windows)]
    {
        let _ = position;
        crate::win32::show_window_menu(window);
    }
    #[cfg(not(windows))]
    window.show_window_menu(position);
}

/// What a double-click on the titlebar does: on macOS the system setting
/// (zoom or minimize), elsewhere maximize or restore.
pub fn titlebar_double_click(window: &Window) {
    #[cfg(target_os = "macos")]
    window.titlebar_double_click();
    #[cfg(not(target_os = "macos"))]
    toggle_maximize(window);
}

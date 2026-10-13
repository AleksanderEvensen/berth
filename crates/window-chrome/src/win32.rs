//! The window commands gpui's Windows backend lacks: it has no window move
//! or window menu, and its zoom only maximizes. Each command is posted to
//! the window procedure, so the modal loop it may start (moving, the menu)
//! runs after gpui's event dispatch has returned rather than inside it.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, HTCAPTION, PostMessageW, SC_MAXIMIZE, SC_MOVE, SC_RESTORE, WM_SYSCOMMAND,
};

/// Opens the window menu at the screen point in `lparam`. Undocumented,
/// but what `DefWindowProc` itself uses for a right-click on the caption.
const WM_POPUPSYSTEMMENU: u32 = 0x0313;

fn hwnd(window: &Window) -> Option<HWND> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    Some(HWND(std::ptr::with_exposed_provenance_mut(
        handle.hwnd.get().cast_unsigned(),
    )))
}

fn post(window: &Window, message: u32, wparam: u32, lparam: isize) {
    let (Some(hwnd), Ok(wparam)) = (hwnd(window), usize::try_from(wparam)) else {
        return;
    };
    // SAFETY: `hwnd` is the live window `window` wraps; posting only
    // queues the message for its window procedure.
    if let Err(error) = unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam)) }
    {
        log::warn!("posting window message {message:#x} failed: {error}");
    }
}

/// Hand the drag to the system move loop, which also snaps.
pub fn start_move(window: &Window) {
    // gpui captures the mouse on press; the move loop needs it released.
    // SAFETY: plain Win32 call on the UI thread.
    unsafe { ReleaseCapture() }.ok();
    post(window, WM_SYSCOMMAND, SC_MOVE | HTCAPTION, 0);
}

pub fn toggle_maximize(window: &Window) {
    let command = if window.is_maximized() {
        SC_RESTORE
    } else {
        SC_MAXIMIZE
    };
    post(window, WM_SYSCOMMAND, command, 0);
}

/// The window menu (restore, move, size, …) at the cursor.
pub fn show_window_menu(window: &Window) {
    let mut cursor = POINT::default();
    // SAFETY: `cursor` is a valid out-pointer for the call.
    if unsafe { GetCursorPos(&raw mut cursor) }.is_err() {
        return;
    }
    // MAKELPARAM: x in the low word, y in the high word.
    let [x0, x1, ..] = cursor.x.to_le_bytes();
    let [y0, y1, ..] = cursor.y.to_le_bytes();
    let Ok(lparam) = isize::try_from(u32::from_le_bytes([x0, x1, y0, y1])) else {
        return;
    };
    post(window, WM_POPUPSYSTEMMENU, 0, lparam);
}

//! The native traffic lights, which `AppKit` draws for us.

use gpui::{Pixels, Window, px};

use crate::caption::TRAFFIC_LIGHT;

/// Fallback right edge of the zoom button, should `AppKit` not hand us the
/// button: three 14pt buttons, 9pt apart (macOS 26+).
const LIGHTS_END: f32 = 14.0 + 3.0 * TRAFFIC_LIGHT + 2.0 * 9.0;

/// Right edge of the lights, or `None` while they are hidden (fullscreen).
pub fn lights_end(window: &Window) -> Option<Pixels> {
    if window.is_fullscreen() {
        return None;
    }
    Some(px(zoom_button_end(window).unwrap_or(LIGHTS_END)))
}

/// Right edge of the native zoom button in window coordinates, measured
/// from `AppKit` so button size and spacing are right on every macOS release.
fn zoom_button_end(window: &Window) -> Option<f32> {
    use num_traits::ToPrimitive as _;
    use objc2_app_kit::{NSView, NSWindowButton};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's AppKit handle points at the window's live content
    // `NSView`, borrowed for as long as `window`; we render on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let zoom = view
        .window()?
        .standardWindowButton(NSWindowButton::ZoomButton)?;
    let frame = zoom.convertRect_toView(zoom.bounds(), None);
    (frame.origin.x + frame.size.width).to_f32()
}

//! Building blocks for gpui apps that draw their own titlebar and window
//! frame, with the platform differences handled.
//!
//! Nothing here decides how your chrome looks. Each piece answers one
//! question or does one job, the same way on every platform, and you
//! compose them in your own elements:
//!
//! - [`actions`]: minimize, maximize or restore, close, move and the
//!   window menu, as the platform's own titlebar performs them.
//! - [`drag_region`]: make any element a titlebar — drag to move,
//!   double-click, window menu.
//! - [`caption_layout`]: which caption buttons to draw on which side,
//!   following the desktop (on Linux its button layout and the
//!   compositor's capabilities), with [`CaptionButton`] for each one's
//!   label, action and Windows hit-test area and glyph.
//! - [`NativeButtons`]: the desktop theme's own drawings of the buttons,
//!   where there is such a theme (GTK on Linux), in every state.
//! - [`traffic_lights_end`] and [`traffic_light_position`]: fitting the
//!   macOS traffic lights into your titlebar.
//! - [`resize`]: hit-testing for a client-side frame's resize band, and
//!   [`ResizeHandles`] to wire it up.
//!
//! A titlebar's caption buttons might look like this:
//!
//! ```ignore
//! let layout = caption_layout(window, cx, ButtonPolicy::Auto);
//! let native = NativeButtons::get();
//! let row = native.and_then(|native| native.row(Side::Right, &layout.right));
//! div().flex().children(layout.right.iter().map(|&button| {
//!     let face = row
//!         .as_ref()
//!         .and_then(|row| row.button(button, "caption", window, cx))
//!         .unwrap_or_else(|| my_own_button_face(button));
//!     face.id(button.label())
//!         .group("caption")
//!         .window_control_area(button.control_area())
//!         .on_click(move |_, window, _| button.perform(window))
//! }))
//! ```

pub mod actions;
mod caption;
mod drag;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux {
    pub mod gtk;
}
#[cfg(target_os = "macos")]
mod macos;
mod native;
pub mod resize;
#[cfg(windows)]
mod win32;

pub use caption::{
    ButtonPolicy, CaptionButton, CaptionLayout, caption_font, caption_layout,
    traffic_light_position, traffic_lights_end,
};
pub use drag::drag_region;
pub use native::{ButtonState, NativeButtons, NativeImage, NativePixels, NativeRow, Side};
pub use resize::ResizeHandles;

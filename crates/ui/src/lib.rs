//! Berth design system: theme tokens, bundled assets and styled components.
//!
//! Behaviour-heavy primitives (inputs, popovers, virtual lists) come from
//! `gpui-base`; everything visible is styled here from [`Theme`] tokens.

mod assets;
pub mod components;
pub mod theme;

pub use assets::{Assets, icons, register_fonts};
pub use components::*;
pub use theme::Theme;

use gpui::App;

/// Install the theme global and register bundled fonts. Call once at startup,
/// before opening windows.
pub fn init(cx: &mut App) {
    cx.set_global(Theme::dark());
    if let Err(err) = register_fonts(cx) {
        eprintln!("berth-ui: failed to register bundled fonts: {err:#}");
    }
}

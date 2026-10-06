//! Berth design system: theme tokens, bundled assets and styled components.
//!
//! Behaviour-heavy primitives (inputs, popovers, virtual lists) come from
//! `gpui-base`; everything visible is styled here from [`Theme`] tokens.

pub mod ansi;
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
    let theme = Theme::dark();
    sync_base_theme(&theme, cx);
    cx.set_global(theme);
    if let Err(err) = register_fonts(cx) {
        eprintln!("berth-ui: failed to register bundled fonts: {err:#}");
    }
}

/// Project our tokens onto `gpui-base`'s semantic tokens so its behaviour
/// components (text input caret, selection, placeholder) paint in our palette.
fn sync_base_theme(theme: &Theme, cx: &mut App) {
    let base = gpui_base::Theme::global_mut(cx);
    base.appearance = match theme.appearance {
        theme::Appearance::Dark => gpui_base::ThemeAppearance::Dark,
        theme::Appearance::Light => gpui_base::ThemeAppearance::Light,
    };
    let c = &mut base.tokens.colors;
    c.background = theme.bg;
    c.foreground = theme.text;
    c.surface = theme.surface;
    c.surface_foreground = theme.text;
    c.primary = theme.accent;
    c.primary_foreground = theme.on_solid;
    c.secondary = theme.surface_raised;
    c.secondary_foreground = theme.text;
    c.muted = theme.row_hover;
    c.muted_foreground = theme.text_faint;
    c.accent = theme.element_hover;
    c.accent_foreground = theme.text;
    c.destructive = theme.danger_strong;
    c.destructive_foreground = gpui::white();
    c.border = theme.border;
    c.input = theme.border;
    c.ring = theme.accent;
    c.selection = theme.accent.opacity(0.35);
}

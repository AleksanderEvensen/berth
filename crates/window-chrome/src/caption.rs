//! Which caption buttons a window shows, where, and what they do.

use gpui::{App, Pixels, Point, SharedString, Window, WindowControlArea, point, px};

use crate::actions;

/// A caption (titlebar) button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    /// Maximize while the window is maximized.
    Restore,
    Close,
}

impl CaptionButton {
    /// The button's accessible name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize",
            Self::Maximize => "Maximize",
            Self::Restore => "Restore",
            Self::Close => "Close",
        }
    }

    /// Do what the button does.
    pub fn perform(self, window: &mut Window) {
        match self {
            Self::Minimize => actions::minimize(window),
            Self::Maximize | Self::Restore => actions::toggle_maximize(window),
            Self::Close => actions::close(window),
        }
    }

    /// The system hit-test area for this button, for gpui's
    /// `window_control_area`. On Windows it makes the button the native
    /// caption button, so hovering maximize opens Snap Layouts; elsewhere
    /// it has no effect.
    #[must_use]
    pub const fn control_area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    /// The glyph Windows draws this button with, in [`caption_font`]. Drawn
    /// at 10px it matches Windows 11's own caption buttons.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Minimize => "\u{E921}",
            Self::Maximize => "\u{E922}",
            Self::Restore => "\u{E923}",
            Self::Close => "\u{E8BB}",
        }
    }
}

/// Which caption buttons to show where the app draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonPolicy {
    /// The platform's buttons, except that a window a tiling window
    /// manager has tiled on every edge keeps only close.
    #[default]
    Auto,
    /// The platform's buttons, always.
    All,
    /// Only close.
    CloseOnly,
    /// None; the window is closed some other way.
    Hidden,
}

/// The caption buttons the app should draw on each side of its titlebar,
/// each in order from the left. Empty where the system draws its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptionLayout {
    pub left: Vec<CaptionButton>,
    pub right: Vec<CaptionButton>,
}

impl CaptionLayout {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.left.is_empty() && self.right.is_empty()
    }
}

/// The caption buttons to draw, per `policy`:
///
/// - **macOS**: none; `AppKit` draws the traffic lights (see
///   [`traffic_lights_end`]).
/// - **Windows**: minimize, maximize or restore, and close on the right,
///   unless fullscreen.
/// - **Linux and FreeBSD**: none under server-side decorations. Otherwise
///   the desktop's button layout (GNOME's `button-layout`, which KDE and
///   others also set), on either side, limited to what the compositor
///   supports (Wayland's `wm_capabilities`).
#[must_use]
pub fn caption_layout(window: &Window, cx: &App, policy: ButtonPolicy) -> CaptionLayout {
    let (left, right) = platform_layout(window, cx);
    if policy == ButtonPolicy::Hidden || (left.is_empty() && right.is_empty()) {
        return CaptionLayout::default();
    }
    let close_only = match policy {
        ButtonPolicy::CloseOnly => true,
        ButtonPolicy::Auto => fully_tiled(window),
        ButtonPolicy::All | ButtonPolicy::Hidden => false,
    };
    let maximized = window.is_maximized();
    let side = |buttons: Vec<CaptionButton>| -> Vec<CaptionButton> {
        buttons
            .into_iter()
            .filter(|button| !close_only || *button == CaptionButton::Close)
            .map(|button| match button {
                CaptionButton::Maximize if maximized => CaptionButton::Restore,
                button => button,
            })
            .collect()
    };
    let mut layout = CaptionLayout {
        left: side(left),
        right: side(right),
    };
    // Asked for close: show it even if the platform's layout has none.
    if policy == ButtonPolicy::CloseOnly && layout.is_empty() {
        layout.right.push(CaptionButton::Close);
    }
    layout
}

/// Whether a tiling window manager has tiled the window on every edge. A
/// maximized window can report every edge tiled too, and isn't counted.
fn fully_tiled(window: &Window) -> bool {
    crate::resize::client_tiling(window).is_some_and(|tiling| {
        tiling.top && tiling.bottom && tiling.left && tiling.right && !window.is_maximized()
    })
}

#[cfg(target_os = "macos")]
fn platform_layout(_window: &Window, _cx: &App) -> (Vec<CaptionButton>, Vec<CaptionButton>) {
    (Vec::new(), Vec::new())
}

#[cfg(windows)]
fn platform_layout(window: &Window, _cx: &App) -> (Vec<CaptionButton>, Vec<CaptionButton>) {
    if window.is_fullscreen() {
        return (Vec::new(), Vec::new());
    }
    (
        Vec::new(),
        vec![
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Close,
        ],
    )
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn platform_layout(window: &Window, cx: &App) -> (Vec<CaptionButton>, Vec<CaptionButton>) {
    use gpui::{WindowButton, WindowButtonLayout};

    if crate::resize::client_tiling(window).is_none() {
        return (Vec::new(), Vec::new());
    }
    let layout = cx
        .button_layout()
        .unwrap_or_else(WindowButtonLayout::linux_default);
    let controls = window.window_controls();
    let side = |buttons: &[Option<WindowButton>]| -> Vec<CaptionButton> {
        buttons
            .iter()
            .flatten()
            .filter_map(|button| match button {
                WindowButton::Minimize => controls.minimize.then_some(CaptionButton::Minimize),
                WindowButton::Maximize => controls.maximize.then_some(CaptionButton::Maximize),
                WindowButton::Close => Some(CaptionButton::Close),
            })
            .collect()
    };
    (side(&layout.left), side(&layout.right))
}

/// The font Windows draws its caption glyphs with (see
/// [`CaptionButton::glyph`]): Segoe Fluent Icons on Windows 11, Segoe MDL2
/// Assets, with the same code points, on Windows 10. `None` elsewhere.
#[must_use]
#[allow(clippy::missing_const_for_fn, reason = "not const on Windows")]
pub fn caption_font(cx: &App) -> Option<SharedString> {
    #[cfg(windows)]
    {
        const FLUENT: &str = "Segoe Fluent Icons";
        thread_local! {
            static FONT: std::cell::OnceCell<SharedString> = const { std::cell::OnceCell::new() };
        }
        Some(FONT.with(|font| {
            font.get_or_init(|| {
                let fluent = cx
                    .text_system()
                    .all_font_names()
                    .iter()
                    .any(|name| name == FLUENT);
                if fluent { FLUENT } else { "Segoe MDL2 Assets" }.into()
            })
            .clone()
        }))
    }
    #[cfg(not(windows))]
    {
        let _ = cx;
        None
    }
}

/// Size of a macOS traffic light.
pub const TRAFFIC_LIGHT: f32 = 14.0;

/// Where macOS should put its close button so the traffic lights centre
/// vertically in a titlebar of `titlebar_height`; for gpui's
/// `TitlebarOptions::traffic_light_position`. Ignored elsewhere.
#[must_use]
pub fn traffic_light_position(titlebar_height: Pixels) -> Point<Pixels> {
    point(
        px(14.0),
        px((f32::from(titlebar_height) - TRAFFIC_LIGHT) / 2.0),
    )
}

/// Right edge of the macOS traffic lights in window coordinates, measured
/// from `AppKit`; `None` while they are hidden (fullscreen) and on other
/// platforms.
#[must_use]
#[allow(clippy::missing_const_for_fn, reason = "not const on macOS")]
pub fn traffic_lights_end(window: &Window) -> Option<Pixels> {
    #[cfg(target_os = "macos")]
    return crate::macos::lights_end(window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}

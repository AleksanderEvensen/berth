//! Styled components. Each is a small `RenderOnce` builder over raw gpui
//! elements, painted exclusively from [`crate::Theme`] tokens.

mod button;
mod card;
mod chart;
mod feedback;
mod field;
mod list;
mod settings;
mod sidebar;
mod tabs;

pub use button::*;
pub use card::*;
pub use chart::*;
pub use feedback::*;
pub use field::*;
pub use list::*;
pub use settings::*;
pub use sidebar::*;
pub use tabs::*;

use gpui::{App, ClickEvent, ElementId, FocusHandle, Svg, Window, svg};

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A focus handle that lives as long as the element keyed by `id` keeps
/// rendering, registered as a tab stop. gpui turns Enter/Space on a focused
/// element into a click, so a focused control with `on_click` is keyboard
/// operable.
pub(crate) fn tab_stop(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(id, cx, |_, cx| cx.focus_handle().tab_stop(true))
        .read(cx)
        .clone()
}

/// An icon element for an embedded asset path (see [`crate::icons`]). Size and
/// colour are set by the caller.
#[must_use]
pub fn icon(path: &'static str) -> Svg {
    use gpui::Styled as _;
    svg().path(path).flex_none()
}

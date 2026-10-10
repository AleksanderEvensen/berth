//! Styled components. Each is a small `RenderOnce` builder over raw gpui
//! elements, painted exclusively from [`crate::Theme`] tokens.

mod button;
mod card;
mod chart;
mod feedback;
mod field;
mod list;
mod scroll;
mod settings;
mod sidebar;
mod tabs;

pub use button::*;
pub use card::*;
pub use chart::*;
pub use feedback::*;
pub use field::*;
pub use list::*;
pub use scroll::*;
pub use settings::*;
pub use sidebar::*;
pub use tabs::*;

use gpui::{
    App, ClickEvent, Div, ElementId, FocusHandle, IntoElement, RenderOnce, Stateful, Svg, Window,
    svg,
};

use crate::Theme;

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A tab-stop focus handle that lives while the element keyed by `id` renders.
///
/// gpui turns Enter/Space on a focused element into a click, so a focused
/// control with `on_click` is keyboard operable.
pub fn tab_stop(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(id, cx, |_, cx| cx.focus_handle().tab_stop(true))
        .read(cx)
        .clone()
}

/// Makes a clickable element built from a div a tab stop.
///
/// The accent ring colours its border while it has keyboard focus. The
/// element brings its own border (transparent if it has none) so the ring
/// doesn't shift layout.
#[derive(IntoElement)]
#[must_use]
pub struct TabStop {
    id: ElementId,
    element: Stateful<Div>,
}

impl TabStop {
    /// `id` keys the focus handle; unique among siblings.
    pub fn new(id: impl Into<ElementId>, element: Stateful<Div>) -> Self {
        Self {
            id: id.into(),
            element,
        }
    }
}

impl RenderOnce for TabStop {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui::{InteractiveElement as _, Styled as _};
        let focus = tab_stop(self.id, window, cx);
        let ring = Theme::of(cx).accent;
        reveal_when_focused(self.element, &focus, window)
            .track_focus(&focus)
            .focus_visible(move |s| s.border_color(ring))
    }
}

/// An icon element for an embedded asset path (see [`crate::icons`]). Size and
/// colour are set by the caller.
#[must_use]
pub fn icon(path: &'static str) -> Svg {
    use gpui::Styled as _;
    svg().path(path).flex_none()
}

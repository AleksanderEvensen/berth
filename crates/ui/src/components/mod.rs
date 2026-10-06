//! Styled components. Each is a small `RenderOnce` builder over raw gpui
//! elements, painted exclusively from [`crate::Theme`] tokens.

mod button;
mod card;
mod feedback;
mod field;
mod list;
mod sidebar;
mod tabs;

pub use button::*;
pub use card::*;
pub use feedback::*;
pub use field::*;
pub use list::*;
pub use sidebar::*;
pub use tabs::*;

use gpui::{App, ClickEvent, Svg, Window, svg};

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// An icon element for an embedded asset path (see [`crate::icons`]). Size and
/// colour are set by the caller.
pub fn icon(path: &'static str) -> Svg {
    use gpui::Styled as _;
    svg().path(path).flex_none()
}

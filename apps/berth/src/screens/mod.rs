pub mod cleanup;
pub mod containers;
pub mod images;
pub mod volumes;

use berth_ui::{ListHeader, Theme};
use gpui::{AnyElement, Div, IntoElement, div, prelude::*, px};

use crate::chrome;

/// Layout facts the shell hands each screen: clearance for the floating
/// sidebar toggle and the window's outer corner radii.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Chrome {
    pub header_inset: f32,
    pub left_radius: f32,
    pub right_radius: f32,
}

/// The left list column: header (also a titlebar drag strip) over `body`.
pub fn list_column(
    theme: &Theme,
    chrome: Chrome,
    drag_id: &'static str,
    header: ListHeader,
    body: AnyElement,
) -> Div {
    div()
        .w(px(Theme::LIST_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .bg(theme.surface)
        .rounded_tl(px(chrome.left_radius))
        .rounded_bl(px(chrome.left_radius))
        .border_r_1()
        .border_color(theme.border)
        .child(chrome::drag_region(
            drag_id,
            div().child(header.leading_inset(chrome.header_inset)),
        ))
        .child(body)
}

/// The right detail plane: a header strip (drag region, optional centred
/// content such as tabs) over `body`.
pub fn detail_pane(
    theme: &Theme,
    chrome: Chrome,
    drag_id: &'static str,
    top: Option<AnyElement>,
    body: impl IntoElement,
) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(theme.bg)
        .rounded_tr(px(chrome.right_radius))
        .rounded_br(px(chrome.right_radius))
        .child(chrome::drag_region(
            drag_id,
            div()
                .h(px(Theme::HEADER_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .px(px(120.0))
                .children(top),
        ))
        .child(div().flex_1().min_h_0().child(body))
}

/// Scrollable, centred reading column for detail content.
pub fn scroll_page(id: &'static str, content: impl IntoElement) -> impl IntoElement {
    div().id(id).size_full().overflow_y_scroll().child(
        div()
            .w_full()
            .max_w(px(780.0))
            .mx_auto()
            .px(px(Theme::SPACE_LG + 8.0))
            .pt(px(Theme::SPACE_SM))
            .pb(px(Theme::SPACE_LG * 2.0))
            .child(content),
    )
}

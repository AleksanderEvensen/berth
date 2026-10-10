pub mod activity;
pub mod cleanup;
pub mod containers;
pub mod engine_help;
pub mod images;
pub mod networks;
pub mod settings;
pub mod volumes;

use berth_docker::Container;
use berth_ui::{ListHeader, Status, Theme, icon, icons, status_dot};
use gpui::{
    AnyElement, Context, Div, EventEmitter, IntoElement, SharedString, div, prelude::*, px,
};

use crate::chrome;
use crate::store::Connection;

/// Status, title and detail line describing the engine connection.
pub fn engine_summary(connection: &Connection) -> (Status, SharedString, SharedString) {
    match connection {
        Connection::Connecting => (
            Status::Transitioning,
            "Connecting…".into(),
            "Looking for an engine".into(),
        ),
        Connection::Connected(info) => (
            Status::Running,
            format!("{} {}", info.name, info.version).into(),
            info.endpoint.to_string().into(),
        ),
        Connection::Disconnected { error } => {
            let detail = engine_help::Problem::of(error)
                .map_or_else(|| error.to_string(), |p| p.short().to_string());
            (Status::Error, "Not connected".into(), detail.into())
        }
    }
}

/// Asks the shell to open a container on the Containers screen (the
/// clickable "Used by" rows of volume and image info).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowContainer(pub String);

/// Asks the shell to open a volume on the Volumes screen (named-volume
/// mounts in container info).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowVolume(pub String);

/// A card listing `users`; clicking a row emits [`ShowContainer`].
pub fn used_by<V: EventEmitter<ShowContainer>>(
    users: &[&Container],
    theme: &Theme,
    cx: &Context<V>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .rounded(px(Theme::PANEL_RADIUS))
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .map(|d| {
            if users.is_empty() {
                d.child(
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .text_color(theme.text_faint)
                        .child("Not used by any container"),
                )
            } else {
                d.children(users.iter().enumerate().map(|(ix, c)| {
                    let id = c.id.clone();
                    div()
                        .id(SharedString::from(format!("used-by-{id}")))
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_SM + 2.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.row_hover))
                        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                        .child(status_dot(
                            if c.state.is_active() {
                                Status::Running
                            } else {
                                Status::Stopped
                            },
                            7.0,
                            None,
                            theme,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(c.name.clone()),
                        )
                        .child(
                            icon(icons::CHEVRON_RIGHT)
                                .size(px(14.0))
                                .text_color(theme.text_faint),
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ShowContainer(id.clone()));
                        }))
                }))
            }
        })
}

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

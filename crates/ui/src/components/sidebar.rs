use gpui::{
    AnimationExt as _, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, IntoElement, RenderOnce,
    SharedString, SpringAnimation, SpringConfig, Window, div, prelude::*, px,
};

use super::{ClickHandler, icon};
use crate::Theme;

/// Critically damped spring for panel widths: no bounce, settles within the
/// 200 ms motion budget (NFR-U4).
pub const PANEL_SPRING: SpringConfig = SpringConfig::new(900.0, 60.0, 1.0);

/// The app's primary navigation column.
///
/// Collapsing springs the column's width to zero. The content keeps its full
/// width while the column narrows (it is clipped, not reflowed), so labels
/// never wrap mid-animation.
#[derive(IntoElement)]
pub struct Sidebar {
    id: ElementId,
    collapsed: bool,
    width: f32,
    corner_radius: f32,
    header: Option<AnyElement>,
    sections: Vec<SidebarSection>,
    footer: Option<AnyElement>,
}

impl Sidebar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            collapsed: false,
            width: Theme::SIDEBAR_WIDTH,
            corner_radius: 0.0,
            header: None,
            sections: Vec::new(),
            footer: None,
        }
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Radius for the outer (left) corners when the sidebar sits at the edge
    /// of a rounded client-decorated window.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// Top strip — typically the titlebar drag region (height is the caller's).
    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn section(mut self, section: SidebarSection) -> Self {
        self.sections.push(section);
        self
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
}

impl RenderOnce for Sidebar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let target = if self.collapsed { 0.0 } else { self.width };

        div()
            .id(self.id.clone())
            .flex_none()
            .h_full()
            .overflow_hidden()
            .child(
                div()
                    .w(px(self.width))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.surface)
                    .rounded_tl(px(self.corner_radius))
                    .rounded_bl(px(self.corner_radius))
                    .border_r_1()
                    .border_color(theme.border)
                    .children(self.header)
                    .child(
                        div()
                            .id("sidebar-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(px(Theme::SPACE_SM))
                            .pb(px(Theme::SPACE_SM))
                            .flex()
                            .flex_col()
                            .gap(px(Theme::SPACE_MD))
                            .children(self.sections),
                    )
                    .when_some(self.footer, |d, footer| {
                        d.child(
                            div()
                                .flex_none()
                                .border_t_1()
                                .border_color(theme.border)
                                .px(px(Theme::SPACE_SM))
                                .py(px(Theme::SPACE_SM))
                                .child(footer),
                        )
                    }),
            )
            .with_spring(
                self.id,
                SpringAnimation::new(PANEL_SPRING)
                    .to(px(target))
                    .with_epsilon(0.5),
                |this, w| this.w(w),
            )
    }
}

/// A titled group of sidebar items.
#[derive(IntoElement)]
pub struct SidebarSection {
    title: Option<SharedString>,
    items: Vec<SidebarItem>,
}

impl SidebarSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            items: Vec::new(),
        }
    }

    pub fn untitled() -> Self {
        Self {
            title: None,
            items: Vec::new(),
        }
    }

    pub fn item(mut self, item: SidebarItem) -> Self {
        self.items.push(item);
        self
    }
}

impl RenderOnce for SidebarSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .when_some(self.title, |d, title| {
                d.child(
                    div()
                        .px(px(Theme::SPACE_SM))
                        .pb(px(Theme::SPACE_XS))
                        .text_size(px(Theme::TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_faint)
                        .child(title),
                )
            })
            .children(self.items)
    }
}

/// One navigation entry: icon, label, optional trailing count.
#[derive(IntoElement)]
pub struct SidebarItem {
    id: ElementId,
    icon: &'static str,
    label: SharedString,
    trailing: Option<SharedString>,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl SidebarItem {
    pub fn new(id: impl Into<ElementId>, icon: &'static str, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            trailing: None,
            active: false,
            on_click: None,
        }
    }

    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for SidebarItem {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let row_hover = theme.row_hover;
        let on_click = self.on_click;
        div()
            .id(self.id)
            .h(px(Theme::SIDEBAR_ROW_HEIGHT))
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .px(px(Theme::SPACE_SM))
            .rounded(px(Theme::CONTROL_RADIUS + 1.0))
            .cursor_pointer()
            .text_size(px(Theme::TEXT_MD + 0.5))
            .map(|d| {
                if self.active {
                    d.bg(theme.element_hover)
                        .text_color(theme.text)
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    d.text_color(theme.text_muted)
                        .hover(move |s| s.bg(row_hover))
                }
            })
            .child(
                icon(self.icon)
                    .size(px(16.0))
                    .text_color(if self.active {
                        theme.text
                    } else {
                        theme.text_muted
                    }),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.label))
            .when_some(self.trailing, |d, t| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(Theme::TEXT_XS))
                        .text_color(theme.text_faint)
                        .child(t),
                )
            })
            .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
    }
}

/// The sidebar toggle glyph: a rounded frame holding a panel that narrows when
/// the sidebar is closed. Drawn from quads in a 24-unit space so it can sit in
/// a spring (`open` 0..=1).
pub fn sidebar_glyph(open: f32, size: f32, color: Hsla) -> Div {
    let s = size / 24.0;
    let stroke = 1.6;
    let open = open.clamp(0.0, 1.0);
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(
            div()
                .absolute()
                .left(px((3.0 - stroke / 2.0) * s))
                .top(px((4.0 - stroke / 2.0) * s))
                .w(px((18.0 + stroke) * s))
                .h(px((16.0 + stroke) * s))
                .rounded(px((4.0 + stroke / 2.0) * s))
                .border(px(stroke * s))
                .border_color(color),
        )
        .child(
            div()
                .absolute()
                .left(px(6.5 * s))
                .top(px(7.5 * s))
                .w(px((1.75 + (5.5 - 1.75) * open) * s))
                .h(px(9.0 * s))
                .rounded(px(0.9 * s))
                .bg(color),
        )
}

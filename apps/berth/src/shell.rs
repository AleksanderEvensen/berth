//! The window root: sidebar + active screen + chrome.

use berth_ui::{
    IconButton, PANEL_SPRING, Sidebar, SidebarItem, SidebarSection, Status, Theme, icons, sidebar_glyph,
    status_dot,
};
use gpui::{
    AnimationExt as _, Context, Entity, FocusHandle, FontWeight, IntoElement, Render,
    SpringAnimation, Window, actions, div, prelude::*, px,
};

use crate::{chrome, screens::containers::ContainersScreen};

actions!(berth, [ToggleSidebar, Quit]);

/// Space the floating sidebar toggle occupies in the list header when the
/// sidebar is collapsed.
const TOGGLE_CLEARANCE: f32 = 34.0;

pub struct Shell {
    focus: FocusHandle,
    sidebar_collapsed: bool,
    containers: Entity<ContainersScreen>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            focus,
            sidebar_collapsed: false,
            containers: cx.new(ContainersScreen::new),
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn render_sidebar(&self, radius: f32, cx: &mut Context<Self>) -> Sidebar {
        let theme = Theme::of(cx);
        let running = self.containers.read(cx).running_count();

        let footer = div()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .px(px(Theme::SPACE_SM))
            .h(px(36.0))
            .child(status_dot(Status::Running, 8.0, None, theme))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_MD))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Docker Engine"),
                    )
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_XS))
                            .text_color(theme.text_faint)
                            .truncate()
                            .child("Fixture data · not connected"),
                    ),
            )
            .child(IconButton::new("settings", icons::SETTINGS).size(28.0, 16.0));

        Sidebar::new("sidebar")
            .collapsed(self.sidebar_collapsed)
            .corner_radius(radius)
            .header(chrome::drag_region(
                "sidebar-titlebar",
                div().h(px(Theme::HEADER_HEIGHT)).flex_none().w_full(),
            ))
            .section(
                SidebarSection::new("Docker")
                    .item(
                        SidebarItem::new("nav-containers", icons::BOX, "Containers")
                            .trailing(running.to_string())
                            .active(true),
                    )
                    .item(SidebarItem::new("nav-volumes", icons::HARD_DRIVE, "Volumes"))
                    .item(SidebarItem::new("nav-images", icons::DISC, "Images"))
                    .item(SidebarItem::new("nav-networks", icons::NETWORK, "Networks")),
            )
            .section(
                SidebarSection::new("General")
                    .item(SidebarItem::new("nav-activity", icons::CHART, "Activity"))
                    .item(SidebarItem::new("nav-cleanup", icons::BROOM, "Clean up")),
            )
            .footer(footer)
    }

    fn render_sidebar_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let color = theme.text_muted;
        let open = if self.sidebar_collapsed { 0.0 } else { 1.0 };
        div()
            .id("sidebar-toggle")
            .absolute()
            .top(px((Theme::HEADER_HEIGHT - 28.0) / 2.0))
            .left(px(Theme::SPACE_SM + 2.0))
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::CONTROL_RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(theme.element_hover))
            .occlude()
            .child(div().with_spring(
                "sidebar-glyph",
                SpringAnimation::new(PANEL_SPRING).to(open),
                move |this, t: f32| this.child(sidebar_glyph(t, 18.0, color)),
            ))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_sidebar(&ToggleSidebar, window, cx)
            }))
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let radius = chrome::content_radius(window);
        let theme = Theme::of(cx).clone();
        let collapsed = self.sidebar_collapsed;
        self.containers.update(cx, |screen, cx| {
            screen.set_chrome(
                if collapsed { TOGGLE_CLEARANCE } else { 0.0 },
                if collapsed { radius } else { 0.0 },
                radius,
                cx,
            )
        });
        let captions = chrome::caption_buttons(window, cx);

        let content = div()
            .id("shell")
            .track_focus(&self.focus)
            .key_context("Shell")
            .on_action(cx.listener(Self::toggle_sidebar))
            .relative()
            .size_full()
            .flex()
            .bg(theme.bg)
            .font_family(theme.font_sans.clone())
            .text_color(theme.text)
            .text_size(px(Theme::TEXT_MD))
            .child(self.render_sidebar(radius, cx))
            .child(div().flex_1().min_w_0().h_full().child(self.containers.clone()))
            .child(self.render_sidebar_toggle(cx))
            .when_some(captions, |d, captions| {
                d.child(
                    div()
                        .absolute()
                        .top(px((Theme::HEADER_HEIGHT - 24.0) / 2.0))
                        .right(px(Theme::SPACE_MD))
                        .child(captions),
                )
            });

        chrome::window_frame(content.into_any_element(), window, cx)
    }
}

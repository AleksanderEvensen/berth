//! The window root: sidebar + active screen + chrome, plus shell-owned
//! overlays (confirm dialog, toasts).

use std::sync::Arc;
use std::time::Duration;

use berth_docker::Engine;
use berth_ui::{
    Button, ButtonTone, Dialog, IconButton, PANEL_SPRING, Sidebar, SidebarItem, SidebarSection,
    Status, Theme, Toast, ToastKind, icons, item_list, modal_overlay, sidebar_glyph, status_dot,
};
use gpui::{
    AnimationExt as _, AnyElement, Context, Entity, FocusHandle, FontWeight, IntoElement, Render,
    SharedString, SpringAnimation, Window, actions, div, prelude::*, px,
};

use crate::dialogs::{CONFIRM_CONTEXT, Cancel, Confirm, ConfirmRequest};
use crate::store::{Connection, Store, StoreEvent};
use crate::{chrome, screens::containers::ContainersScreen};

actions!(berth, [ToggleSidebar, Quit]);

/// Space the floating sidebar toggle occupies in the list header when the
/// sidebar is collapsed.
const TOGGLE_CLEARANCE: f32 = 34.0;
const TOAST_LIFETIME: Duration = Duration::from_secs(6);
const MAX_TOASTS: usize = 3;

struct ToastItem {
    id: usize,
    title: SharedString,
    detail: SharedString,
}

pub struct Shell {
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    sidebar_collapsed: bool,
    store: Entity<Store>,
    containers: Entity<ContainersScreen>,
    confirm: Option<ConfirmRequest>,
    toasts: Vec<ToastItem>,
    next_toast_id: usize,
}

impl Shell {
    pub fn new(engine: Arc<dyn Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let store = cx.new(|cx| Store::new(engine, cx));
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&store, |this, _, event: &StoreEvent, cx| match event {
            StoreEvent::Error { title, detail } => {
                this.push_toast(title.clone(), detail.clone(), cx)
            }
        })
        .detach();

        let containers = cx.new(|cx| ContainersScreen::new(store.clone(), cx));
        cx.subscribe_in(
            &containers,
            window,
            |this, _, request: &ConfirmRequest, window, cx| {
                this.confirm = Some(request.clone());
                window.focus(&this.dialog_focus, cx);
                cx.notify();
            },
        )
        .detach();

        Self {
            focus,
            dialog_focus: cx.focus_handle(),
            sidebar_collapsed: false,
            store,
            containers,
            confirm: None,
            toasts: Vec::new(),
            next_toast_id: 0,
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    // ---- confirm dialog ----

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(request) = self.confirm.take() {
            (request.on_confirm)(window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn render_confirm(&self, request: &ConfirmRequest, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let mut dialog = Dialog::new(request.title.clone()).message(request.message.clone());
        if !request.items.is_empty() {
            dialog = dialog.body(item_list(&request.items, 6, theme));
        }
        let dialog = dialog
            .button(
                Button::new("confirm-cancel", "Cancel")
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx))),
            )
            .button(
                Button::new("confirm-accept", request.confirm_label.clone())
                    .solid()
                    .tone(ButtonTone::Danger)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.confirm(&Confirm, window, cx)),
                    ),
            );
        modal_overlay(
            div()
                .track_focus(&self.dialog_focus)
                .key_context(CONFIRM_CONTEXT)
                .on_action(cx.listener(Self::confirm))
                .on_action(cx.listener(Self::cancel))
                .child(dialog),
            theme,
        )
        .into_any_element()
    }

    // ---- toasts ----

    fn push_toast(&mut self, title: String, detail: String, cx: &mut Context<Self>) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(ToastItem {
            id,
            title: title.into(),
            detail: detail.into(),
        });
        if self.toasts.len() > MAX_TOASTS {
            self.toasts.remove(0);
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_LIFETIME).await;
            this.update(cx, |this, cx| this.dismiss_toast(id, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn dismiss_toast(&mut self, id: usize, cx: &mut Context<Self>) {
        self.toasts.retain(|t| t.id != id);
        cx.notify();
    }

    fn render_toasts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom(px(Theme::SPACE_LG + 4.0))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(Theme::SPACE_SM))
            .children(self.toasts.iter().map(|toast| {
                let id = toast.id;
                Toast::new(("toast", id), ToastKind::Error, toast.title.clone())
                    .detail(toast.detail.clone())
                    .on_dismiss(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)))
            }))
    }

    // ---- sidebar ----

    fn render_engine_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (status, name, detail): (Status, SharedString, SharedString) =
            match self.store.read(cx).connection() {
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
                    (Status::Error, "Not connected".into(), error.clone().into())
                }
            };
        div()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .px(px(Theme::SPACE_SM))
            .h(px(36.0))
            .child(status_dot(status, 8.0, None, theme))
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
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_XS))
                            .text_color(theme.text_faint)
                            .truncate()
                            .child(detail),
                    ),
            )
            .child(IconButton::new("settings", icons::SETTINGS).size(28.0, 16.0))
    }

    fn render_sidebar(&self, radius: f32, cx: &mut Context<Self>) -> Sidebar {
        let running = self.store.read(cx).running_count();
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
                    .item(SidebarItem::new(
                        "nav-volumes",
                        icons::HARD_DRIVE,
                        "Volumes",
                    ))
                    .item(SidebarItem::new("nav-images", icons::DISC, "Images"))
                    .item(SidebarItem::new("nav-networks", icons::NETWORK, "Networks")),
            )
            .section(
                SidebarSection::new("General")
                    .item(SidebarItem::new("nav-activity", icons::CHART, "Activity"))
                    .item(SidebarItem::new("nav-cleanup", icons::BROOM, "Clean up")),
            )
            .footer(self.render_engine_footer(cx))
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
            .on_click(
                cx.listener(|this, _, window, cx| this.toggle_sidebar(&ToggleSidebar, window, cx)),
            )
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
        let confirm = self
            .confirm
            .clone()
            .map(|request| self.render_confirm(&request, cx));

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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.containers.clone()),
            )
            .child(self.render_sidebar_toggle(cx))
            .when_some(captions, |d, captions| {
                d.child(
                    div()
                        .absolute()
                        .top(px((Theme::HEADER_HEIGHT - 24.0) / 2.0))
                        .right(px(Theme::SPACE_MD))
                        .child(captions),
                )
            })
            .child(self.render_toasts(cx))
            .children(confirm);

        chrome::window_frame(content.into_any_element(), window, cx)
    }
}

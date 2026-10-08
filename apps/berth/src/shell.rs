//! The window root: sidebar + active screen + chrome, plus shell-owned
//! overlays (confirm dialog, toasts).

use std::time::Duration;

use berth_ui::{
    Button, ButtonTone, Dialog, IconButton, PANEL_SPRING, Sidebar, SidebarItem, SidebarSection,
    Theme, Toast, ToastKind, icons, item_list, modal_overlay, sidebar_glyph, status_dot,
};
use gpui::{
    AnimationExt as _, AnyElement, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    IntoElement, Render, SharedString, SpringAnimation, Window, actions, div, prelude::*, px,
};

use crate::chrome;
use crate::dialogs::{CONFIRM_CONTEXT, Cancel, Confirm, ConfirmRequest};
use crate::preferences;
use crate::screens::{
    Chrome,
    activity::ActivityScreen,
    cleanup::CleanupScreen,
    containers::ContainersScreen,
    engine_summary,
    images::ImagesScreen,
    networks::NetworksScreen,
    settings::{Page, SettingsScreen},
    volumes::VolumesScreen,
};
use crate::store::{Connector, Store, StoreEvent};

actions!(
    berth,
    [
        #[derive(Eq)]
        ToggleSidebar,
        #[derive(Eq)]
        FocusSearch,
        #[derive(Eq)]
        OpenSettings,
        #[derive(Eq)]
        FocusNext,
        #[derive(Eq)]
        FocusPrev,
        #[derive(Eq)]
        Quit
    ]
);

/// Space the floating sidebar toggle occupies in the list header when the
/// sidebar is collapsed.
const TOGGLE_CLEARANCE: f32 = 34.0;
const TOAST_LIFETIME: Duration = Duration::from_secs(6);
const MAX_TOASTS: usize = 3;

/// Which screen the sidebar has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Containers,
    Volumes,
    Images,
    Networks,
    Activity,
    Cleanup,
    Settings,
}

struct ToastItem {
    id: usize,
    kind: ToastKind,
    title: SharedString,
    detail: SharedString,
}

pub struct Shell {
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    sidebar_collapsed: bool,
    store: Entity<Store>,
    route: Route,
    /// Where the settings sidebar's "Back" returns to.
    back: Route,
    containers: Entity<ContainersScreen>,
    volumes: Entity<VolumesScreen>,
    images: Entity<ImagesScreen>,
    networks: Entity<NetworksScreen>,
    activity: Entity<ActivityScreen>,
    cleanup: Entity<CleanupScreen>,
    settings: Entity<SettingsScreen>,
    confirm: Option<ConfirmRequest>,
    toasts: Vec<ToastItem>,
    next_toast_id: usize,
    /// The settings problem already toasted, so it is reported once.
    settings_problem: Option<SharedString>,
}

impl Shell {
    pub fn new(connector: Connector, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        preferences::observe_system_appearance(window, cx).detach();

        let store = cx.new(|cx| Store::new(connector, cx));
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&store, |this, _, event: &StoreEvent, cx| match event {
            StoreEvent::Error { title, detail } => {
                this.push_toast(ToastKind::Error, title.clone(), detail.clone(), cx);
            }
            StoreEvent::Info { title, detail } => {
                this.push_toast(ToastKind::Info, title.clone(), detail.clone(), cx);
            }
        })
        .detach();
        cx.observe_global::<preferences::Preferences>(Self::report_settings_problem)
            .detach();
        // Report a problem from loading settings once the shell exists.
        cx.spawn(async move |this, cx| {
            this.update(cx, Self::report_settings_problem).ok();
        })
        .detach();

        let containers = cx.new(|cx| ContainersScreen::new(store.clone(), cx));
        let volumes = cx.new(|cx| VolumesScreen::new(store.clone(), cx));
        let images = cx.new(|cx| ImagesScreen::new(store.clone(), cx));
        let networks = cx.new(|cx| NetworksScreen::new(store.clone(), cx));
        let activity = cx.new(|cx| ActivityScreen::new(store.clone(), cx));
        let cleanup = cx.new(|cx| CleanupScreen::new(store.clone(), cx));
        let settings = cx.new(|cx| SettingsScreen::new(store.clone(), window, cx));
        Self::present_confirms(&containers, window, cx);
        Self::present_confirms(&volumes, window, cx);
        Self::present_confirms(&images, window, cx);
        Self::present_confirms(&networks, window, cx);
        Self::present_confirms(&cleanup, window, cx);

        Self {
            focus,
            dialog_focus: cx.focus_handle(),
            sidebar_collapsed: preferences::get(cx).sidebar_collapsed,
            store,
            route: Route::Containers,
            back: Route::Containers,
            containers,
            volumes,
            images,
            networks,
            activity,
            cleanup,
            settings,
            confirm: None,
            toasts: Vec::new(),
            next_toast_id: 0,
            settings_problem: None,
        }
    }

    fn report_settings_problem(&mut self, cx: &mut Context<Self>) {
        let problem = preferences::problem(cx);
        if problem == self.settings_problem {
            return;
        }
        self.settings_problem.clone_from(&problem);
        if let Some(problem) = problem {
            self.push_toast(
                ToastKind::Error,
                "Settings problem".into(),
                problem.to_string(),
                cx,
            );
        }
    }

    /// Show the confirm dialog whenever `screen` asks for one.
    fn present_confirms<T: EventEmitter<ConfirmRequest>>(
        screen: &Entity<T>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            screen,
            window,
            |this, _, request: &ConfirmRequest, window, cx| {
                this.confirm = Some(request.clone());
                window.focus(&this.dialog_focus, cx);
                cx.notify();
            },
        )
        .detach();
    }

    fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route == self.route {
            return;
        }
        if route == Route::Settings {
            self.back = self.route;
        }
        self.route = route;
        // Focus may sit in the screen being hidden (e.g. the search field);
        // left there, shell shortcuts would stop resolving.
        window.focus(&self.focus, cx);
        self.containers.update(cx, |screen, cx| {
            screen.set_visible(route == Route::Containers, cx);
        });
        self.activity.update(cx, |screen, cx| {
            screen.set_visible(route == Route::Activity, cx);
        });
        cx.notify();
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Containers, window, cx);
        self.containers
            .update(cx, |screen, cx| screen.open_search(window, cx));
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, window, cx);
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        let collapsed = self.sidebar_collapsed;
        preferences::update(cx, |s| s.sidebar_collapsed = collapsed);
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

    fn render_confirm(&self, request: &ConfirmRequest, cx: &Context<Self>) -> AnyElement {
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

    fn push_toast(
        &mut self,
        kind: ToastKind,
        title: String,
        detail: String,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_toast_id;
        // IDs only identify the bounded list of recent toasts.
        self.next_toast_id = self.next_toast_id.wrapping_add(1);
        self.toasts.push(ToastItem {
            id,
            kind,
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

    fn render_toasts(&self, cx: &Context<Self>) -> impl IntoElement {
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
                Toast::new(("toast", id), toast.kind, toast.title.clone())
                    .detail(toast.detail.clone())
                    .on_dismiss(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)))
            }))
    }

    // ---- sidebar ----

    fn render_engine_footer(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (status, name, detail) = engine_summary(self.store.read(cx).connection());
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
            .child(
                IconButton::new("settings", icons::SETTINGS)
                    .size(28.0, 16.0)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(&OpenSettings, window, cx);
                    })),
            )
    }

    /// Settings swaps the navigation for its own pages, with a way back.
    fn render_settings_sidebar(&self, radius: f32, cx: &Context<Self>) -> Sidebar {
        let current = self.settings.read(cx).page();
        let back = self.back;
        Sidebar::new("sidebar")
            .collapsed(self.sidebar_collapsed)
            .corner_radius(radius)
            .header(chrome::drag_region(
                "sidebar-titlebar",
                div().h(px(Theme::HEADER_HEIGHT)).flex_none().w_full(),
            ))
            .section(SidebarSection::untitled().item(
                SidebarItem::new("settings-back", icons::ARROW_LEFT, "Back").on_click(
                    cx.listener(move |this, _, window, cx| this.navigate(back, window, cx)),
                ),
            ))
            .section(Page::ALL.into_iter().fold(
                SidebarSection::new("Settings"),
                |section, page| {
                    section.item(
                        SidebarItem::new(
                            SharedString::from(format!("settings-{}", page.label())),
                            page.icon(),
                            page.label(),
                        )
                        .active(page == current)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings
                                .update(cx, |screen, cx| screen.set_page(page, cx));
                        })),
                    )
                },
            ))
            .footer(self.render_engine_footer(cx))
    }

    fn render_sidebar(&self, radius: f32, cx: &Context<Self>) -> Sidebar {
        if self.route == Route::Settings {
            return self.render_settings_sidebar(radius, cx);
        }
        let store = self.store.read(cx);
        let running = store.running_count();
        let volumes = store.volumes().len();
        let images = store.images().len();
        let networks = store.networks().len();
        let item = |id: &'static str, icon, label: &'static str, route: Route| {
            SidebarItem::new(id, icon, label)
                .active(self.route == route)
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(route, window, cx)))
        };
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
                        item(
                            "nav-containers",
                            icons::BOX,
                            "Containers",
                            Route::Containers,
                        )
                        .trailing(running.to_string()),
                    )
                    .item(
                        item("nav-volumes", icons::HARD_DRIVE, "Volumes", Route::Volumes)
                            .trailing(volumes.to_string()),
                    )
                    .item(
                        item("nav-images", icons::DISC, "Images", Route::Images)
                            .trailing(images.to_string()),
                    )
                    .item(
                        item("nav-networks", icons::NETWORK, "Networks", Route::Networks)
                            .trailing(networks.to_string()),
                    ),
            )
            .section(
                SidebarSection::new("General")
                    .item(item(
                        "nav-activity",
                        icons::CHART,
                        "Activity",
                        Route::Activity,
                    ))
                    .item(item(
                        "nav-cleanup",
                        icons::BROOM,
                        "Clean up",
                        Route::Cleanup,
                    )),
            )
            .footer(self.render_engine_footer(cx))
    }

    fn render_sidebar_toggle(&self, leading: f32, cx: &Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let color = theme.text_muted;
        let open = if self.sidebar_collapsed { 0.0 } else { 1.0 };
        div()
            .id("sidebar-toggle")
            .absolute()
            .top(px((Theme::HEADER_HEIGHT - 28.0) / 2.0))
            .left(px(leading))
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::CONTROL_RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(theme.element_hover))
            .occlude()
            .map(|d| {
                if theme.reduce_motion {
                    d.child(sidebar_glyph(open, 18.0, color))
                } else {
                    d.child(div().with_spring(
                        "sidebar-glyph",
                        SpringAnimation::new(PANEL_SPRING).to(open),
                        move |this, t: f32| this.child(sidebar_glyph(t, 18.0, color)),
                    ))
                }
            })
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
        let leading = chrome::titlebar_leading(window);
        let frame = Chrome {
            header_inset: if collapsed {
                leading - chrome::TITLEBAR_LEADING + TOGGLE_CLEARANCE
            } else {
                0.0
            },
            left_radius: if collapsed { radius } else { 0.0 },
            right_radius: radius,
        };
        let screen = match self.route {
            Route::Containers => {
                self.containers.update(cx, |screen, cx| {
                    screen.set_chrome(frame.header_inset, frame.left_radius, radius, cx);
                });
                self.containers.clone().into_any_element()
            }
            Route::Volumes => {
                self.volumes
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.volumes.clone().into_any_element()
            }
            Route::Images => {
                self.images
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.images.clone().into_any_element()
            }
            Route::Networks => {
                self.networks
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.networks.clone().into_any_element()
            }
            Route::Activity => {
                self.activity
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.activity.clone().into_any_element()
            }
            Route::Cleanup => {
                self.cleanup
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.cleanup.clone().into_any_element()
            }
            Route::Settings => {
                self.settings
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
                self.settings.clone().into_any_element()
            }
        };
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
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::open_settings))
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrev, window, cx| window.focus_prev(cx))
            .relative()
            .size_full()
            .flex()
            .bg(theme.bg)
            .font_family(theme.font_sans.clone())
            .text_color(theme.text)
            .text_size(px(Theme::TEXT_MD))
            .child(self.render_sidebar(radius, cx))
            .child(div().flex_1().min_w_0().h_full().child(screen))
            .child(self.render_sidebar_toggle(leading, cx))
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

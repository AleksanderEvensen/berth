//! The window root: sidebar + active screen + chrome, plus shell-owned
//! overlays (confirm dialog, command palette, toasts).

use std::time::Duration;

use berth_ui::{
    Button, ButtonTone, Dialog, IconButton, PANEL_SPRING, ScrollStep, Sidebar, SidebarItem,
    SidebarSection, TabStop, Theme, Toast, ToastKind, icons, item_list, modal_overlay,
    scroll_active_pane, sidebar_glyph, status_dot,
};
use gpui::{
    AnimationExt as _, AnyElement, Context, Div, Entity, EventEmitter, FocusHandle, FontWeight,
    IntoElement, Render, SharedString, SpringAnimation, Stateful, Window, div, prelude::*, px,
};

use crate::chrome;
use crate::dialogs::{CONFIRM_CONTEXT, Cancel, Confirm, ConfirmRequest};
use crate::keymap::{
    ClearSelection, Collapse, CursorNext, CursorPrev, DeleteSelected, Expand, ExtendFirst,
    ExtendLast, ExtendNext, ExtendPrev, FocusNext, FocusPrev, FocusSearch, GoActivity, GoCleanup,
    GoContainers, GoImages, GoNetworks, GoVolumes, OpenPort, OpenSettings, Restart, ScrollBottom,
    ScrollPageDown, ScrollPageUp, ScrollTop, SelectAll, SelectFirst, SelectLast, SelectNext,
    SelectPrev, ShowFiles, ShowInfo, ShowLogs, ShowShortcuts, ShowStats, ShowTerminal,
    TogglePalette, ToggleRunning, ToggleSelected, ToggleSidebar,
};
use crate::palette::{Palette, PaletteEvent, Target};
use crate::preferences;
use crate::screens::{
    Chrome, ListOp, ListScreen, ShowContainer, ShowEngineHelp, ShowVolume,
    activity::ActivityScreen,
    cleanup::CleanupScreen,
    containers::ContainersScreen,
    engine_help::Problem,
    engine_summary,
    images::ImagesScreen,
    networks::NetworksScreen,
    settings::{Page, SettingsScreen},
    volumes::VolumesScreen,
};
use crate::selection::Step;
use crate::store::{Connector, Store, StoreEvent};

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

/// The open command palette and the focus to give back when it closes.
struct OpenPalette {
    view: Entity<Palette>,
    previous_focus: Option<FocusHandle>,
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
    palette: Option<OpenPalette>,
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
        // A focused control can disappear (a disabled button, a closed
        // terminal); without focus no shortcut resolves, so take it back.
        cx.on_focus_lost(window, |this: &mut Self, window, cx| {
            let target = this.focus_trap(cx).unwrap_or_else(|| this.focus.clone());
            window.focus(&target, cx);
        })
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
        Self::follow_container_links(&volumes, window, cx);
        Self::follow_container_links(&images, window, cx);
        cx.subscribe_in(
            &containers,
            window,
            |this, _, ShowVolume(name): &ShowVolume, window, cx| {
                this.navigate(Route::Volumes, window, cx);
                this.volumes
                    .update(cx, |screen, cx| screen.reveal(name, cx));
            },
        )
        .detach();
        cx.subscribe_in(
            &containers,
            window,
            |this, _, _: &ShowEngineHelp, window, cx| {
                this.show_engine_help(window, cx);
            },
        )
        .detach();

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
            palette: None,
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

    /// Open the Containers screen on whichever container `screen` links to.
    fn follow_container_links<T: EventEmitter<ShowContainer>>(
        screen: &Entity<T>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            screen,
            window,
            |this, _, ShowContainer(id): &ShowContainer, window, cx| {
                this.navigate(Route::Containers, window, cx);
                this.containers
                    .update(cx, |screen, cx| screen.reveal(id, cx));
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
        self.volumes.update(cx, |screen, cx| {
            screen.set_visible(route == Route::Volumes, cx);
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

    /// The engine fix-it guide lives on Settings → Engine.
    fn show_engine_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, window, cx);
        self.settings
            .update(cx, |screen, cx| screen.set_page(Page::Engine, cx));
    }

    fn show_shortcuts(&mut self, _: &ShowShortcuts, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, window, cx);
        self.settings
            .update(cx, |screen, cx| screen.set_page(Page::Shortcuts, cx));
    }

    /// Apply a list key to the current screen's list. Screens without one
    /// scroll instead: the arrows by a line, Home and End to the ends.
    fn list(&self, op: ListOp, window: &mut Window, cx: &mut Context<Self>) {
        fn apply<T: ListScreen>(screen: &Entity<T>, op: ListOp, cx: &mut Context<Shell>) {
            screen.update(cx, |screen, cx| screen.list_op(op, cx));
        }
        match self.route {
            Route::Containers => apply(&self.containers, op, cx),
            Route::Volumes => apply(&self.volumes, op, cx),
            Route::Images => apply(&self.images, op, cx),
            Route::Networks => apply(&self.networks, op, cx),
            Route::Activity | Route::Cleanup | Route::Settings => {
                let scroll = match op {
                    ListOp::Step(Step::Prev, _) | ListOp::Cursor(Step::Prev) => ScrollStep::LineUp,
                    ListOp::Step(Step::Next, _) | ListOp::Cursor(Step::Next) => {
                        ScrollStep::LineDown
                    }
                    ListOp::Step(Step::First, _) | ListOp::Cursor(Step::First) => ScrollStep::Top,
                    ListOp::Step(Step::Last, _) | ListOp::Cursor(Step::Last) => ScrollStep::Bottom,
                    ListOp::SelectAll | ListOp::Clear | ListOp::Toggle | ListOp::Delete => return,
                };
                scroll_active_pane(scroll, window, cx);
            }
        }
    }

    /// Where focus must stay while a modal is open: the confirm dialog or
    /// the palette.
    fn focus_trap(&self, cx: &Context<Self>) -> Option<FocusHandle> {
        if self.confirm.is_some() {
            Some(self.dialog_focus.clone())
        } else {
            self.palette
                .as_ref()
                .map(|palette| palette.view.read(cx).focus_handle(cx))
        }
    }

    /// Tab and Shift+Tab. While a modal is open, focus stays in it, so
    /// nothing behind it can be pressed.
    fn cycle_focus(&self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Bounds the search for a stop inside the modal.
        const MAX_STOPS: usize = 1024;
        let trap = self.focus_trap(cx);
        for _ in 0..MAX_STOPS {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
            if trap
                .as_ref()
                .is_none_or(|trap| trap.contains_focused(window, cx))
            {
                return;
            }
        }
        if let Some(trap) = trap {
            window.focus(&trap, cx);
        }
    }

    /// `Esc` clears the selection; on Settings it goes back.
    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == Route::Settings {
            self.navigate(self.back, window, cx);
        } else {
            self.list(ListOp::Clear, window, cx);
        }
    }

    /// Run a Containers-only key, when that screen is showing.
    fn on_containers(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ContainersScreen, &mut Context<ContainersScreen>),
    ) {
        if self.route == Route::Containers {
            self.containers.update(cx, f);
        }
    }

    /// Show a container detail tab, from any screen. Focus returns to the
    /// shell, as it may be in the live view being replaced; picking the tab
    /// already shown goes back into its terminal instead.
    fn show_tab(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Containers, window, cx);
        let focused = tab == self.containers.read(cx).tab()
            && self
                .containers
                .update(cx, |screen, cx| screen.focus_terminal(window, cx));
        if !focused {
            window.focus(&self.focus, cx);
            self.containers
                .update(cx, |screen, cx| screen.set_tab(tab, cx));
        }
    }

    /// Keys that act on the screens. Not registered while the confirm
    /// dialog is open, so they can't act behind it. They stay registered
    /// under the palette, which runs its commands through them.
    fn screen_actions(content: Stateful<Div>, cx: &Context<Self>) -> Stateful<Div> {
        content
            .on_action(cx.listener(Self::toggle_palette))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::show_shortcuts))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(|this, _: &GoContainers, window, cx| {
                this.navigate(Route::Containers, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoVolumes, window, cx| {
                this.navigate(Route::Volumes, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoImages, window, cx| {
                this.navigate(Route::Images, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoNetworks, window, cx| {
                this.navigate(Route::Networks, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoActivity, window, cx| {
                this.navigate(Route::Activity, window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoCleanup, window, cx| {
                this.navigate(Route::Cleanup, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                this.list(ListOp::SelectAll, window, cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteSelected, window, cx| {
                this.list(ListOp::Delete, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSelected, window, cx| {
                this.list(ListOp::Toggle, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, window, cx| {
                this.list(ListOp::Step(Step::Prev, false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNext, window, cx| {
                this.list(ListOp::Step(Step::Next, false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectFirst, window, cx| {
                this.list(ListOp::Step(Step::First, false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLast, window, cx| {
                this.list(ListOp::Step(Step::Last, false), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExtendPrev, window, cx| {
                this.list(ListOp::Step(Step::Prev, true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExtendNext, window, cx| {
                this.list(ListOp::Step(Step::Next, true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExtendFirst, window, cx| {
                this.list(ListOp::Step(Step::First, true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExtendLast, window, cx| {
                this.list(ListOp::Step(Step::Last, true), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CursorPrev, window, cx| {
                this.list(ListOp::Cursor(Step::Prev), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CursorNext, window, cx| {
                this.list(ListOp::Cursor(Step::Next), window, cx);
            }))
            .on_action(cx.listener(|_, _: &ScrollPageUp, window, cx| {
                scroll_active_pane(ScrollStep::PageUp, window, cx);
            }))
            .on_action(cx.listener(|_, _: &ScrollPageDown, window, cx| {
                scroll_active_pane(ScrollStep::PageDown, window, cx);
            }))
            .on_action(cx.listener(|_, _: &ScrollTop, window, cx| {
                scroll_active_pane(ScrollStep::Top, window, cx);
            }))
            .on_action(cx.listener(|_, _: &ScrollBottom, window, cx| {
                scroll_active_pane(ScrollStep::Bottom, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRunning, _, cx| {
                this.on_containers(cx, ContainersScreen::toggle_running);
            }))
            .on_action(cx.listener(|this, _: &Restart, _, cx| {
                this.on_containers(cx, ContainersScreen::restart);
            }))
            .on_action(cx.listener(|this, _: &OpenPort, window, cx| {
                this.on_containers(cx, |screen, cx| screen.open_port(window, cx));
            }))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| {
                this.on_containers(cx, |screen, cx| screen.set_expanded(false, cx));
            }))
            .on_action(cx.listener(|this, _: &Expand, _, cx| {
                this.on_containers(cx, |screen, cx| screen.set_expanded(true, cx));
            }))
            .on_action(cx.listener(|this, _: &ShowInfo, window, cx| this.show_tab(0, window, cx)))
            .on_action(cx.listener(|this, _: &ShowStats, window, cx| this.show_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &ShowLogs, window, cx| this.show_tab(2, window, cx)))
            .on_action(cx.listener(|this, _: &ShowTerminal, window, cx| {
                this.show_tab(3, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowFiles, window, cx| this.show_tab(4, window, cx)))
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        let collapsed = self.sidebar_collapsed;
        preferences::update(cx, |s| s.sidebar_collapsed = collapsed);
        cx.notify();
    }

    // ---- command palette ----

    fn toggle_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        let previous_focus = window.focused(cx);
        let view = cx.new(|cx| Palette::new(self.store.clone(), window, cx));
        cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.close_palette(window, cx);
                if let PaletteEvent::Run(target) = event {
                    this.run_palette_target(target, window, cx);
                }
            },
        )
        .detach();
        self.palette = Some(OpenPalette {
            view,
            previous_focus,
        });
        cx.notify();
    }

    /// Close the palette, giving focus back to where it was. If that is
    /// gone (a search field that closed on blur), the shell takes it.
    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(palette) = self.palette.take() {
            window.focus(palette.previous_focus.as_ref().unwrap_or(&self.focus), cx);
            cx.notify();
        }
    }

    /// Run a command as its keys would, or select an item on its screen.
    fn run_palette_target(&mut self, target: &Target, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Target::Command(shortcut) => {
                // Dispatched once this update is done, as the shell's own
                // listeners handle it; through the shell, which is always
                // in the rendered frame, unlike a focus that has closed.
                if let Some(action) = shortcut.command() {
                    let shell = self.focus.clone();
                    window.defer(cx, move |window, cx| {
                        shell.dispatch_action(action.as_ref(), window, cx);
                    });
                }
            }
            Target::Container(id) => {
                self.navigate(Route::Containers, window, cx);
                self.containers
                    .update(cx, |screen, cx| screen.reveal(id, cx));
            }
            Target::Image(id) => {
                self.navigate(Route::Images, window, cx);
                self.images.update(cx, |screen, cx| screen.reveal(id, cx));
            }
            Target::Volume(name) => {
                self.navigate(Route::Volumes, window, cx);
                self.volumes
                    .update(cx, |screen, cx| screen.reveal(name, cx));
            }
        }
    }

    // ---- confirm dialog ----

    /// Enter confirms while the dialog itself has focus. With a button
    /// focused (Tab), Enter presses that button instead, so Enter on Cancel
    /// cancels.
    fn confirm_key(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_focus.is_focused(window) {
            self.accept(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                    .on_click(cx.listener(|this, _, window, cx| this.accept(window, cx))),
            );
        modal_overlay(
            div()
                .track_focus(&self.dialog_focus)
                .key_context(CONFIRM_CONTEXT)
                .on_action(cx.listener(Self::confirm_key))
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
        let connection = self.store.read(cx).connection();
        let (status, name, detail) = engine_summary(connection);
        // With a fix to offer, the text opens the guide on Settings → Engine.
        let fixable = Problem::of(connection).is_some();
        let summary = div()
            .id("engine-summary")
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
                    .text_color(if fixable {
                        theme.accent
                    } else {
                        theme.text_faint
                    })
                    .truncate()
                    .child(detail),
            );
        div()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .px(px(Theme::SPACE_SM))
            .h(px(36.0))
            .child(status_dot(status, 8.0, None, theme))
            .child(match (fixable, self.sidebar_collapsed) {
                (false, _) => summary.into_any_element(),
                (true, collapsed) => {
                    let link = summary
                        .rounded(px(Theme::CONTROL_RADIUS))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_engine_help(window, cx);
                        }));
                    // Hidden in a collapsed sidebar: no tab stop.
                    if collapsed {
                        link.into_any_element()
                    } else {
                        TabStop::new("engine-summary", link).into_any_element()
                    }
                }
            })
            .child(
                IconButton::new("settings", icons::SETTINGS)
                    .size(28.0, 16.0)
                    .tab_stop(!self.sidebar_collapsed)
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
        let toggle = div()
            .id("sidebar-toggle")
            .border_1()
            .border_color(gpui::transparent_black())
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
            );
        TabStop::new("sidebar-toggle", toggle)
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let radius = chrome::content_radius(window);
        let theme = Theme::of(cx).clone();
        let collapsed = self.sidebar_collapsed;
        let leading = chrome::titlebar_leading(window, cx);
        let frame = Chrome {
            header_inset: if collapsed {
                leading - chrome::TITLEBAR_LEADING + TOGGLE_CLEARANCE
            } else {
                0.0
            },
            caption_clearance: chrome::caption_clearance(window, cx),
            left_radius: if collapsed { radius } else { 0.0 },
            right_radius: radius,
        };
        let screen = match self.route {
            Route::Containers => {
                self.containers
                    .update(cx, |screen, cx| screen.set_chrome(frame, cx));
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
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                this.cycle_focus(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrev, window, cx| {
                this.cycle_focus(false, window, cx);
            }))
            .map(|content| {
                if self.confirm.is_none() {
                    Self::screen_actions(content, cx)
                } else {
                    content
                }
            })
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
            .children(captions)
            .child(self.render_toasts(cx))
            .children(self.palette.as_ref().map(|palette| palette.view.clone()))
            .children(confirm);

        chrome::window_frame(content.into_any_element(), window, cx)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use berth_docker::fixtures::FixtureEngine;
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::keymap;

    fn open_shell(cx: &mut TestAppContext) -> (Entity<Shell>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_base::init(cx);
            berth_ui::init(cx);
            preferences::init_in_memory(cx);
            keymap::bind(cx);
        });
        let connector: Connector = Arc::new(|| Ok(Arc::new(FixtureEngine::default())));
        let (shell, cx) = cx.add_window_view(|window, cx| Shell::new(connector, window, cx));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        (shell, cx)
    }

    fn route(shell: &Entity<Shell>, cx: &VisualTestContext) -> Route {
        shell.read_with(cx, |shell, _| shell.route)
    }

    fn palette_open(shell: &Entity<Shell>, cx: &VisualTestContext) -> bool {
        shell.read_with(cx, |shell, _| shell.palette.is_some())
    }

    #[gpui::test]
    fn palette_runs_commands_and_jumps_to_items(cx: &mut TestAppContext) {
        let (shell, cx) = open_shell(cx);

        cx.simulate_keystrokes("secondary-k");
        assert!(palette_open(&shell, cx));
        cx.simulate_input("go to images");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!palette_open(&shell, cx));
        assert_eq!(route(&shell, cx), Route::Images);

        let volume = shell.read_with(cx, |shell, cx| {
            shell
                .store
                .read(cx)
                .volumes()
                .first()
                .map(|v| v.name.clone())
        });
        let Some(volume) = volume else {
            panic!("the fixtures have no volumes");
        };
        cx.simulate_keystrokes("secondary-k");
        cx.simulate_input(&volume);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(route(&shell, cx), Route::Volumes);
        let selected = shell.read_with(cx, |shell, cx| {
            shell.volumes.read(cx).selected().map(str::to_string)
        });
        assert_eq!(selected, Some(volume));
    }

    #[gpui::test]
    fn palette_fits_the_smallest_window(cx: &mut TestAppContext) {
        let (_, cx) = open_shell(cx);
        // Without the entrance spring, so the card is where it settles.
        cx.update(|_, cx| preferences::update(cx, |s| s.reduce_motion = true));
        let window = gpui::size(px(860.0), px(520.0));
        cx.simulate_resize(window);
        cx.simulate_keystrokes("secondary-k");
        cx.run_until_parked();
        let Some(palette) = cx.debug_bounds("palette") else {
            panic!("the palette didn't render");
        };
        assert!(palette.bottom() <= window.height, "{palette:?}");
    }

    #[gpui::test]
    fn closing_the_palette_restores_focus(cx: &mut TestAppContext) {
        let (shell, cx) = open_shell(cx);
        cx.simulate_keystrokes("secondary-f");
        cx.simulate_input("db");
        let before = cx.update(|window, cx| window.focused(cx));
        assert!(before.is_some());

        cx.simulate_keystrokes("secondary-k");
        assert!(palette_open(&shell, cx));
        let in_palette = cx.update(|window, cx| window.focused(cx));
        assert_ne!(in_palette, before);
        // Keys for the screens don't act behind it, nor take its focus.
        cx.simulate_keystrokes("alt-3");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), in_palette);
        assert_eq!(
            shell.read_with(cx, |shell, cx| shell.containers.read(cx).tab()),
            0
        );
        cx.simulate_keystrokes("down up escape");
        assert!(!palette_open(&shell, cx));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), before);

        // ctrl-k toggles it closed too.
        cx.simulate_keystrokes("secondary-k secondary-k");
        assert!(!palette_open(&shell, cx));
        assert_eq!(cx.update(|window, cx| window.focused(cx)), before);
    }
}

//! Settings (stories H1, H2): appearance and engine endpoint. Every change is
//! applied immediately and persisted by [`crate::preferences`]. Shortcuts
//! lists the keyboard map (NFR-U1).

use berth_docker::{DiscoveryEnv, docker_contexts, parse_host};
use berth_settings::{Accent, AppearanceMode, EngineChoice};
use berth_ui::{
    Appearance, Button, ColorSwatch, OPTION_PREVIEW_RADIUS, OptionCard, ScrollPane, SegmentedTabs,
    SettingRow, SettingsGroup, Switch, TextField, Theme, icons, page_title, section_title,
    status_dot,
};
use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, Hsla, IntoElement, Render, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use gpui_base::input::{InputEvent, InputState};

use super::engine_help::{Problem, engine_help};
use super::{Chrome, engine_summary};
use crate::chrome;
use crate::keymap::{self, Group, SHORTCUTS};
use crate::preferences;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Engine,
    Shortcuts,
}

impl Page {
    pub const ALL: [Self; 3] = [Self::Appearance, Self::Engine, Self::Shortcuts];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Engine => "Engine",
            Self::Shortcuts => "Shortcuts",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Appearance => icons::TUNING,
            Self::Engine => icons::SERVER,
            Self::Shortcuts => icons::KEYBOARD,
        }
    }
}

const fn mode_icon(mode: AppearanceMode) -> &'static str {
    match mode {
        AppearanceMode::System => icons::MONITOR,
        AppearanceMode::Light => icons::SUN,
        AppearanceMode::Dark => icons::MOON,
    }
}

/// The endpoint tab shown. It can run ahead of the saved choice: picking
/// Context or Custom only saves once a context or host is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineTab {
    Auto,
    Context,
    Custom,
}

impl EngineTab {
    const ALL: [Self; 3] = [Self::Auto, Self::Context, Self::Custom];

    const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Context => "Context",
            Self::Custom => "Custom",
        }
    }

    const fn of(choice: &EngineChoice) -> Self {
        match choice {
            EngineChoice::Auto => Self::Auto,
            EngineChoice::Context { .. } => Self::Context,
            EngineChoice::Host { .. } => Self::Custom,
        }
    }
}

pub struct SettingsScreen {
    store: Entity<Store>,
    page: Page,
    chrome: Chrome,
    tab: EngineTab,
    /// Docker contexts on disk; `None` until listed.
    contexts: Option<Vec<String>>,
    host: Entity<InputState>,
    host_error: Option<SharedString>,
    _host_events: Subscription,
}

impl SettingsScreen {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<preferences::Preferences>(|_, cx| cx.notify())
            .detach();
        let saved = preferences::get(cx).engine.clone();
        let saved_host = match &saved {
            EngineChoice::Host { host } => host.clone(),
            EngineChoice::Auto | EngineChoice::Context { .. } => String::new(),
        };
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("unix:///run/user/1000/podman/podman.sock")
                .default_value(saved_host)
        });
        let host_events =
            cx.subscribe_in(
                &host,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.apply_host(cx),
                    InputEvent::Change if this.host_error.is_some() => {
                        this.host_error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        Self {
            store,
            page: Page::Appearance,
            chrome: Chrome::default(),
            tab: EngineTab::of(&saved),
            contexts: None,
            host,
            host_error: None,
            _host_events: host_events,
        }
    }

    pub const fn page(&self) -> Page {
        self.page
    }

    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            if page == Page::Engine {
                Self::list_contexts(cx);
            }
            cx.notify();
        }
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
            cx.notify();
        }
    }

    // ---- engine ----

    /// Re-read the Docker contexts off the UI thread.
    fn list_contexts(cx: &Context<Self>) {
        let list = cx
            .background_executor()
            .spawn(async { docker_contexts(&DiscoveryEnv::from_process()) });
        cx.spawn(async move |this, cx| {
            let contexts = list.await;
            this.update(cx, |this, cx| {
                this.contexts = Some(contexts);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Save `choice` and, if it changed, drop the old engine's state and
    /// connect to the new one.
    fn set_engine(&self, choice: EngineChoice, cx: &mut Context<Self>) {
        if preferences::update(cx, |s| s.engine = choice) {
            self.store.update(cx, Store::switch_engine);
        }
    }

    fn choose_tab(&mut self, tab: EngineTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.host_error = None;
        if tab == EngineTab::Auto {
            self.set_engine(EngineChoice::Auto, cx);
        }
        cx.notify();
    }

    fn apply_host(&mut self, cx: &mut Context<Self>) {
        let host = self.host.read(cx).value().trim().to_string();
        match validate_host(&host) {
            Ok(()) => {
                self.host_error = None;
                self.set_engine(EngineChoice::Host { host }, cx);
            }
            Err(error) => self.host_error = Some(error.into()),
        }
        cx.notify();
    }

    fn render_engine(&self, cx: &Context<Self>) -> Div {
        let theme = Theme::of(cx);
        let connection = self.store.read(cx).connection();
        let (status, title, detail) = engine_summary(connection);
        let problem = Problem::of(connection);
        let connection = SettingsGroup::new().row(
            SettingRow::new(title).description(detail).control(
                div()
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_MD))
                    .child(status_dot(status, 8.0, None, theme))
                    .child(
                        Button::new("engine-reconnect", "Reconnect")
                            .icon(icons::RESTART)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, Store::reconnect);
                            })),
                    ),
            ),
        );

        let tabs = SegmentedTabs::new(
            "engine-choice",
            EngineTab::ALL.into_iter().map(EngineTab::label),
        )
        .selected(
            EngineTab::ALL
                .iter()
                .position(|t| *t == self.tab)
                .unwrap_or(0),
        )
        .on_select(cx.listener(|this, ix: &usize, _, cx| {
            if let Some(tab) = EngineTab::ALL.get(*ix) {
                this.choose_tab(*tab, cx);
            }
        }));
        let endpoint = SettingRow::new("Connect to").control(tabs);
        let endpoint = match self.tab {
            EngineTab::Auto => endpoint.description(
                "DOCKER_HOST, then the current Docker context, then the rootful, \
                 rootless and Podman sockets.",
            ),
            EngineTab::Context => endpoint
                .description(
                    "A named Docker context, whichever one the docker CLI has selected. \
                 Takes precedence over DOCKER_HOST.",
                )
                .body(self.render_contexts(cx)),
            EngineTab::Custom => endpoint
                .description("A Docker-compatible API address. Takes precedence over DOCKER_HOST.")
                .body(self.render_host(cx)),
        };

        div()
            .child(page_title("Engine", theme))
            .child(section_title("Connection", theme))
            .child(connection)
            .child(section_title("Endpoint", theme))
            .child(SettingsGroup::new().row(endpoint))
            .children(problem.map(|problem| engine_help(&problem, theme)))
    }

    fn render_contexts(&self, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let hint = |text: &'static str| {
            div()
                .px(px(2.0))
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text_faint)
                .child(text)
                .into_any_element()
        };
        let Some(contexts) = &self.contexts else {
            return hint("Looking for Docker contexts…");
        };
        if contexts.is_empty() {
            return hint("No Docker contexts found. Create one with `docker context create`.");
        }
        let selected = match &preferences::get(cx).engine {
            EngineChoice::Context { name } => Some(name.clone()),
            EngineChoice::Auto | EngineChoice::Host { .. } => None,
        };
        div()
            .flex()
            .flex_col()
            .rounded(px(Theme::ROW_RADIUS))
            .border_1()
            .border_color(theme.border)
            .children(contexts.iter().enumerate().map(|(ix, name)| {
                let active = selected.as_deref() == Some(name.as_str());
                let choice = name.clone();
                div()
                    .h(px(40.0))
                    .px(px(Theme::SPACE_MD))
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_MD))
                    .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.font_mono.clone())
                            .text_size(px(Theme::TEXT_MD))
                            .text_color(theme.text)
                            .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(name.clone()),
                    )
                    .child(if active {
                        div()
                            .text_size(px(Theme::TEXT_SM))
                            .text_color(theme.accent)
                            .child("In use")
                            .into_any_element()
                    } else {
                        Button::new(SharedString::from(format!("context-{name}")), "Use")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_engine(
                                    EngineChoice::Context {
                                        name: choice.clone(),
                                    },
                                    cx,
                                );
                            }))
                            .into_any_element()
                    })
            }))
            .into_any_element()
    }

    fn render_host(&self, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_SM))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_SM))
                    .child(
                        div().flex_1().min_w_0().child(
                            TextField::new(&self.host)
                                .mono()
                                .invalid(self.host_error.is_some()),
                        ),
                    )
                    .child(
                        Button::new("engine-apply", "Connect")
                            .solid()
                            .on_click(cx.listener(|this, _, _, cx| this.apply_host(cx))),
                    ),
            )
            .child(
                div()
                    .px(px(2.0))
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(if self.host_error.is_some() {
                        theme.danger
                    } else {
                        theme.text_faint
                    })
                    .child(self.host_error.clone().unwrap_or_else(|| {
                        "unix://, tcp://, http(s):// or npipe:// — TLS follows \
                         DOCKER_TLS_VERIFY and DOCKER_CERT_PATH."
                            .into()
                    })),
            )
            .into_any_element()
    }

    /// A load/save problem; a failed save offers to write again.
    fn render_problem(cx: &Context<Self>) -> Option<AnyElement> {
        let problem = preferences::problem(cx)?;
        let save_failed = preferences::save_failed(cx);
        let theme = Theme::of(cx);
        Some(
            div()
                .mb(px(Theme::SPACE_MD))
                .p(px(Theme::SPACE_MD))
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_MD))
                .rounded(px(Theme::PANEL_RADIUS))
                .bg(theme.danger_wash)
                .border_1()
                .border_color(theme.danger.opacity(0.3))
                .child(
                    berth_ui::icon(icons::DANGER)
                        .size(px(16.0))
                        .text_color(theme.danger),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(Theme::TEXT_SM))
                        .text_color(theme.text)
                        .child(problem),
                )
                .when(save_failed, |d| {
                    d.child(
                        Button::new("settings-save-again", "Save again")
                            .on_click(|_, _, cx| preferences::retry_save(cx)),
                    )
                })
                .into_any_element(),
        )
    }

    // ---- appearance ----

    fn render_appearance(cx: &Context<Self>) -> Div {
        let theme = Theme::of(cx);
        let settings = preferences::get(cx);
        let accent = settings.accent;

        let schemes =
            div()
                .flex()
                .gap(px(Theme::SPACE_LG))
                .children(AppearanceMode::ALL.into_iter().map(|mode| {
                    OptionCard::new(
                        SharedString::from(format!("scheme-{}", mode.label())),
                        mode.label(),
                        scheme_preview(mode, accent),
                    )
                    .icon(mode_icon(mode))
                    .selected(settings.appearance == mode)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        preferences::update(cx, |s| s.appearance = mode);
                    }))
                }));

        let swatches = div()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_XS))
            .children(Accent::ALL.into_iter().map(|choice| {
                ColorSwatch::new(
                    SharedString::from(format!("accent-{}", choice.label())),
                    Theme::new(theme.appearance, choice).accent,
                )
                .selected(choice == accent)
                .on_click(cx.listener(move |_, _, _, cx| {
                    preferences::update(cx, |s| s.accent = choice);
                }))
            }));

        let reduce_motion = settings.reduce_motion;
        let group =
            SettingsGroup::new()
                .row(
                    SettingRow::new("Accent color")
                        .description(accent.label())
                        .control(swatches),
                )
                .row(
                    SettingRow::new("Reduce motion")
                        .description("Open and close panels without animating them.")
                        .control(Switch::new("reduce-motion", reduce_motion).on_toggle(
                            cx.listener(move |_, _, _, cx| {
                                preferences::update(cx, |s| s.reduce_motion = !reduce_motion);
                            }),
                        )),
                );

        div()
            .child(page_title("Appearance", theme))
            .child(section_title("Color scheme", theme))
            .child(schemes)
            .child(div().h(px(Theme::SPACE_LG + 4.0)))
            .child(group)
    }
}

impl SettingsScreen {
    // ---- shortcuts ----

    fn render_shortcuts(cx: &Context<Self>) -> Div {
        let theme = Theme::of(cx);
        Group::ALL.into_iter().fold(
            div().child(page_title("Shortcuts", theme)),
            |page, group| {
                let rows = SHORTCUTS
                    .iter()
                    .filter(|shortcut| shortcut.group == group)
                    .fold(SettingsGroup::new(), |rows, shortcut| {
                        rows.row(
                            SettingRow::new(shortcut.label).control(keys(shortcut.keys, theme)),
                        )
                    });
                page.child(section_title(group.label(), theme)).child(rows)
            },
        )
    }
}

/// Keycaps for each alternative keystroke, separated by "or".
fn keys(keystrokes: &[&str], theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(Theme::SPACE_SM))
        .children(keystrokes.iter().enumerate().flat_map(|(ix, keystroke)| {
            let or = (ix > 0).then(|| {
                div()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_faint)
                    .child("or")
                    .into_any_element()
            });
            let caps = div()
                .flex()
                .gap(px(3.0))
                .children(keymap::keycaps(keystroke).into_iter().map(|cap| {
                    div()
                        .h(px(22.0))
                        .min_w(px(22.0))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(Theme::CONTROL_RADIUS))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.row_hover)
                        .text_size(px(Theme::TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .child(cap)
                }))
                .into_any_element();
            or.into_iter().chain([caps])
        }))
}

/// The same syntax check discovery applies, so a saved host never fails
/// to parse at connect time.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("Enter an engine address.".into());
    }
    parse_host(host)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// A miniature window in `appearance`: sidebar of label bars beside a content
/// panel whose first line takes the accent.
fn mini_window(appearance: Appearance, accent: Accent) -> Div {
    let palette = Theme::new(appearance, accent);
    let bar = |width: f32, color: Hsla| div().h(px(4.0)).w(px(width)).rounded_full().bg(color);
    let ink = palette.text_faint.opacity(0.55);
    div()
        .size_full()
        .flex()
        .gap(px(6.0))
        .p(px(8.0))
        .bg(palette.surface)
        .child(
            div()
                .w(px(30.0))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .pt(px(4.0))
                .child(bar(22.0, ink))
                .child(bar(26.0, ink))
                .child(bar(18.0, ink))
                .child(bar(24.0, ink)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .p(px(8.0))
                .rounded(px(5.0))
                .bg(palette.bg)
                .border_1()
                .border_color(palette.border)
                .child(bar(40.0, palette.accent))
                .child(bar(72.0, ink))
                .child(bar(58.0, ink))
                .child(bar(66.0, ink)),
        )
}

fn scheme_preview(mode: AppearanceMode, accent: Accent) -> AnyElement {
    let radius = px(OPTION_PREVIEW_RADIUS);
    match mode {
        AppearanceMode::Light | AppearanceMode::Dark => {
            let appearance = if mode == AppearanceMode::Light {
                Appearance::Light
            } else {
                Appearance::Dark
            };
            mini_window(appearance, accent)
                .rounded(radius)
                .overflow_hidden()
                .into_any_element()
        }
        AppearanceMode::System => div()
            .size_full()
            .flex()
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .rounded_l(radius)
                    .child(mini_window(Appearance::Light, accent)),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .rounded_r(radius)
                    .child(mini_window(Appearance::Dark, accent)),
            )
            .into_any_element(),
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let content = match self.page {
            Page::Appearance => Self::render_appearance(cx),
            Page::Engine => self.render_engine(cx),
            Page::Shortcuts => Self::render_shortcuts(cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .rounded_l(px(self.chrome.left_radius))
            .rounded_r(px(self.chrome.right_radius))
            .child(chrome::drag_region(
                "settings-titlebar",
                div().h(px(Theme::HEADER_HEIGHT)).flex_none().w_full(),
            ))
            .child(
                div().flex_1().min_h_0().child(ScrollPane::new(
                    "settings-scroll",
                    div()
                        .w_full()
                        .max_w(px(720.0))
                        .mx_auto()
                        .px(px(Theme::SPACE_LG * 2.0))
                        .pb(px(Theme::SPACE_LG * 3.0))
                        .children(Self::render_problem(cx))
                        .child(content),
                )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_validation() {
        assert!(validate_host("unix:///var/run/docker.sock").is_ok());
        assert!(validate_host("tcp://10.0.0.2:2376").is_ok());
        assert!(validate_host("npipe:////./pipe/docker_engine").is_ok());
        assert!(validate_host("").is_err());
        assert!(validate_host("/var/run/docker.sock").is_err());
        assert!(validate_host("ssh://box").is_err());
        assert!(validate_host("tcp://").is_err());
        assert!(validate_host("unix://relative.sock").is_err());
        assert!(validate_host("tcp:///localhost:2375").is_err());
        assert!(validate_host("tcp://local host:2375").is_err());
    }
}

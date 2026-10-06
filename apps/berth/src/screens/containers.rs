//! Containers screen: list column (stacks + containers) and detail pane.
//!
//! Reads the [`Store`]; lifecycle actions go through it, destructive ones are
//! confirmed first via a [`ConfirmRequest`] that the shell presents.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use berth_docker::{Container, ContainerState, ContainerTree, MountKind, Node, Stack};
use berth_ui::SearchField;
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, SectionLabel, SegmentedTabs, Spinner, Status, Theme, empty_state, icon, icons,
    status_dot,
};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, IntoElement, KeyDownEvent, Render,
    SharedString, Subscription, Window, div, list, prelude::*, px,
};
use gpui_base::input::{InputEvent, InputState};

use crate::chrome;
use crate::dialogs::ConfirmRequest;
use crate::live::{Live, LiveKey, LiveTab};
use crate::store::{Connection, Op, Store};
use crate::virtual_rows::VirtualRows;

const TABS: [&str; 5] = ["Info", "Stats", "Logs", "Terminal", "Files"];

/// One line of the (virtualized) container list. Descriptors only: rows are
/// built from the store when they scroll into view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Section { running: bool, count: usize },
    Stack { project: String },
    Container { id: String, depth: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Stack(String),
    Container(String),
}

pub struct ContainersScreen {
    store: Entity<Store>,
    /// Compose projects the user collapsed (stacks start expanded).
    collapsed: HashSet<String>,
    selection: Option<Selection>,
    tab: usize,
    search: Option<(Entity<InputState>, Subscription)>,
    /// Lower-cased, trimmed search query; empty = no filter.
    query: String,
    header_inset: f32,
    left_radius: f32,
    right_radius: f32,
    rows: VirtualRows<Row>,
    /// Compose project → member container ids, in service order.
    stack_members: HashMap<String, Vec<String>>,
    /// Container id → index into the store's container list.
    index: HashMap<String, usize>,
    /// The open Stats/Logs/Terminal view, streaming only while shown.
    live: Live,
    /// Whether the shell is showing this screen.
    visible: bool,
}

impl EventEmitter<ConfirmRequest> for ContainersScreen {}

impl ContainersScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |this, store, cx| {
            this.drop_stale_selection(store.read(cx).containers());
            cx.notify();
        })
        .detach();
        Self {
            store,
            collapsed: HashSet::new(),
            selection: None,
            tab: 0,
            search: None,
            query: String::new(),
            header_inset: 0.0,
            left_radius: 0.0,
            right_radius: 0.0,
            rows: VirtualRows::new(),
            stack_members: HashMap::new(),
            index: HashMap::new(),
            live: Live::default(),
            visible: true,
        }
    }

    /// The shell shows or hides this screen; live streams stop while hidden.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            // A hidden screen doesn't render, so close the stream here.
            if !visible {
                self.live = Live::default();
            }
            cx.notify();
        }
    }

    /// The live view the current state calls for, if any.
    fn live_key(&self, store: &Store) -> Option<LiveKey> {
        let tab = match self.tab {
            1 => LiveTab::Stats,
            2 => LiveTab::Logs,
            3 => LiveTab::Terminal,
            _ => return None,
        };
        let Some(Selection::Container(id)) = self.selection.as_ref().filter(|_| self.visible)
        else {
            return None;
        };
        let container = store.containers().iter().find(|c| &c.id == id)?;
        Some(LiveKey {
            tab,
            container: id.clone(),
            running: container.state == ContainerState::Running,
            engine: store.engine()?,
        })
    }

    /// Rebuild row descriptors from the current tree and splice only the
    /// changed range into the list state, so the scroll position survives
    /// containers moving between sections.
    fn sync_rows(&mut self, tree: &ContainerTree<'_>, store: &Store) {
        let mut rows = Vec::new();
        let mut stack_members = HashMap::new();
        for (running, nodes) in [(true, &tree.running), (false, &tree.stopped)] {
            if nodes.is_empty() {
                continue;
            }
            let count = if running {
                tree.running_container_count()
            } else {
                0
            };
            rows.push(Row::Section { running, count });
            for node in nodes {
                match node {
                    Node::Container(c) => rows.push(Row::Container {
                        id: c.id.clone(),
                        depth: 0,
                    }),
                    Node::Stack(stack) => {
                        let project = stack.project.to_string();
                        let members: Vec<String> =
                            stack.containers.iter().map(|c| c.id.clone()).collect();
                        rows.push(Row::Stack {
                            project: project.clone(),
                        });
                        let expanded = !self.query.is_empty() || !self.collapsed.contains(&project);
                        if expanded {
                            rows.extend(members.iter().map(|id| Row::Container {
                                id: id.clone(),
                                depth: 1,
                            }));
                        }
                        stack_members.insert(project, members);
                    }
                }
            }
        }
        self.rows.sync(rows);
        self.stack_members = stack_members;
        self.index = store
            .containers()
            .iter()
            .enumerate()
            .map(|(ix, c)| (c.id.clone(), ix))
            .collect();
    }

    fn render_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let containers = store.containers();
        let lookup = |id: &String| self.index.get(id).and_then(|&ix| containers.get(ix));
        let row = match self.rows.get(ix) {
            None => return div().into_any_element(),
            Some(Row::Section {
                running: true,
                count,
            }) => SectionLabel::new("Running")
                .count(*count)
                .into_any_element(),
            Some(Row::Section { running: false, .. }) => {
                SectionLabel::new("Stopped").into_any_element()
            }
            Some(Row::Stack { project }) => {
                let stack = Stack {
                    project,
                    containers: self
                        .stack_members
                        .get(project)
                        .map(|ids| ids.iter().filter_map(lookup).collect())
                        .unwrap_or_default(),
                };
                let expanded = !self.query.is_empty() || !self.collapsed.contains(project);
                self.stack_row(store, &stack, expanded, cx)
                    .into_any_element()
            }
            Some(Row::Container { id, depth }) => lookup(id).map_or_else(
                || div().into_any_element(),
                |c| self.container_row(store, c, *depth, cx).into_any_element(),
            ),
        };
        div()
            .px(px(Theme::SPACE_SM))
            .pb(px(2.0))
            .child(row)
            .into_any_element()
    }

    /// Layout facts owned by the shell: clearance for the floating sidebar
    /// toggle and the window's outer corner radii.
    pub fn set_chrome(&mut self, header_inset: f32, left: f32, right: f32, cx: &mut Context<Self>) {
        if (self.header_inset, self.left_radius, self.right_radius) != (header_inset, left, right) {
            self.header_inset = header_inset;
            self.left_radius = left;
            self.right_radius = right;
            cx.notify();
        }
    }

    /// Show (or refocus) the search field.
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder("Filter containers"));
            let subscription = cx.subscribe_in(
                &state,
                window,
                |this, state, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        this.query = state.read(cx).value().trim().to_lowercase();
                        cx.notify();
                    }
                    InputEvent::Blur if this.query.is_empty() => this.close_search(cx),
                    _ => {}
                },
            );
            self.search = Some((state, subscription));
        }
        if let Some((state, _)) = &self.search {
            state.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search = None;
        self.query.clear();
        cx.notify();
    }

    fn drop_stale_selection(&mut self, containers: &[Container]) {
        let exists = match &self.selection {
            None => return,
            Some(Selection::Container(id)) => containers.iter().any(|c| &c.id == id),
            Some(Selection::Stack(project)) => containers
                .iter()
                .any(|c| c.compose.as_ref().is_some_and(|cr| &cr.project == project)),
        };
        if !exists {
            self.selection = None;
        }
    }

    fn run(&self, ids: Vec<String>, op: Op, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.run(ids, op, cx));
    }

    fn confirm_delete(
        &self,
        title: String,
        message: &str,
        names: Vec<SharedString>,
        ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        cx.emit(ConfirmRequest {
            title: title.into(),
            message: message.to_string().into(),
            items: names,
            confirm_label: "Delete".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| store.run(ids.clone(), Op::Remove, cx));
            }),
        });
    }

    fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = Some(selection);
        cx.notify();
    }

    // ---- list column ----

    fn render_list(
        &self,
        store: &Store,
        tree: &ContainerTree<'_>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx);
        let running = tree.running_container_count();

        let subtitle = match store.connection() {
            Connection::Connecting => "Connecting…".to_string(),
            Connection::Disconnected { .. } => "Not connected".to_string(),
            Connection::Connected(_) if running == 0 => "None running".to_string(),
            Connection::Connected(_) => format!("{running} running"),
        };
        let header = ListHeader::new("Containers")
            .subtitle(subtitle)
            .leading_inset(self.header_inset)
            .action(
                IconButton::new("search", icons::SEARCH)
                    .active(self.search.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.search.is_some() {
                            this.close_search(cx);
                        } else {
                            this.open_search(window, cx);
                        }
                    })),
            )
            .action(IconButton::new("new-container", icons::PLUS));

        let body: AnyElement = if self.rows.is_empty() {
            self.render_list_placeholder(store, cx)
        } else {
            list(
                self.rows.state.clone(),
                cx.processor(|this, ix: usize, _, cx| this.render_row(ix, cx)),
            )
            .flex_1()
            .min_h_0()
            .pb(px(Theme::SPACE_MD))
            .into_any_element()
        };

        div()
            .w(px(Theme::LIST_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .rounded_tl(px(self.left_radius))
            .rounded_bl(px(self.left_radius))
            .border_r_1()
            .border_color(theme.border)
            .child(chrome::drag_region("list-titlebar", div().child(header)))
            .when_some(self.search.as_ref(), |d, (state, _)| {
                d.child(
                    div()
                        .px(px(Theme::SPACE_SM + 2.0))
                        .pb(px(Theme::SPACE_XS))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.close_search(cx);
                            }
                        }))
                        .child(SearchField::new(state).on_clear(cx.listener(
                            |this, _, window, cx| {
                                if let Some((state, _)) = &this.search {
                                    state.update(cx, |state, cx| state.set_value("", window, cx));
                                }
                            },
                        ))),
                )
            })
            .child(body)
    }

    /// Connecting / disconnected / empty states for the list column.
    fn render_list_placeholder(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let column = div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_LG * 2.0))
            .pb(px(Theme::HEADER_HEIGHT));
        let title = |text: &'static str| {
            div()
                .text_size(px(Theme::TEXT_LG))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(text)
        };
        let detail = |text: String| {
            div()
                .text_size(px(Theme::TEXT_SM))
                .line_height(px(17.0))
                .text_center()
                .text_color(theme.text_faint)
                .child(text)
        };
        match store.connection() {
            Connection::Connecting => column
                .child(Spinner::new("connecting").size(18.0))
                .child(title("Connecting to engine…"))
                .into_any_element(),
            Connection::Disconnected { error } => column
                .child(title("Can't reach the engine"))
                .child(detail(error.clone()))
                .child(div().pt(px(Theme::SPACE_SM)).child(
                    Button::new("reconnect", "Retry").on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, Store::reconnect);
                    })),
                ))
                .into_any_element(),
            Connection::Connected(_) if !self.query.is_empty() => column
                .child(title("No matches"))
                .child(detail(format!(
                    "Nothing matches “{}” by name, image or project.",
                    self.query
                )))
                .into_any_element(),
            Connection::Connected(_) => column
                .child(title("No containers"))
                .child(detail(
                    "Run a container or `docker compose up` and it shows up here.".into(),
                ))
                .into_any_element(),
        }
    }

    fn stack_row(
        &self,
        store: &Store,
        stack: &Stack<'_>,
        expanded: bool,
        cx: &Context<Self>,
    ) -> ResourceRow {
        let project = stack.project.to_string();
        let active = stack.is_active();
        let selected = self.selection == Some(Selection::Stack(project.clone()));
        let total = stack.containers.len();
        let busy = stack
            .containers
            .iter()
            .find_map(|c| store.pending(&c.id))
            .map(Op::progress_label);
        // Stop what runs; start everything otherwise.
        let lifecycle_ids: Vec<String> = stack
            .containers
            .iter()
            .filter(|c| !active || c.state.is_active())
            .map(|c| c.id.clone())
            .collect();
        let all_ids: Vec<String> = stack.containers.iter().map(|c| c.id.clone()).collect();
        let names: Vec<SharedString> = stack
            .containers
            .iter()
            .map(|c| SharedString::from(c.name.clone()))
            .collect();

        let toggle = {
            let project = project.clone();
            cx.listener(move |this, _, _, cx| {
                if !this.collapsed.remove(&project) {
                    this.collapsed.insert(project.clone());
                }
                cx.notify();
            })
        };
        let lifecycle = cx.listener(move |this, _, _, cx| {
            let op = if active { Op::Stop } else { Op::Start };
            this.run(lifecycle_ids.clone(), op, cx);
        });
        let delete = {
            let project = project.clone();
            cx.listener(move |this, _, _, cx| {
                this.confirm_delete(
                    format!("Delete stack {project}?"),
                    "All of its containers are stopped and removed. Volumes are kept.",
                    names.clone(),
                    all_ids.clone(),
                    cx,
                );
            })
        };
        let select = {
            let project = project.clone();
            cx.listener(move |this, _, _, cx| this.select(Selection::Stack(project.clone()), cx))
        };

        ResourceRow::new(
            SharedString::from(format!("stack-{project}")),
            project.clone(),
        )
        .leading(
            Badge::icon(project.clone(), icons::LAYERS)
                .status(if active {
                    Status::Running
                } else {
                    Status::Stopped
                })
                .dimmed(!active),
        )
        .subtitle(match stack.active_count() {
            0 => format!("{total} services"),
            n => format!("{n} of {total} running"),
        })
        .busy(busy)
        .disclosure(expanded, toggle)
        .selected(selected)
        .dimmed(!active)
        .action(lifecycle_button(&project, active).on_click(lifecycle))
        .action(delete_button(&project).on_click(delete))
        .on_click(select)
    }

    fn container_row(
        &self,
        store: &Store,
        c: &Container,
        depth: usize,
        cx: &Context<Self>,
    ) -> ResourceRow {
        let id = c.id.clone();
        let active = c.state.is_active();
        let selected = self.selection == Some(Selection::Container(id.clone()));
        let title = match (&c.compose, depth) {
            (Some(compose), 1) => compose.service.clone(),
            _ => c.name.clone(),
        };

        let lifecycle = {
            let ids = vec![id.clone()];
            cx.listener(move |this, _, _, cx| {
                let op = if active { Op::Stop } else { Op::Start };
                this.run(ids.clone(), op, cx);
            })
        };
        let delete = {
            let ids = vec![id.clone()];
            let name = c.name.clone();
            let message = if active {
                "The container is stopped and removed. Its volumes are kept."
            } else {
                "The container is removed. Its volumes are kept."
            };
            cx.listener(move |this, _, _, cx| {
                this.confirm_delete(
                    format!("Delete {name}?"),
                    message,
                    Vec::new(),
                    ids.clone(),
                    cx,
                );
            })
        };
        let select = {
            let id = id.clone();
            cx.listener(move |this, _, _, cx| this.select(Selection::Container(id.clone()), cx))
        };

        let mut row = ResourceRow::new(SharedString::from(format!("ctr-{id}")), title)
            .leading(
                Badge::monogram(image_name(&c.image))
                    .status(status_of(c.state))
                    .dimmed(!active),
            )
            .subtitle(c.image.clone())
            .busy(store.pending(&id).map(Op::progress_label))
            .depth(depth)
            .reserve_disclosure()
            .selected(selected)
            .dimmed(!active);

        if let Some(url) = c.web_url().filter(|_| active) {
            row = row.action(
                IconButton::new(SharedString::from(format!("open-{id}")), icons::LINK)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        if active {
            let ids = vec![id.clone()];
            row = row.action(
                IconButton::new(SharedString::from(format!("restart-{id}")), icons::RESTART)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.run(ids.clone(), Op::Restart, cx)),
                    ),
            );
        }
        row.action(lifecycle_button(&id, active).on_click(lifecycle))
            .action(delete_button(&id).on_click(delete))
            .on_click(select)
    }

    // ---- detail pane ----

    fn render_detail(
        &self,
        store: &Store,
        tree: &ContainerTree<'_>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = Theme::of(cx);
        let tabs = SegmentedTabs::new("detail-tabs", TABS)
            .selected(self.tab)
            .on_select(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = *ix;
                cx.notify();
            }));

        let body: AnyElement = match (&self.selection, self.tab) {
            (None, _) => empty_state(
                "No selection",
                "Select a container or compose stack to inspect it.",
                theme,
            )
            .into_any_element(),
            (Some(Selection::Container(id)), 0) => {
                store.containers().iter().find(|c| &c.id == id).map_or_else(
                    || empty_state("Gone", "This container was removed.", theme).into_any_element(),
                    |c| scroll_page(container_info(c, theme)).into_any_element(),
                )
            }
            (Some(Selection::Stack(project)), 0) => {
                let stack = tree
                    .running
                    .iter()
                    .chain(&tree.stopped)
                    .find_map(|n| match n {
                        Node::Stack(s) if s.project == project => Some(s),
                        _ => None,
                    });
                stack.map_or_else(
                    || empty_state("Gone", "This stack was removed.", theme).into_any_element(),
                    |stack| scroll_page(stack_info(stack, theme)).into_any_element(),
                )
            }
            (Some(selection), tab) => self.live.view().map_or_else(
                || {
                    let (title, detail) = live_placeholder(selection, tab, store);
                    empty_state(title, detail, theme).into_any_element()
                },
                gpui::AnyView::into_any_element,
            ),
        };

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .rounded_tr(px(self.right_radius))
            .rounded_br(px(self.right_radius))
            .child(chrome::drag_region(
                "detail-titlebar",
                div()
                    .h(px(Theme::HEADER_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(120.0))
                    .child(tabs),
            ))
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl Render for ContainersScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let want = self.live_key(self.store.read(cx));
        let mut live = std::mem::take(&mut self.live);
        live.sync(want, window, cx);
        self.live = live;
        let store = self.store.read(cx);
        let filtered: Vec<Container>;
        let containers = if self.query.is_empty() {
            store.containers()
        } else {
            filtered = store
                .containers()
                .iter()
                .filter(|c| matches_query(c, &self.query))
                .cloned()
                .collect();
            &filtered
        };
        let tree = ContainerTree::build(containers);
        let store = self.store.clone();
        let store = store.read(cx);
        self.sync_rows(&tree, store);
        let list = self.render_list(store, &tree, cx).into_any_element();
        let detail = self.render_detail(store, &tree, cx).into_any_element();
        div().size_full().flex().child(list).child(detail)
    }
}

/// Why a live tab shows nothing: stacks have no single stream, stopped
/// containers have nothing to sample or exec into.
fn live_placeholder(selection: &Selection, tab: usize, store: &Store) -> (&'static str, String) {
    let paused = match selection {
        Selection::Stack(_) => None,
        Selection::Container(id) => store
            .containers()
            .iter()
            .find(|c| &c.id == id)
            .map(|c| c.state == ContainerState::Paused),
    };
    match (tab, paused) {
        (4, _) => (
            "Files",
            "Browsing the container filesystem is planned.".into(),
        ),
        (_, None) if matches!(selection, Selection::Stack(_)) => (
            TABS.get(tab).copied().unwrap_or_default(),
            "Select a container in the stack to see this.".into(),
        ),
        (_, Some(true)) => ("Paused", "Unpause the container to continue.".into()),
        (_, Some(false)) => (
            "Not running",
            "Start the container to see live usage or open a shell.".into(),
        ),
        _ => ("Not connected", "Waiting for the engine…".into()),
    }
}

/// Case-insensitive match on name, image, compose project and service.
/// `query` is already lower-cased.
fn matches_query(c: &Container, query: &str) -> bool {
    let hit = |s: &str| s.to_lowercase().contains(query);
    hit(&c.name)
        || hit(&c.image)
        || c.compose
            .as_ref()
            .is_some_and(|cr| hit(&cr.project) || hit(&cr.service))
}

fn lifecycle_button(key: &str, active: bool) -> IconButton {
    IconButton::new(
        SharedString::from(format!("lifecycle-{key}")),
        if active { icons::STOP } else { icons::PLAY },
    )
}

fn delete_button(key: &str) -> IconButton {
    IconButton::new(SharedString::from(format!("delete-{key}")), icons::TRASH)
        .tone(ButtonTone::Danger)
}

const fn status_of(state: ContainerState) -> Status {
    match state {
        ContainerState::Running => Status::Running,
        ContainerState::Paused => Status::Paused,
        ContainerState::Restarting | ContainerState::Removing => Status::Transitioning,
        ContainerState::Dead => Status::Error,
        ContainerState::Created | ContainerState::Exited => Status::Stopped,
    }
}

/// `ghcr.io/berth/api:dev` → `api`: the part worth a monogram and a tint.
fn image_name(image: &str) -> String {
    let without_digest = image.split_once('@').map_or(image, |(name, _)| name);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split_once(':')
        .map_or(last, |(name, _)| name)
        .to_string()
}

fn scroll_page(content: impl IntoElement) -> impl IntoElement {
    div()
        .id("detail-scroll")
        .size_full()
        .overflow_y_scroll()
        .child(
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

fn status_value(c: &Container, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(Theme::SPACE_SM))
        .child(status_dot(status_of(c.state), 7.0, None, theme))
        .child(div().text_color(theme.text).child(c.status.clone()))
}

fn container_info(c: &Container, theme: &Theme) -> impl IntoElement {
    let identity = KeyValueCard::new()
        .row("Name", Cell::plain(c.name.clone()))
        .row("ID", Cell::mono(c.short_id().to_string()))
        .row("Image", Cell::plain(c.image.clone()))
        .row_element("Status", status_value(c, theme));

    let mut network = KeyValueCard::new().row(
        "IP address",
        c.ip.as_ref()
            .map_or_else(|| Cell::muted("Not running"), |ip| Cell::mono(ip.clone())),
    );
    if let Some(compose) = &c.compose {
        network = network
            .row("Compose project", Cell::plain(compose.project.clone()))
            .row("Service", Cell::plain(compose.service.clone()));
    }

    let ports = DataTable::new([
        ("Host port", 1.0),
        ("Container port", 1.0),
        ("Protocol", 1.0),
    ])
    .empty("No published ports")
    .rows(c.ports.iter().map(|p| {
        vec![
            match p.host_port {
                Some(port) if c.state.is_active() => Cell::link(port.to_string()),
                Some(port) => Cell::plain(port.to_string()),
                None => Cell::muted("—"),
            },
            Cell::plain(p.container_port.to_string()),
            Cell::muted(p.protocol.label()),
        ]
    }));

    let mounts = DataTable::new([("Source", 1.2), ("Destination", 1.0)])
        .empty("No mounts")
        .rows(c.mounts.iter().map(|m| {
            vec![
                match m.kind {
                    MountKind::Volume => Cell::link(m.source.clone()),
                    MountKind::Bind | MountKind::Tmpfs => Cell::plain(m.source.clone()),
                },
                Cell::plain(if m.read_only {
                    format!("{} (read-only)", m.destination)
                } else {
                    m.destination.clone()
                }),
            ]
        }));

    let labels = DataTable::new([("Key", 1.0), ("Value", 1.2)])
        .empty("No labels")
        .rows(c.labels.iter().map(|(k, v)| {
            let key = if k.starts_with("com.docker.compose.") {
                Cell::muted(k.clone())
            } else {
                Cell::plain(k.clone())
            };
            vec![key, Cell::plain(v.clone())]
        }));

    div()
        .flex()
        .flex_col()
        .gap(px(Theme::SPACE_MD))
        .child(identity)
        .child(network)
        .child(div().child(Heading::new("Ports")).child(ports))
        .child(div().child(Heading::new("Mounts")).child(mounts))
        .child(div().child(Heading::new("Labels")).child(labels))
}

fn stack_info(stack: &Stack<'_>, theme: &Theme) -> impl IntoElement {
    let services = stack.containers.iter().enumerate().map(|(ix, c)| {
        let service = c
            .compose
            .as_ref()
            .map_or_else(|| c.name.clone(), |cr| cr.service.clone());
        div()
            .h(px(40.0))
            .px(px(Theme::SPACE_MD))
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM + 2.0))
            .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
            .child(status_dot(status_of(c.state), 7.0, None, theme))
            .child(
                div()
                    .text_color(theme.text)
                    .font_weight(FontWeight::MEDIUM)
                    .child(service),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_faint)
                    .child(c.image.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_muted)
                    .child(c.status.clone()),
            )
    });

    let card = || {
        div()
            .flex()
            .flex_col()
            .rounded(px(Theme::PANEL_RADIUS))
            .bg(theme.surface_card)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
    };

    div()
        .flex()
        .flex_col()
        .child(Heading::new("Services"))
        .child(card().children(services))
        .when_some(stack.working_dir(), |d, dir| {
            d.child(Heading::new("Project")).child(
                card().child(
                    div()
                        .id("open-project-dir")
                        .h(px(42.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_SM + 2.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.row_hover))
                        .child(
                            icon(icons::FOLDER)
                                .size(px(16.0))
                                .text_color(theme.text_muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme.text)
                                .child("Open in file manager"),
                        )
                        .child(
                            div()
                                .text_size(px(Theme::TEXT_SM))
                                .text_color(theme.text_faint)
                                .font_family(theme.font_mono.clone())
                                .child(dir.to_string()),
                        )
                        .child(
                            icon(icons::CHEVRON_RIGHT)
                                .size(px(14.0))
                                .text_color(theme.text_faint),
                        )
                        .on_click({
                            let dir = dir.to_string();
                            move |_, _, cx| cx.open_with_system(std::path::Path::new(&dir))
                        }),
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::image_name;

    #[test]
    fn image_names() {
        assert_eq!(image_name("postgres:17-alpine"), "postgres");
        assert_eq!(image_name("ghcr.io/berth/api:dev"), "api");
        assert_eq!(image_name("localhost:5000/app"), "app");
        assert_eq!(image_name("redis"), "redis");
        assert_eq!(image_name("ghcr.io/berth/api@sha256:abc"), "api");
        assert_eq!(image_name("ghcr.io/berth/api:dev@sha256:abc"), "api");
    }
}

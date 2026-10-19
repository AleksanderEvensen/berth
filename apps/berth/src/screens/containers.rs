//! Containers screen: list column (stacks + containers) and detail pane.
//!
//! Reads the [`Store`]; lifecycle actions go through it, destructive ones are
//! confirmed first via a [`ConfirmRequest`] that the shell presents.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use berth_docker::{
    Container, ContainerState, ContainerTree, Endpoint, MountKind, Node, PortBinding, Stack,
};
use berth_ui::SearchField;
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, ScrollPane, SectionLabel, SegmentedTabs, Spinner, Status, TabStop, Theme,
    empty_state, icon, icons, status_dot,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, FontWeight,
    IntoElement, KeyDownEvent, Pixels, Point, Render, SharedString, Subscription, Task, Window,
    anchored, deferred, div, list, prelude::*, px,
};
use gpui_base::input::{InputEvent, InputState};

use super::engine_help::Problem;
use super::{Chrome, ListOp, ListScreen, ShowEngineHelp, ShowVolume};
use crate::chrome;
use crate::dialogs::ConfirmRequest;
use crate::keymap::{MENU_CONTEXT, MenuCancel, MenuConfirm, MenuNext, MenuPrev};
use crate::live::{Live, LiveKey, LiveTab};
use crate::selection::MultiSelect;
use crate::store::{Connection, Op, Store};
use crate::virtual_rows::VirtualRows;

const TABS: [&str; 5] = ["Info", "Stats", "Logs", "Terminal", "Files"];

const STACK_DELETE: &str = "All of its containers are stopped and removed. Volumes are kept.";
const BULK_DELETE: &str = "Running containers are stopped first. Volumes are kept.";

const fn delete_message(active: bool) -> &'static str {
    if active {
        "The container is stopped and removed. Its volumes are kept."
    } else {
        "The container is removed. Its volumes are kept."
    }
}

/// One line of the (virtualized) container list. Descriptors only: rows are
/// built from the store when they scroll into view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Section { running: bool, count: usize },
    Stack { project: String },
    Container { id: String, depth: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Selection {
    Stack(String),
    Container(String),
}

/// Environment for the selected container. List calls don't report it, so
/// it is fetched with inspect unless the store already has it.
#[derive(Debug)]
enum Env {
    Loading,
    Failed(String),
    Loaded(Vec<String>),
}

/// [`Env`] for one container, and the engine session that produced it.
#[derive(Debug)]
struct FetchedEnv {
    id: String,
    generation: u64,
    env: Env,
}

/// Whether `id` needs (re)fetching. A container's config never changes, so
/// a loaded environment stays valid; failures retry on a new engine session.
fn needs_fetch(cached: Option<&FetchedEnv>, id: &str, generation: u64) -> bool {
    match cached {
        Some(f) if f.id == id => !matches!(f.env, Env::Loaded(_)) && f.generation != generation,
        _ => true,
    }
}

/// The open "which port?" menu: container id, where it was opened, the
/// item the arrow keys are on, and the focus to give back when it closes.
#[derive(Debug, Clone)]
struct PortMenu {
    id: String,
    position: Point<Pixels>,
    highlight: Option<usize>,
    previous_focus: Option<FocusHandle>,
}

pub struct ContainersScreen {
    store: Entity<Store>,
    /// Compose projects the user collapsed (stacks start expanded).
    collapsed: HashSet<String>,
    selection: MultiSelect<Selection>,
    /// Selectable stacks and containers in display order (for shift-ranges).
    order: Vec<Selection>,
    tab: usize,
    search: Option<(Entity<InputState>, Subscription)>,
    /// Lower-cased, trimmed search query; empty = no filter.
    query: String,
    chrome: Chrome,
    rows: VirtualRows<Row>,
    /// Compose project → member container ids, in service order.
    stack_members: HashMap<String, Vec<String>>,
    /// Container id → index into the store's container list.
    index: HashMap<String, usize>,
    /// The open Stats/Logs/Terminal/Files view, live only while shown.
    live: Live,
    /// Whether the shell is showing this screen.
    visible: bool,
    /// A container to scroll into view once the rows are next synced.
    reveal: Option<String>,
    /// Fetched environment of the selected container (Info tab).
    env: Option<FetchedEnv>,
    env_fetch: Option<Task<()>>,
    port_menu: Option<PortMenu>,
    /// Focused while the port menu is open, so its keys reach it.
    menu_focus: FocusHandle,
}

impl EventEmitter<ConfirmRequest> for ContainersScreen {}
impl EventEmitter<ShowVolume> for ContainersScreen {}
impl EventEmitter<ShowEngineHelp> for ContainersScreen {}

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
            selection: MultiSelect::default(),
            order: Vec::new(),
            tab: 0,
            search: None,
            query: String::new(),
            chrome: Chrome::default(),
            rows: VirtualRows::new(),
            stack_members: HashMap::new(),
            index: HashMap::new(),
            live: Live::default(),
            visible: true,
            reveal: None,
            env: None,
            env_fetch: None,
            port_menu: None,
            menu_focus: cx.focus_handle(),
        }
    }

    /// Close the port menu, returning focus to where it was if the menu
    /// still holds it.
    fn close_port_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.port_menu.take() {
            if self.menu_focus.is_focused(window)
                && let Some(previous) = menu.previous_focus
            {
                previous.focus(window, cx);
            }
            cx.notify();
        }
    }

    /// Fetch the environment when the Info tab shows a container the store
    /// only knows from a list call, or retry a failure on a new session.
    fn sync_env(&mut self, cx: &Context<Self>) {
        let store = self.store.read(cx);
        let id = match self.selection.single() {
            Some(Selection::Container(id)) if self.tab == 0 && self.visible => id.clone(),
            _ => {
                self.env = None;
                self.env_fetch = None;
                return;
            }
        };
        let known = store
            .containers()
            .iter()
            .find(|c| c.id == id)
            .is_none_or(|c| c.env.is_some());
        let generation = store.generation();
        if known || !needs_fetch(self.env.as_ref(), &id, generation) {
            return;
        }
        let Some(engine) = store.engine() else {
            self.env = Some(FetchedEnv {
                id,
                generation,
                env: Env::Failed("Not connected".into()),
            });
            self.env_fetch = None;
            return;
        };
        self.env = Some(FetchedEnv {
            id: id.clone(),
            generation,
            env: Env::Loading,
        });
        self.env_fetch = Some(cx.spawn(async move |this, cx| {
            let result = engine.inspect_container(&id).await;
            this.update(cx, |this, cx| {
                if let Some(fetched) = &mut this.env
                    && fetched.id == id
                    && fetched.generation == generation
                {
                    fetched.env = result.map_or_else(
                        |e| Env::Failed(e.to_string()),
                        |c| Env::Loaded(c.env.unwrap_or_default()),
                    );
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The environment to show for `c`: the store's when it has one.
    fn env_of<'a>(&'a self, c: &'a Container) -> Option<Result<&'a [String], &'a str>> {
        if let Some(env) = &c.env {
            return Some(Ok(env));
        }
        match &self.env {
            Some(FetchedEnv {
                id,
                env: Env::Loaded(env),
                ..
            }) if *id == c.id => Some(Ok(env)),
            Some(FetchedEnv {
                id,
                env: Env::Failed(error),
                ..
            }) if *id == c.id => Some(Err(error)),
            _ => None,
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
            4 => LiveTab::Files,
            _ => return None,
        };
        let Some(Selection::Container(id)) = self.selection.single().filter(|_| self.visible)
        else {
            return None;
        };
        let container = store.containers().iter().find(|c| &c.id == id)?;
        Some(LiveKey {
            tab,
            container: id.clone(),
            running: container.state == ContainerState::Running,
            starts: store.start_count(id),
            engine: store.engine()?,
            mounts: container.mounts.clone(),
        })
    }

    /// Rebuild row descriptors from the current tree and splice only the
    /// changed range into the list state, so the scroll position survives
    /// containers moving between sections.
    /// Returns whether any rows changed.
    fn sync_rows(&mut self, tree: &ContainerTree<'_>, store: &Store) -> bool {
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
        self.order = rows
            .iter()
            .filter_map(|row| match row {
                Row::Section { .. } => None,
                Row::Stack { project } => Some(Selection::Stack(project.clone())),
                Row::Container { id, .. } => Some(Selection::Container(id.clone())),
            })
            .collect();
        let changed = self.rows.sync(rows);
        self.stack_members = stack_members;
        self.index = store
            .containers()
            .iter()
            .enumerate()
            .map(|(ix, c)| (c.id.clone(), ix))
            .collect();
        changed
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
    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
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
        let ids: HashSet<&str> = containers.iter().map(|c| c.id.as_str()).collect();
        let projects: HashSet<&str> = containers
            .iter()
            .filter_map(|c| c.compose.as_ref().map(|cr| cr.project.as_str()))
            .collect();
        self.selection.retain(|selection| match selection {
            Selection::Container(id) => ids.contains(id.as_str()),
            Selection::Stack(project) => projects.contains(project.as_str()),
        });
    }

    /// The detail tab shown.
    pub const fn tab(&self) -> usize {
        self.tab
    }

    /// Focus the open terminal; returns whether one is open.
    pub fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.live.focus_terminal(window, cx)
    }

    /// Where focus goes back to when the port menu closes: kept from a menu
    /// already open, which itself has focus now.
    fn menu_return(&mut self, window: &Window, cx: &App) -> Option<FocusHandle> {
        match self.port_menu.take() {
            Some(open) => open.previous_focus,
            None => window.focused(cx),
        }
    }

    /// Show detail tab `tab` (Info, Stats, Logs, Terminal, Files).
    pub fn set_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab < TABS.len() && self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    /// The selected containers that have no operation in flight.
    fn actionable<'a>(&self, store: &'a Store) -> Vec<&'a Container> {
        self.selected_containers(store)
            .into_iter()
            .filter(|c| store.pending(&c.id).is_none())
            .collect()
    }

    /// Stop what runs in the selection; start everything otherwise, as a
    /// stack's row button does.
    pub fn toggle_running(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let containers = self.actionable(store);
        let running: Vec<String> = containers
            .iter()
            .filter(|c| c.state.is_active())
            .map(|c| c.id.clone())
            .collect();
        let (ids, op) = if running.is_empty() {
            (containers.iter().map(|c| c.id.clone()).collect(), Op::Start)
        } else {
            (running, Op::Stop)
        };
        if !ids.is_empty() {
            self.run(ids, op, cx);
        }
    }

    /// Restart the running containers in the selection.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let ids: Vec<String> = self
            .actionable(store)
            .iter()
            .filter(|c| c.state.is_active())
            .map(|c| c.id.clone())
            .collect();
        if !ids.is_empty() {
            self.run(ids, Op::Restart, cx);
        }
    }

    /// Open the selected container's published port, or with several, the
    /// menu listing them at its row.
    pub fn open_port(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Selection::Container(id)) = self.selection.single() else {
            return;
        };
        let id = id.clone();
        let store = self.store.read(cx);
        let Some(c) = store
            .containers()
            .iter()
            .find(|c| c.id == id && c.state.is_active())
        else {
            return;
        };
        let Some(engine) = endpoint(store) else {
            return;
        };
        let urls: Vec<String> = c
            .web_ports(engine)
            .iter()
            .filter_map(|p| p.url(engine))
            .collect();
        match urls.as_slice() {
            [] => {}
            [url] => cx.open_url(url),
            _ => {
                let position = self
                    .rows
                    .position(
                        |row| matches!(row, Row::Container { id: row_id, .. } if *row_id == id),
                    )
                    .and_then(|ix| self.rows.state.bounds_for_item(ix))
                    .map_or_else(
                        || self.rows.state.viewport_bounds().origin,
                        |row| row.bottom_left(),
                    );
                let previous_focus = self.menu_return(window, cx);
                self.port_menu = Some(PortMenu {
                    id,
                    position,
                    highlight: Some(0),
                    previous_focus,
                });
                window.focus(&self.menu_focus, cx);
                cx.notify();
            }
        }
    }

    /// Collapse (`false`) or expand the selected stack. Collapsing from a
    /// container in a stack selects the stack.
    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        match self.selection.single().cloned() {
            Some(Selection::Stack(project)) => {
                if expanded {
                    self.collapsed.remove(&project);
                } else {
                    self.collapsed.insert(project);
                }
            }
            Some(Selection::Container(id)) if !expanded => {
                let store = self.store.read(cx);
                let project = store
                    .containers()
                    .iter()
                    .find(|c| c.id == id)
                    .and_then(|c| c.compose.as_ref())
                    .map(|compose| compose.project.clone())
                    .filter(|project| self.stack_members.contains_key(project));
                if let Some(project) = project {
                    if let Some(ix) = self
                        .rows
                        .position(|row| matches!(row, Row::Stack { project: p } if *p == project))
                    {
                        self.rows.scroll_into_view(ix);
                    }
                    self.selection.select_one(Selection::Stack(project));
                }
            }
            _ => return,
        }
        cx.notify();
    }

    /// Confirm deleting the selection (Delete key), worded like the row
    /// and bulk buttons.
    fn delete_selected(&self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let containers = self.actionable(store);
        let ids: Vec<String> = containers.iter().map(|c| c.id.clone()).collect();
        let names: Vec<SharedString> = containers
            .iter()
            .map(|c| SharedString::from(c.name.clone()))
            .collect();
        let (title, message, items) = match (self.selection.single(), containers.as_slice()) {
            (_, []) => return,
            (Some(Selection::Stack(project)), _) => {
                (format!("Delete stack {project}?"), STACK_DELETE, names)
            }
            (Some(Selection::Container(_)), [c]) => (
                format!("Delete {}?", c.name),
                delete_message(c.state.is_active()),
                Vec::new(),
            ),
            _ => {
                let total = ids.len();
                (
                    format!("Delete {total} container{}?", plural(total)),
                    BULK_DELETE,
                    names,
                )
            }
        };
        self.confirm_delete(title, message, items, ids, cx);
    }

    fn move_menu(&mut self, back: bool, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let Some(menu) = &mut self.port_menu else {
            return;
        };
        let count = store
            .containers()
            .iter()
            .find(|c| c.id == menu.id)
            .zip(endpoint(store))
            .map_or(0, |(c, engine)| c.web_ports(engine).len());
        menu.highlight = menu_step(menu.highlight, count, back);
        cx.notify();
    }

    fn open_highlighted_port(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let url = self.port_menu.as_ref().and_then(|menu| {
            let c = store.containers().iter().find(|c| c.id == menu.id)?;
            let engine = endpoint(store)?;
            c.web_ports(engine).get(menu.highlight?)?.url(engine)
        });
        if let Some(url) = url {
            cx.open_url(&url);
            self.close_port_menu(window, cx);
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

    /// Select a container and scroll it into view on the Info tab, expanding
    /// its stack and dropping a search that would hide it.
    pub fn reveal(&mut self, id: &str, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let Some(container) = store.containers().iter().find(|c| c.id == id) else {
            return;
        };
        if let Some(compose) = &container.compose {
            self.collapsed.remove(&compose.project);
        }
        let hidden = !self.query.is_empty() && !matches_query(container, &self.query);
        if hidden {
            self.close_search(cx);
        }
        self.selection
            .select_one(Selection::Container(id.to_string()));
        self.tab = 0;
        self.reveal = Some(id.to_string());
        cx.notify();
    }

    fn click(&mut self, selection: &Selection, event: &ClickEvent, cx: &mut Context<Self>) {
        self.selection
            .click(selection, event.modifiers(), &self.order);
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
            .leading_inset(self.chrome.header_inset)
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
            );

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
            .rounded_tl(px(self.chrome.left_radius))
            .rounded_bl(px(self.chrome.left_radius))
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
            Connection::Disconnected { error, .. } => column
                .child(title("Can't reach the engine"))
                .child(detail(error.to_string()))
                .child(
                    div()
                        .pt(px(Theme::SPACE_SM))
                        .flex()
                        .gap(px(Theme::SPACE_SM))
                        .when(Problem::of(store.connection()).is_some(), |d| {
                            d.child(
                                Button::new("engine-help", "How to fix")
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ShowEngineHelp))),
                            )
                        })
                        .child(Button::new("reconnect", "Retry").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.store.update(cx, Store::reconnect);
                            },
                        ))),
                )
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
        let selected = self.selection.contains(&Selection::Stack(project.clone()));
        let cursor = self.selection.is_cursor(&Selection::Stack(project.clone()));
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
                    STACK_DELETE,
                    names.clone(),
                    all_ids.clone(),
                    cx,
                );
            })
        };
        let select = {
            let project = project.clone();
            cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.click(&Selection::Stack(project.clone()), event, cx);
            })
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
        .keyboard_cursor(cursor)
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
        let selected = self.selection.contains(&Selection::Container(id.clone()));
        let cursor = self.selection.is_cursor(&Selection::Container(id.clone()));
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
            let message = delete_message(active);
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
            cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.click(&Selection::Container(id.clone()), event, cx);
            })
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
            .keyboard_cursor(cursor)
            .dimmed(!active);

        let engine = endpoint(store);
        let web_ports = engine.map(|e| c.web_ports(e)).unwrap_or_default();
        if active && web_ports.len() > 1 {
            let id = id.clone();
            row = row.action(
                IconButton::new(SharedString::from(format!("open-{id}")), icons::LINK)
                    .tab_stop(false)
                    .active(self.port_menu.as_ref().is_some_and(|m| m.id == id))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let previous_focus = this.menu_return(window, cx);
                        this.port_menu = Some(PortMenu {
                            id: id.clone(),
                            position: event.position(),
                            highlight: None,
                            previous_focus,
                        });
                        window.focus(&this.menu_focus, cx);
                        cx.notify();
                    })),
            );
        } else if let Some(url) = engine
            .zip(web_ports.first())
            .and_then(|(e, p)| p.url(e))
            .filter(|_| active)
        {
            row = row.action(
                IconButton::new(SharedString::from(format!("open-{id}")), icons::LINK)
                    .tab_stop(false)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        if active {
            let ids = vec![id.clone()];
            row = row.action(
                IconButton::new(SharedString::from(format!("restart-{id}")), icons::RESTART)
                    .tab_stop(false)
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
            .on_select(cx.listener(|this, ix: &usize, _, cx| this.set_tab(*ix, cx)));

        let bulk = self.selection.len() > 1;
        let body: AnyElement = match (self.selection.single(), self.tab) {
            (None, _) if bulk => self.render_bulk(store, cx),
            (None, _) => empty_state(
                "No selection",
                "Select a container or compose stack to inspect it. \
                 Shift- or Ctrl-click to select several.",
                theme,
            )
            .into_any_element(),
            (Some(Selection::Container(id)), 0) => {
                store.containers().iter().find(|c| &c.id == id).map_or_else(
                    || empty_state("Gone", "This container was removed.", theme).into_any_element(),
                    |c| {
                        scroll_page(container_info(
                            c,
                            endpoint(store),
                            self.env_of(c),
                            theme,
                            cx,
                        ))
                        .into_any_element()
                    },
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
            .rounded_tr(px(self.chrome.right_radius))
            .rounded_br(px(self.chrome.right_radius))
            .child(chrome::drag_region(
                "detail-titlebar",
                div()
                    .h(px(Theme::HEADER_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(self.chrome.caption_clearance))
                    .when(!bulk, |d| d.child(tabs)),
            ))
            .child(div().flex_1().min_h_0().child(body))
    }

    /// Containers the selection covers: selected containers plus the
    /// members a selected stack shows (a search can hide some), in store
    /// order.
    fn selected_containers<'a>(&self, store: &'a Store) -> Vec<&'a Container> {
        let in_selected_stack = |c: &Container| {
            c.compose.as_ref().is_some_and(|cr| {
                self.selection
                    .contains(&Selection::Stack(cr.project.clone()))
                    && self
                        .stack_members
                        .get(&cr.project)
                        .is_some_and(|members| members.contains(&c.id))
            })
        };
        store
            .containers()
            .iter()
            .filter(|c| {
                self.selection.contains(&Selection::Container(c.id.clone())) || in_selected_stack(c)
            })
            .collect()
    }

    /// Detail pane for several selected rows: counts and bulk start, stop
    /// and delete.
    fn render_bulk(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let selected = self.selected_containers(store);
        // The store skips containers with an operation in flight, so leave
        // them out of the actions and report them as busy instead.
        let (busy, containers): (Vec<&Container>, Vec<&Container>) = selected
            .iter()
            .partition(|c| store.pending(&c.id).is_some());
        let (running, stopped): (Vec<&Container>, Vec<&Container>) =
            containers.iter().partition(|c| c.state.is_active());
        let ids =
            |list: &[&Container]| -> Vec<String> { list.iter().map(|c| c.id.clone()).collect() };
        let start_ids = ids(&stopped);
        let stop_ids = ids(&running);
        let delete_ids = ids(&containers);
        let names: Vec<SharedString> = containers
            .iter()
            .map(|c| SharedString::from(c.name.clone()))
            .collect();
        let total = containers.len();

        let mut actions = div().flex().gap(px(Theme::SPACE_SM));
        if !start_ids.is_empty() {
            let count = start_ids.len();
            actions = actions.child(
                Button::new("start-selected", format!("Start {count}"))
                    .icon(icons::PLAY)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run(start_ids.clone(), Op::Start, cx);
                    })),
            );
        }
        if !stop_ids.is_empty() {
            let count = stop_ids.len();
            actions = actions.child(
                Button::new("stop-selected", format!("Stop {count}"))
                    .icon(icons::STOP)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run(stop_ids.clone(), Op::Stop, cx);
                    })),
            );
        }
        if total > 0 {
            actions = actions.child(
                Button::new("delete-selected", format!("Delete {total}"))
                    .icon(icons::TRASH)
                    .solid()
                    .tone(ButtonTone::Danger)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_delete(
                            format!("Delete {total} container{}?", plural(total)),
                            BULK_DELETE,
                            names.clone(),
                            delete_ids.clone(),
                            cx,
                        );
                    })),
            );
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(Theme::SPACE_SM))
            .child(
                div()
                    .text_size(px(Theme::TEXT_LG + 2.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(format!("{} selected", self.selection.len())),
            )
            .child(
                div()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_faint)
                    .child(match busy.len() {
                        0 => format!(
                            "{total} container{} · {} running",
                            plural(total),
                            running.len()
                        ),
                        n => format!(
                            "{} container{} · {} running · {n} busy",
                            selected.len(),
                            plural(selected.len()),
                            running.len()
                        ),
                    }),
            )
            .child(div().pt(px(Theme::SPACE_MD)).child(actions))
            .into_any_element()
    }
}

impl ListScreen for ContainersScreen {
    fn list_op(&mut self, op: ListOp, cx: &mut Context<Self>) {
        if op == ListOp::Delete {
            self.delete_selected(cx);
        } else if let Some(key) = op.apply(&mut self.selection, &self.order)
            && let Some(ix) = self.rows.position(|row| match (row, &key) {
                (Row::Stack { project }, Selection::Stack(p)) => project == p,
                (Row::Container { id, .. }, Selection::Container(c)) => id == c,
                _ => false,
            })
        {
            self.rows.scroll_into_view(ix);
        }
        cx.notify();
    }
}

impl Render for ContainersScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let want = self.live_key(self.store.read(cx));
        let mut live = std::mem::take(&mut self.live);
        live.sync(want, window, cx);
        self.live = live;
        self.sync_env(cx);
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
        let rows_changed = self.sync_rows(&tree, store);
        if let Some(id) = self.reveal.take()
            && let Some(ix) = self
                .rows
                .position(|row| matches!(row, Row::Container { id: row_id, .. } if *row_id == id))
        {
            self.rows.reveal(ix, rows_changed);
        }
        let list = self.render_list(store, &tree, cx).into_any_element();
        let detail = self.render_detail(store, &tree, cx).into_any_element();
        let menu = self.port_menu.as_ref().and_then(|menu| {
            let c = store
                .containers()
                .iter()
                .find(|c| c.id == menu.id && c.state.is_active())?;
            Some(port_menu(c, endpoint(store)?, menu, &self.menu_focus, cx))
        });
        if menu.is_none() {
            // The container stopped or went away while the menu was open.
            self.close_port_menu(window, cx);
        }
        div()
            .size_full()
            .flex()
            .child(list)
            .child(detail)
            .children(menu)
    }
}

/// Every published web port of `c`, opened from the row link (B4).
fn port_menu(
    c: &Container,
    engine: &Endpoint,
    menu: &PortMenu,
    focus: &FocusHandle,
    cx: &Context<ContainersScreen>,
) -> AnyElement {
    let theme = Theme::of(cx);
    let items = c
        .web_ports(engine)
        .into_iter()
        .enumerate()
        .filter_map(|(ix, p)| {
            let url = p.url(engine)?;
            Some(
                div()
                    .id(("port-menu-item", ix))
                    .h(px(30.0))
                    .px(px(Theme::SPACE_SM))
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_LG))
                    .rounded(px(Theme::CONTROL_RADIUS))
                    .cursor_pointer()
                    .when(menu.highlight == Some(ix), |d| d.bg(theme.element_hover))
                    .hover(|s| s.bg(theme.element_hover))
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme.text)
                            .child(url.trim_start_matches("http://").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_SM))
                            .text_color(theme.text_faint)
                            .child(format!("{}/{}", p.container_port, p.protocol.label())),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.open_url(&url);
                        this.close_port_menu(window, cx);
                    })),
            )
        });
    deferred(
        anchored().position(menu.position).snap_to_window().child(
            div()
                .id("port-menu")
                .track_focus(focus)
                .key_context(MENU_CONTEXT)
                .occlude()
                .on_action(cx.listener(|this, _: &MenuPrev, _, cx| this.move_menu(true, cx)))
                .on_action(cx.listener(|this, _: &MenuNext, _, cx| this.move_menu(false, cx)))
                .on_action(cx.listener(|this, _: &MenuConfirm, window, cx| {
                    this.open_highlighted_port(window, cx);
                }))
                .on_action(cx.listener(|this, _: &MenuCancel, window, cx| {
                    this.close_port_menu(window, cx);
                }))
                .min_w(px(220.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .bg(theme.surface_overlay)
                .border_1()
                .border_color(theme.border)
                .rounded(px(Theme::PANEL_RADIUS))
                .shadow_lg()
                .text_size(px(Theme::TEXT_MD))
                .children(items)
                .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                    this.close_port_menu(window, cx);
                })),
        ),
    )
    .with_priority(1)
    .into_any_element()
}

/// The menu item an arrow key lands on, wrapping around `count` items;
/// `back` is up. With none highlighted, down starts at the top, up at the
/// bottom.
pub fn menu_step(from: Option<usize>, count: usize, back: bool) -> Option<usize> {
    let last = count.checked_sub(1)?;
    Some(match (from, back) {
        (None, false) => 0,
        (None, true) => last,
        (Some(ix), true) => ix.checked_sub(1).unwrap_or(last),
        (Some(ix), false) if ix >= last => 0,
        (Some(ix), false) => ix.saturating_add(1),
    })
}

/// Why a live tab shows nothing: stacks have no single stream, stopped
/// containers have nothing to sample, exec into or list.
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
        (_, None) if matches!(selection, Selection::Stack(_)) => (
            TABS.get(tab).copied().unwrap_or_default(),
            "Select a container in the stack to see this.".into(),
        ),
        (_, Some(true)) => ("Paused", "Unpause the container to continue.".into()),
        (4, Some(false)) => (
            "Not running",
            "Start the container to browse its files.".into(),
        ),
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

const fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn lifecycle_button(key: &str, active: bool) -> IconButton {
    IconButton::new(
        SharedString::from(format!("lifecycle-{key}")),
        if active { icons::STOP } else { icons::PLAY },
    )
    .tab_stop(false)
}

fn delete_button(key: &str) -> IconButton {
    IconButton::new(SharedString::from(format!("delete-{key}")), icons::TRASH)
        .tone(ButtonTone::Danger)
        .tab_stop(false)
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
pub fn image_name(image: &str) -> String {
    let without_digest = image.split_once('@').map_or(image, |(name, _)| name);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    last.split_once(':')
        .map_or(last, |(name, _)| name)
        .to_string()
}

fn scroll_page(content: impl IntoElement) -> impl IntoElement {
    ScrollPane::new(
        "detail-scroll",
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

/// The connected engine, whose host serves published ports.
const fn endpoint(store: &Store) -> Option<&Endpoint> {
    match store.connection() {
        Connection::Connected(info) => Some(&info.endpoint),
        _ => None,
    }
}

fn port_cell(c: &Container, p: &PortBinding, engine: Option<&Endpoint>) -> Cell {
    match (p.host_port, engine.and_then(|e| p.url(e))) {
        (Some(port), Some(url)) if c.state.is_active() => {
            Cell::link(port.to_string()).on_click(move |_, _, cx| cx.open_url(&url))
        }
        (Some(port), _) => Cell::plain(port.to_string()),
        (None, _) => Cell::muted("—"),
    }
}

fn container_info(
    c: &Container,
    engine: Option<&Endpoint>,
    env: Option<Result<&[String], &str>>,
    theme: &Theme,
    cx: &Context<ContainersScreen>,
) -> impl IntoElement {
    let command = c.command.clone().filter(|cmd| !cmd.is_empty());
    let identity = KeyValueCard::new()
        .row("Name", Cell::plain(c.name.clone()).copyable())
        .row(
            "ID",
            Cell::mono(c.short_id().to_string()).copy_value(c.id.clone()),
        )
        .row("Image", Cell::plain(c.image.clone()).copyable())
        .row_element("Status", status_value(c, theme))
        .row(
            "Command",
            command.map_or_else(|| Cell::muted("None"), |cmd| Cell::mono(cmd).copyable()),
        );

    let mut network = KeyValueCard::new().row(
        "IP address",
        c.ip.as_ref().map_or_else(
            || Cell::muted("Not running"),
            |ip| Cell::mono(ip.clone()).copyable(),
        ),
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
    .id("ports")
    .empty("No published ports")
    .rows(c.ports.iter().map(|p| {
        vec![
            port_cell(c, p, engine),
            Cell::plain(p.container_port.to_string()),
            Cell::muted(p.protocol.label()),
        ]
    }));

    let mounts = DataTable::new([("Source", 1.2), ("Destination", 1.0)])
        .id("mounts")
        .empty("No mounts")
        .rows(c.mounts.iter().map(|m| {
            vec![
                match m.kind {
                    MountKind::Volume => {
                        let name = m.source.clone();
                        Cell::link(m.source.clone())
                            .copyable()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ShowVolume(name.clone()));
                            }))
                    }
                    MountKind::Bind | MountKind::Tmpfs => Cell::plain(m.source.clone()).copyable(),
                },
                Cell::plain(if m.read_only {
                    format!("{} (read-only)", m.destination)
                } else {
                    m.destination.clone()
                })
                .copy_value(m.destination.clone()),
            ]
        }));

    let env_table = DataTable::new([("Key", 1.0), ("Value", 1.6)]).id("env");
    let env = match env {
        None => env_table.empty("Loading…"),
        Some(Err(error)) => env_table.empty(format!("Couldn't load the environment: {error}")),
        Some(Ok(env)) => env_table
            .empty("No environment variables")
            .rows(env.iter().map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                vec![
                    Cell::plain(key.to_string()).copyable(),
                    Cell::mono(value.to_string()).copyable(),
                ]
            })),
    };

    let labels = DataTable::new([("Key", 1.0), ("Value", 1.2)])
        .id("labels")
        .empty("No labels")
        .rows(c.labels.iter().map(|(k, v)| {
            let key = if k.starts_with("com.docker.compose.") {
                Cell::muted(k.clone()).copyable()
            } else {
                Cell::plain(k.clone()).copyable()
            };
            vec![key, Cell::plain(v.clone()).copyable()]
        }));

    div()
        .flex()
        .flex_col()
        .gap(px(Theme::SPACE_MD))
        .child(identity)
        .child(network)
        .child(div().child(Heading::new("Ports")).child(ports))
        .child(div().child(Heading::new("Mounts")).child(mounts))
        .child(div().child(Heading::new("Environment")).child(env))
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
                card().child(TabStop::new(
                    "open-project-dir",
                    div()
                        .id("open-project-dir")
                        .h(px(42.0))
                        .rounded(px(Theme::PANEL_RADIUS))
                        .border_1()
                        .border_color(gpui::transparent_black())
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
                )),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{image_name, menu_step};

    #[test]
    fn menu_steps_wrap() {
        assert_eq!(menu_step(None, 0, false), None);
        assert_eq!(menu_step(None, 3, false), Some(0));
        assert_eq!(menu_step(None, 3, true), Some(2));
        assert_eq!(menu_step(Some(2), 3, false), Some(0));
        assert_eq!(menu_step(Some(0), 3, true), Some(2));
        assert_eq!(menu_step(Some(1), 3, false), Some(2));
    }

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

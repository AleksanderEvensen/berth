//! Activity monitor (F1): live usage of every running container as a tree
//! of stacks → containers, sortable by column, over sparklines of the totals.
//!
//! Streams exist only while the screen is visible: one stats stream per
//! running container, dropped when it stops or the screen hides (NFR-P5).
//! Samples are recorded silently; the screen repaints once a second.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use berth_docker::{Container, ContainerState, Engine, StatsSample};
use berth_ui::{Badge, ListHeader, Theme, empty_state, icon, icons};
use gpui::{
    AnyElement, Context, Entity, FontWeight, IntoElement, Render, SharedString, Task,
    Transformation, Window, div, percentage, prelude::*, px,
};
use num_traits::ToPrimitive as _;

use super::Chrome;
use super::containers::image_name;
use crate::chrome;
use crate::format;
use crate::live::{HISTORY, card, chart_value, io_card, rate};
use crate::store::{Connection, Store};

/// Repaint and totals-sampling interval; matches the engine's stats cadence.
const TICK: Duration = Duration::from_secs(1);
const ROW_HEIGHT: f32 = 30.0;
const INDENT: f32 = 18.0;
const METRIC_WIDTH: f32 = 104.0;
/// Collapse key of the "Containers" root row.
const ROOT: &str = "root";
/// Delay before reopening a stats stream that ended while its container is
/// still running; doubles per failed attempt up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

fn next_backoff(backoff: Duration) -> Duration {
    backoff.saturating_mul(2).min(RETRY_MAX)
}

/// Resource usage at one moment: CPU and memory as reported, network and
/// disk as per-second rates between the last two samples.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Usage {
    cpu: f64,
    mem: u64,
    /// (rx, tx) bytes/s.
    net: (u64, u64),
    /// (read, write) bytes/s.
    disk: (u64, u64),
}

impl Usage {
    fn between(prev: Option<&StatsSample>, cur: &StatsSample) -> Self {
        let (net, disk) = prev.map_or_else(Default::default, |prev| {
            let elapsed = cur.time_nanos.saturating_sub(prev.time_nanos);
            (
                (
                    rate(prev.net_rx, cur.net_rx, elapsed),
                    rate(prev.net_tx, cur.net_tx, elapsed),
                ),
                (
                    rate(prev.blk_read, cur.blk_read, elapsed),
                    rate(prev.blk_write, cur.blk_write, elapsed),
                ),
            )
        });
        Self {
            cpu: cur.cpu_percent,
            mem: cur.mem_used,
            net,
            disk,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            cpu: self.cpu + other.cpu,
            mem: self.mem.saturating_add(other.mem),
            net: (
                self.net.0.saturating_add(other.net.0),
                self.net.1.saturating_add(other.net.1),
            ),
            disk: (
                self.disk.0.saturating_add(other.disk.0),
                self.disk.1.saturating_add(other.disk.1),
            ),
        }
    }

    fn sum<'a>(usages: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        usages.into_iter().copied().reduce(Self::add)
    }

    fn cmp_by(&self, other: &Self, column: Column) -> Ordering {
        let both = |(a, b): (u64, u64)| a.saturating_add(b);
        match column {
            Column::Name => Ordering::Equal,
            Column::Cpu => self.cpu.total_cmp(&other.cpu),
            Column::Memory => self.mem.cmp(&other.mem),
            Column::Network => both(self.net).cmp(&both(other.net)),
            Column::Disk => both(self.disk).cmp(&both(other.disk)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    Cpu,
    Memory,
    Network,
    Disk,
}

impl Column {
    const METRICS: [Self; 4] = [Self::Cpu, Self::Memory, Self::Network, Self::Disk];

    const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Cpu => "CPU %",
            Self::Memory => "Memory",
            Self::Network => "Network",
            Self::Disk => "Disk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sort {
    column: Column,
    descending: bool,
}

impl Default for Sort {
    /// A→Z, so rows hold still until the user asks for a metric.
    fn default() -> Self {
        Self {
            column: Column::Name,
            descending: false,
        }
    }
}

impl Sort {
    /// Clicking the active column flips it; another column starts with the
    /// natural order (names A→Z, metrics largest first).
    fn toggle(self, column: Column) -> Self {
        if self.column == column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: column != Column::Name,
            }
        }
    }

    /// Order by the column (rows without a reading last when descending),
    /// then by name so equal readings don't shuffle between repaints.
    fn cmp(self, a: (&str, Option<&Usage>), b: (&str, Option<&Usage>)) -> Ordering {
        let primary = match (self.column, a.1, b.1) {
            (Column::Name, _, _) => a.0.cmp(b.0),
            (column, Some(x), Some(y)) => x.cmp_by(y, column),
            (_, x, y) => x.is_some().cmp(&y.is_some()),
        };
        let primary = if self.descending {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| a.0.cmp(b.0))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RowKind {
    Root,
    Stack,
    Container { image: String },
}

/// One line of the tree table.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    /// Collapse key for expandable rows; container id otherwise.
    key: String,
    label: String,
    depth: u8,
    kind: RowKind,
    /// `None` until the first sample arrives.
    usage: Option<Usage>,
    /// `Some(expanded)` for rows with children.
    expanded: Option<bool>,
}

/// A top-level entry under the root: a compose stack or a lone container.
enum Group<'a> {
    Stack {
        project: &'a str,
        members: Vec<&'a Container>,
        usage: Option<Usage>,
    },
    Single(&'a Container),
}

/// The visible rows for the running `containers`, sorted and collapsed.
fn build_rows(
    containers: &[Container],
    usage: &HashMap<String, Usage>,
    sort: Sort,
    collapsed: &HashSet<String>,
) -> Vec<Row> {
    let running: Vec<&Container> = containers
        .iter()
        .filter(|c| c.state == ContainerState::Running)
        .collect();
    if running.is_empty() {
        return Vec::new();
    }
    // Scaled services have several replicas; those keep their full names.
    let mut services: HashMap<(&str, &str), usize> = HashMap::new();
    for compose in running.iter().filter_map(|c| c.compose.as_ref()) {
        let count = services
            .entry((&compose.project, &compose.service))
            .or_default();
        *count = count.saturating_add(1);
    }
    let label = |c: &Container| match &c.compose {
        Some(compose) if services.get(&(&*compose.project, &*compose.service)) == Some(&1) => {
            compose.service.clone()
        }
        _ => c.name.clone(),
    };
    let by = |a: &Container, b: &Container| {
        sort.cmp((&label(a), usage.get(&a.id)), (&label(b), usage.get(&b.id)))
            .then_with(|| a.id.cmp(&b.id))
    };

    let mut stacks: BTreeMap<&str, Vec<&Container>> = BTreeMap::new();
    let mut groups = Vec::new();
    for c in &running {
        match &c.compose {
            Some(compose) => stacks.entry(&compose.project).or_default().push(c),
            None => groups.push(Group::Single(c)),
        }
    }
    for (project, mut members) in stacks {
        members.sort_by(|a, b| by(a, b));
        let usage = Usage::sum(members.iter().filter_map(|c| usage.get(&c.id)));
        groups.push(Group::Stack {
            project,
            members,
            usage,
        });
    }
    let key = |g: &Group<'_>| match g {
        Group::Stack { project, usage, .. } => (project.to_string(), *usage),
        Group::Single(c) => (c.name.clone(), usage.get(&c.id).copied()),
    };
    groups.sort_by(|a, b| {
        let (a, b) = (key(a), key(b));
        sort.cmp((&a.0, a.1.as_ref()), (&b.0, b.1.as_ref()))
    });

    let container_row = |c: &Container, depth: u8| Row {
        key: c.id.clone(),
        label: if depth > 1 { label(c) } else { c.name.clone() },
        depth,
        kind: RowKind::Container {
            image: c.image.clone(),
        },
        usage: usage.get(&c.id).copied(),
        expanded: None,
    };
    let root_open = !collapsed.contains(ROOT);
    let mut rows = vec![Row {
        key: ROOT.into(),
        label: "Containers".into(),
        depth: 0,
        kind: RowKind::Root,
        usage: Usage::sum(running.iter().filter_map(|c| usage.get(&c.id))),
        expanded: Some(root_open),
    }];
    if !root_open {
        return rows;
    }
    for group in groups {
        match group {
            Group::Single(c) => rows.push(container_row(c, 1)),
            Group::Stack {
                project,
                members,
                usage,
            } => {
                let key = format!("stack:{project}");
                let open = !collapsed.contains(&key);
                rows.push(Row {
                    key,
                    label: project.to_string(),
                    depth: 1,
                    kind: RowKind::Stack,
                    usage,
                    expanded: Some(open),
                });
                if open {
                    rows.extend(members.into_iter().map(|c| container_row(c, 2)));
                }
            }
        }
    }
    rows
}

/// One container's stats stream and its latest reading.
struct Feed {
    /// The start count the stream was opened for; a restart reopens it.
    starts: u64,
    last: Option<StatsSample>,
    usage: Option<Usage>,
    _stream: Task<()>,
}

pub struct ActivityScreen {
    store: Entity<Store>,
    chrome: Chrome,
    visible: bool,
    /// The engine the feeds stream from; a new session restarts them.
    engine: Option<Arc<dyn Engine>>,
    feeds: HashMap<String, Feed>,
    /// Totals sampled each tick, oldest first.
    history: VecDeque<Usage>,
    sort: Sort,
    collapsed: HashSet<String>,
    ticker: Option<Task<()>>,
}

impl ActivityScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |this, _, cx| {
            this.sync_feeds(cx);
            cx.notify();
        })
        .detach();
        Self {
            store,
            chrome: Chrome::default(),
            visible: false,
            engine: None,
            feeds: HashMap::new(),
            history: VecDeque::with_capacity(HISTORY.saturating_add(1)),
            sort: Sort::default(),
            collapsed: HashSet::new(),
            ticker: None,
        }
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
            cx.notify();
        }
    }

    /// The shell shows or hides this screen; every stream stops while hidden.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.sync_feeds(cx);
        cx.notify();
    }

    /// Open a stream for each running container and drop the rest; the
    /// ticker runs only while there is at least one stream.
    fn sync_feeds(&mut self, cx: &Context<Self>) {
        let store = self.store.read(cx);
        let engine = store.engine().filter(|_| self.visible);
        let same_engine = match (&self.engine, &engine) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_engine {
            self.feeds.clear();
            self.history.clear();
            self.engine.clone_from(&engine);
        }
        let running: HashMap<String, u64> = match &engine {
            Some(_) => store
                .containers()
                .iter()
                .filter(|c| c.state == ContainerState::Running)
                .map(|c| (c.id.clone(), store.start_count(&c.id)))
                .collect(),
            None => HashMap::new(),
        };
        self.feeds
            .retain(|id, feed| running.get(id) == Some(&feed.starts));
        if let Some(engine) = engine {
            for (id, starts) in running {
                self.feeds
                    .entry(id.clone())
                    .or_insert_with(|| Self::open_feed(engine.clone(), id, starts, cx));
            }
        }
        self.sync_ticker(cx);
    }

    fn sync_ticker(&mut self, cx: &Context<Self>) {
        if self.feeds.is_empty() {
            self.ticker = None;
            // The chart would show a gap as a flat line; start afresh.
            self.history.clear();
        } else if self.ticker.is_none() {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    let alive = this.update(cx, |this, cx| {
                        this.sample_totals();
                        cx.notify();
                    });
                    if alive.is_err() {
                        return;
                    }
                }
            }));
        }
    }

    /// Stream `id`'s stats for as long as the feed exists. A stream that
    /// ends or fails while the container still runs clears the reading (it
    /// is no longer live) and is reopened with backoff.
    fn open_feed(engine: Arc<dyn Engine>, id: String, starts: u64, cx: &Context<Self>) -> Feed {
        Feed {
            starts,
            last: None,
            usage: None,
            _stream: cx.spawn(async move |this, cx| {
                use futures::StreamExt as _;
                let mut backoff = RETRY_MIN;
                loop {
                    let mut stream = engine.stats(&id);
                    while let Some(Ok(sample)) = stream.next().await {
                        backoff = RETRY_MIN;
                        let alive = this.update(cx, |this, _| {
                            if let Some(feed) = this.feeds.get_mut(&id) {
                                feed.usage = Some(Usage::between(feed.last.as_ref(), &sample));
                                feed.last = Some(sample);
                            }
                        });
                        if alive.is_err() {
                            return;
                        }
                    }
                    let alive = this.update(cx, |this, cx| {
                        if let Some(feed) = this.feeds.get_mut(&id) {
                            feed.usage = None;
                            feed.last = None;
                        }
                        cx.notify();
                    });
                    if alive.is_err() {
                        return;
                    }
                    cx.background_executor().timer(backoff).await;
                    backoff = next_backoff(backoff);
                }
            }),
        }
    }

    fn usage(&self) -> HashMap<String, Usage> {
        self.feeds
            .iter()
            .filter_map(|(id, feed)| Some((id.clone(), feed.usage?)))
            .collect()
    }

    fn sample_totals(&mut self) {
        let total = Usage::sum(self.feeds.values().filter_map(|f| f.usage.as_ref()));
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(total.unwrap_or_default());
    }

    fn toggle_collapsed(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
        cx.notify();
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = self.sort.toggle(column);
        cx.notify();
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let column = |column: Column| {
            let active = self.sort.column == column;
            let arrow = icon(icons::ARROW_DOWN)
                .size(px(12.0))
                .text_color(theme.text_muted)
                .when(!self.sort.descending, |svg| {
                    svg.with_transformation(Transformation::rotate(percentage(0.5)))
                });
            div()
                .id(SharedString::from(format!("activity-sort-{column:?}")))
                .h_full()
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_XS))
                .px(px(Theme::SPACE_SM))
                .cursor_pointer()
                .hover(|s| s.text_color(theme.text))
                .text_color(if active { theme.text } else { theme.text_muted })
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .map(|d| {
                    let arrow = active.then_some(arrow);
                    // The arrow sits on the label's inner side.
                    if column == Column::Name {
                        d.flex_1().min_w_0().child(column.label()).children(arrow)
                    } else {
                        d.w(px(METRIC_WIDTH))
                            .flex_none()
                            .justify_end()
                            .children(arrow)
                            .child(column.label())
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx)))
        };
        div()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(Theme::TEXT_SM))
            .child(column(Column::Name))
            .children(Column::METRICS.map(column))
            .into_any_element()
    }

    fn render_row(row: Row, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let row_hover = theme.row_hover;
        let indent = f32::from(row.depth) * INDENT;
        let disclosure = row.expanded.map(|open| {
            let key = row.key.clone();
            div()
                .id(SharedString::from(format!("activity-toggle-{}", row.key)))
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(
                    icon(if open {
                        icons::CHEVRON_DOWN
                    } else {
                        icons::CHEVRON_RIGHT
                    })
                    .size(px(12.0))
                    .text_color(theme.text_faint),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_collapsed(&key, cx)))
        });
        let badge = match &row.kind {
            RowKind::Root => Badge::icon(ROOT, icons::BOX),
            RowKind::Stack => Badge::icon(row.label.clone(), icons::LAYERS),
            RowKind::Container { image } => Badge::monogram(image_name(image)),
        }
        .size(20.0);
        let metric = |text: String| {
            div()
                .w(px(METRIC_WIDTH))
                .flex_none()
                .px(px(Theme::SPACE_SM))
                .flex()
                .justify_end()
                .text_color(theme.text_muted)
                .child(text)
        };
        let per_second = |(a, b): (u64, u64)| format!("{}/s", format::bytes(a.saturating_add(b)));
        let cells = row.usage.map_or_else(
            || std::array::from_fn(|_| "—".to_string()),
            |u| {
                [
                    format!("{:.1}", u.cpu),
                    format::bytes(u.mem),
                    per_second(u.net),
                    per_second(u.disk),
                ]
            },
        );
        div()
            .id(SharedString::from(format!("activity-row-{}", row.key)))
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .hover(move |s| s.bg(row_hover))
            .text_size(px(Theme::TEXT_MD))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(Theme::SPACE_SM))
                    .pl(px(indent + Theme::SPACE_XS))
                    .child(div().w(px(16.0)).flex_none().children(disclosure))
                    .child(badge)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.text)
                            .when(row.kind == RowKind::Root, |d| {
                                d.font_weight(FontWeight::MEDIUM)
                            })
                            .child(row.label),
                    ),
            )
            .children(cells.map(metric))
            .into_any_element()
    }

    fn render_totals(&self, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let latest = self.history.back().copied().unwrap_or_default();
        let cpu = self
            .history
            .iter()
            .map(|u| u.cpu.to_f32().unwrap_or(0.0))
            .collect();
        let mem = self.history.iter().map(|u| chart_value(u.mem)).collect();
        let net: Vec<_> = self.history.iter().map(|u| u.net).collect();
        let disk: Vec<_> = self.history.iter().map(|u| u.disk).collect();
        let (primary, secondary) = (theme.accent, theme.success);
        let sparkline = |series| {
            berth_ui::Sparkline::new(HISTORY)
                .series(berth_ui::Series::new(series, primary))
                .into_any_element()
        };
        div()
            .flex_none()
            .flex()
            .gap(px(Theme::SPACE_MD))
            .px(px(Theme::SPACE_LG))
            .py(px(Theme::SPACE_LG))
            .border_t_1()
            .border_color(theme.border)
            .child(card(
                "Total CPU",
                format!("{:.1}%", latest.cpu),
                None,
                sparkline(cpu),
                theme,
            ))
            .child(card(
                "Memory",
                format::bytes(latest.mem),
                None,
                sparkline(mem),
                theme,
            ))
            .child(io_card(
                "Network",
                ("↓", "↑"),
                latest.net,
                &net,
                (primary, secondary),
                theme,
            ))
            .child(io_card(
                "Disk",
                ("R", "W"),
                latest.disk,
                &disk,
                (primary, secondary),
                theme,
            ))
            .into_any_element()
    }
}

impl Render for ActivityScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let store = self.store.read(cx);
        let rows = build_rows(
            store.containers(),
            &self.usage(),
            self.sort,
            &self.collapsed,
        );
        let running = store
            .containers()
            .iter()
            .filter(|c| c.state == ContainerState::Running)
            .count();
        let subtitle = match running {
            1 => "1 running container".to_string(),
            n => format!("{n} running containers"),
        };
        let empty = match store.connection() {
            Connection::Connected(_) if rows.is_empty() => Some((
                "No running containers",
                "Start a container to see its live usage.".to_string(),
            )),
            Connection::Connected(_) => None,
            Connection::Connecting => Some(("Connecting…", "Looking for an engine".to_string())),
            Connection::Disconnected { error, .. } => Some(("Not connected", error.to_string())),
        };

        let body = if let Some((title, detail)) = empty {
            empty_state(title, detail, theme).into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .px(px(Theme::SPACE_LG))
                        .child(self.render_header(cx))
                        .child(
                            div()
                                .id("activity-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .children(rows.into_iter().map(|row| Self::render_row(row, cx))),
                        ),
                )
                .child(self.render_totals(cx))
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .rounded_l(px(self.chrome.left_radius))
            .rounded_r(px(self.chrome.right_radius))
            .child(chrome::drag_region(
                "activity-titlebar",
                div().child(
                    ListHeader::new("Activity")
                        .subtitle(subtitle)
                        .leading_inset(self.chrome.header_inset),
                ),
            ))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_docker::ComposeRef;

    fn container(id: &str, project: Option<&str>, state: ContainerState) -> Container {
        Container {
            id: id.into(),
            name: id.into(),
            image: format!("{id}:latest"),
            state,
            status: String::new(),
            ip: None,
            ports: Vec::new(),
            mounts: Vec::new(),
            labels: Vec::new(),
            compose: project.map(|project| ComposeRef {
                project: project.into(),
                service: format!("{id}-svc"),
                working_dir: None,
            }),
            created: 0,
            command: None,
            env: None,
        }
    }

    fn cpu(cpu: f64) -> Usage {
        Usage {
            cpu,
            mem: 100,
            net: (1, 2),
            disk: (3, 4),
        }
    }

    fn sample(secs: i64, rx: u64, write: u64) -> StatsSample {
        StatsSample {
            time_nanos: secs.saturating_mul(1_000_000_000),
            cpu_percent: 5.0,
            online_cpus: 2,
            mem_used: 1024,
            mem_limit: 4096,
            net_rx: rx,
            net_tx: 0,
            blk_read: 0,
            blk_write: write,
        }
    }

    const BUSIEST: Sort = Sort {
        column: Column::Cpu,
        descending: true,
    };

    #[test]
    fn toggling_starts_metrics_largest_first_and_flips() {
        assert_eq!(Sort::default().toggle(Column::Cpu), BUSIEST);
        assert_eq!(
            Sort::default().toggle(Column::Name),
            Sort {
                column: Column::Name,
                descending: true
            }
        );
    }

    fn labels(rows: &[Row]) -> Vec<(u8, &str)> {
        rows.iter().map(|r| (r.depth, r.label.as_str())).collect()
    }

    fn fixture() -> (Vec<Container>, HashMap<String, Usage>) {
        let containers = vec![
            container("rustfs", None, ContainerState::Running),
            container("db", Some("voting"), ContainerState::Running),
            container("cache", Some("voting"), ContainerState::Running),
            container("old", None, ContainerState::Exited),
        ];
        let usage = [("rustfs", 2.0), ("db", 10.0), ("cache", 1.0)]
            .into_iter()
            .map(|(id, c)| (id.to_string(), cpu(c)))
            .collect();
        (containers, usage)
    }

    #[test]
    fn usage_rates_need_a_previous_sample() {
        let first = Usage::between(None, &sample(0, 1000, 0));
        assert_eq!((first.cpu, first.mem, first.net), (5.0, 1024, (0, 0)));
        let next = Usage::between(Some(&sample(0, 1000, 0)), &sample(2, 3000, 4096));
        assert_eq!(next.net, (1000, 0));
        assert_eq!(next.disk, (0, 2048));
    }

    #[test]
    fn tree_groups_stacks_and_sorts_busiest_first() {
        let (containers, usage) = fixture();
        let rows = build_rows(&containers, &usage, BUSIEST, &HashSet::new());
        assert_eq!(
            labels(&rows),
            [
                (0, "Containers"),
                (1, "voting"),
                (2, "db-svc"),
                (2, "cache-svc"),
                (1, "rustfs"),
            ]
        );
        let total = rows.first().and_then(|r| r.usage);
        assert_eq!(
            total.map(|u| (u.cpu, u.mem, u.net)),
            Some((13.0, 300, (3, 6)))
        );
        let stack = rows.get(1).and_then(|r| r.usage);
        assert_eq!(stack.map(|u| u.cpu), Some(11.0));
    }

    #[test]
    fn sorting_by_name_and_flipping() {
        let (containers, usage) = fixture();
        let by_name = Sort::default();
        assert_eq!(by_name, BUSIEST.toggle(Column::Name));
        let rows = build_rows(&containers, &usage, by_name, &HashSet::new());
        assert_eq!(
            labels(&rows),
            [
                (0, "Containers"),
                (1, "rustfs"),
                (1, "voting"),
                (2, "cache-svc"),
                (2, "db-svc"),
            ]
        );
        let least_cpu = BUSIEST.toggle(Column::Cpu);
        let rows = build_rows(&containers, &usage, least_cpu, &HashSet::new());
        assert_eq!(
            labels(&rows).get(1..),
            Some(
                &[
                    (1, "rustfs"),
                    (1, "voting"),
                    (2, "cache-svc"),
                    (2, "db-svc")
                ][..]
            )
        );
    }

    #[test]
    fn rows_without_readings_sort_last() {
        let (containers, mut usage) = fixture();
        usage.remove("db");
        usage.remove("cache");
        let rows = build_rows(&containers, &usage, BUSIEST, &HashSet::new());
        assert_eq!(rows.get(1).map(|r| r.label.as_str()), Some("rustfs"));
        assert_eq!(rows.get(2).map(|r| r.usage), Some(None));
    }

    #[test]
    fn collapsed_rows_hide_their_children() {
        let (containers, usage) = fixture();
        let stack: HashSet<String> = std::iter::once("stack:voting".to_string()).collect();
        let rows = build_rows(&containers, &usage, BUSIEST, &stack);
        assert_eq!(
            labels(&rows),
            [(0, "Containers"), (1, "voting"), (1, "rustfs")]
        );
        assert_eq!(rows.get(1).and_then(|r| r.expanded), Some(false));
        let root: HashSet<String> = std::iter::once(ROOT.to_string()).collect();
        assert_eq!(build_rows(&containers, &usage, BUSIEST, &root).len(), 1);
    }

    #[test]
    fn replicas_of_a_scaled_service_keep_their_names() {
        let mut containers = vec![
            container("voting-api-2", Some("voting"), ContainerState::Running),
            container("voting-api-1", Some("voting"), ContainerState::Running),
            container("db", Some("voting"), ContainerState::Running),
        ];
        for c in containers.iter_mut().take(2) {
            if let Some(compose) = c.compose.as_mut() {
                compose.service = "api".into();
            }
        }
        let rows = build_rows(
            &containers,
            &HashMap::new(),
            Sort::default(),
            &HashSet::new(),
        );
        assert_eq!(
            labels(&rows).get(2..),
            Some(&[(2, "db-svc"), (2, "voting-api-1"), (2, "voting-api-2")][..])
        );
    }

    #[test]
    fn retry_backoff_doubles_up_to_the_cap() {
        assert_eq!(next_backoff(RETRY_MIN), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(20)), RETRY_MAX);
        assert_eq!(next_backoff(RETRY_MAX), RETRY_MAX);
    }

    #[test]
    fn nothing_running_means_no_rows() {
        let containers = vec![container("old", None, ContainerState::Exited)];
        assert_eq!(
            build_rows(&containers, &HashMap::new(), BUSIEST, &HashSet::new()),
            Vec::<Row>::new()
        );
    }
}

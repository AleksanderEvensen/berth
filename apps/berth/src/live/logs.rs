//! Logs tab: a follow-tail log stream over a [`LogBuffer`], rendered as a
//! virtualized monospace list with ANSI colours (B6, FR-7, NFR-P4).
//!
//! Unwrapped lines all share one height, so a `uniform_list` shows them.
//! Wrapped lines vary in height and need a `gpui::list`, whose [`ListState`]
//! is spliced as the buffer changes, but only while wrapping is on.
//!
//! The stream lives exactly as long as this view; the containers screen
//! creates it when the tab becomes visible and drops it when it isn't
//! (NFR-P5). Arriving lines are applied in batches, at most once a frame.

use std::sync::Arc;
use std::time::Duration;

use berth_docker::{Engine, LogLine, LogOptions, LogStream};
use berth_ui::ansi::ansi_color;
use berth_ui::{IconButton, SearchField, Theme, icon, icons};
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, FollowMode, FontStyle, FontWeight, HighlightStyle,
    IntoElement, ListAlignment, ListOffset, ListState, Render, ScrollStrategy, ScrollWheelEvent,
    SharedString, StyledText, Subscription, Task, UnderlineStyle, UniformListScrollHandle, Window,
    div, list, prelude::*, px, uniform_list,
};
use gpui_base::input::{InputEvent, InputState};

use super::ansi::{SgrStyle, segments};
use super::log_buffer::{CAPACITY, LogBuffer, LogEntry, Needle};

const ROW_HEIGHT: f32 = 18.0;
/// Most lines applied per update; larger backlogs keep draining without
/// waiting for the next frame.
const BATCH: usize = 4096;
const FRAME: Duration = Duration::from_millis(16);
const FILTER_DEBOUNCE: Duration = Duration::from_millis(120);
/// History lines checked per frame while a new filter is applied.
const FILTER_SLICE: usize = 25_000;
/// History requested when the tab opens.
const TAIL: u32 = 10_000;

enum StreamState {
    Streaming,
    Ended,
    Failed(String),
}

pub struct LogsView {
    buffer: LogBuffer,
    scroll: UniformListScrollHandle,
    /// Pinned to the newest line; scrolling up pauses it.
    follow: bool,
    timestamps: bool,
    /// Long lines wrap onto several rows instead of being clipped.
    wrap: bool,
    /// Rows of the wrapped view; synced with the buffer only while `wrap`.
    lines: ListState,
    /// SGR state each stream (stdout, stderr) ended its last line in.
    styles: [SgrStyle; 2],
    search: Entity<InputState>,
    state: StreamState,
    _search: Subscription,
    _stream: Task<()>,
    filter_task: Option<Task<()>>,
}

impl LogsView {
    pub fn new(
        engine: &Arc<dyn Engine>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter logs"));
        let subscription = cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = state.read(cx).value().trim().to_string();
                this.schedule_filter(query, cx);
            }
        });
        let options = LogOptions {
            follow: true,
            tail: history_lines(),
            since: None,
        };
        let stream = engine.logs(id, options);
        Self {
            buffer: LogBuffer::new(CAPACITY),
            scroll: UniformListScrollHandle::new(),
            follow: true,
            timestamps: false,
            wrap: false,
            lines: ListState::new(0, ListAlignment::Top, px(400.0)),
            styles: [SgrStyle::default(); 2],
            search,
            state: StreamState::Streaming,
            _search: subscription,
            _stream: cx.spawn(async move |this, cx| {
                use futures::StreamExt as _;
                let mut batches = stream.ready_chunks(BATCH);
                while let Some(batch) = batches.next().await {
                    let full = batch.len() >= BATCH;
                    if this.update(cx, |view, cx| view.append(batch, cx)).is_err() {
                        return;
                    }
                    if !full {
                        cx.background_executor().timer(FRAME).await;
                    }
                }
                this.update(cx, |view, cx| {
                    if matches!(view.state, StreamState::Streaming) {
                        view.state = StreamState::Ended;
                    }
                    cx.notify();
                })
                .ok();
            }),
            filter_task: None,
        }
    }

    fn append(&mut self, batch: Vec<berth_docker::EngineResult<LogLine>>, cx: &mut Context<Self>) {
        let shown_before = self.buffer.len();
        let mut evicted = 0u32;
        for item in batch {
            match item {
                Ok(line) => {
                    let style = match line.stream {
                        LogStream::Stdout => &mut self.styles[0],
                        LogStream::Stderr => &mut self.styles[1],
                    };
                    if self.buffer.push(LogEntry::new(&line, style)) {
                        evicted = evicted.saturating_add(1);
                    }
                }
                Err(err) => self.state = StreamState::Failed(err.to_string()),
            }
        }
        if self.wrap {
            // Evicted rows leave the front and new ones join the end; the
            // list keeps the same lines under the reader by itself.
            let evicted = usize::try_from(evicted).unwrap_or(usize::MAX);
            if let Some(kept) = shown_before.checked_sub(evicted) {
                self.lines.splice(0..evicted, 0);
                self.lines
                    .splice(kept..kept, self.buffer.len().saturating_sub(kept));
            } else {
                self.lines.reset(self.buffer.len());
            }
            if self.follow {
                self.lines.set_follow_mode(FollowMode::Tail);
            }
        } else if self.follow {
            self.scroll.scroll_to_bottom();
        } else if evicted > 0 {
            // Rows moved up as the oldest were evicted: keep the same lines
            // under the reader instead of drifting with new output.
            let handle = &self.scroll.0.borrow().base_handle;
            let offset = handle.offset();
            let shift = f32::from(u16::try_from(evicted).unwrap_or(u16::MAX)) * ROW_HEIGHT;
            handle.set_offset(gpui::point(
                offset.x,
                px((f32::from(offset.y) + shift).min(0.0)),
            ));
        }
        cx.notify();
    }

    fn schedule_filter(&mut self, query: String, cx: &Context<Self>) {
        self.filter_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let started = this.update(cx, |view, cx| {
                view.buffer.set_filter(&query);
                if view.wrap {
                    view.lines.reset(view.buffer.len());
                }
                view.follow = true;
                view.scroll_to_end();
                cx.notify();
            });
            if started.is_err() {
                return;
            }
            // Scan the history a slice at a time so typing and rendering
            // stay responsive over a million lines.
            loop {
                let done = this.update(cx, |view, cx| {
                    let at = view.buffer.scanned_len();
                    let done = view.buffer.scan(FILTER_SLICE);
                    if view.wrap {
                        let found = view.buffer.scanned_len().saturating_sub(at);
                        view.lines.splice(at..at, found);
                    }
                    if view.follow {
                        view.scroll_to_end();
                    }
                    cx.notify();
                    done
                });
                if done.unwrap_or(true) {
                    return;
                }
                cx.background_executor().timer(FRAME).await;
            }
        }));
    }

    fn scroll_to_end(&self) {
        if self.wrap {
            self.lines.set_follow_mode(FollowMode::Tail);
        } else {
            self.scroll.scroll_to_bottom();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let dy = f32::from(event.delta.pixel_delta(px(ROW_HEIGHT)).y);
        let follow = if dy > 0.0 {
            false
        } else if self.wrap {
            // Resuming is picked up after layout; see `watch_tail`.
            self.follow
        } else {
            self.follow || self.scroll.is_scrolled_to_end().unwrap_or(true)
        };
        if follow != self.follow {
            self.follow = follow;
            cx.notify();
        }
    }

    /// The wrapped list resumes following by itself whenever a layout finds
    /// it at the bottom (scrolled back down, resized, or too short to
    /// scroll); pick that up after the next frame is laid out. Its
    /// unmeasured rows make `is_scrolled_to_end` unknown before then.
    fn watch_tail(window: &Window, cx: &Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, cx| {
                view.update(cx, |this, cx| {
                    if this.wrap && this.lines.is_following_tail() && !this.follow {
                        this.follow = true;
                        cx.notify();
                    }
                })
                .ok();
            });
        });
    }

    fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.follow = true;
        self.scroll_to_end();
        cx.notify();
    }

    /// Switch between clipped and wrapped lines, keeping the top line in
    /// view when not following.
    fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        if self.wrap {
            let offset = self.scroll.0.borrow().base_handle.offset();
            let top = usize::from(px((-f32::from(offset.y) / ROW_HEIGHT).max(0.0)));
            self.lines.reset(self.buffer.len());
            self.lines.set_follow_mode(FollowMode::Tail);
            if !self.follow {
                self.lines.scroll_to(ListOffset {
                    item_ix: top,
                    offset_in_item: px(0.0),
                });
            }
        } else {
            let top = self.lines.logical_scroll_top().item_ix;
            self.lines.reset(0);
            if self.follow {
                self.scroll.scroll_to_bottom();
            } else {
                self.scroll.scroll_to_item_strict(top, ScrollStrategy::Top);
            }
        }
        cx.notify();
    }

    fn copy(&self, cx: &Context<Self>) {
        let mut text = String::new();
        for entry in self.buffer.shown() {
            if self.timestamps
                && let Some(ts) = entry.timestamp
            {
                text.push_str(&format_timestamp(ts));
                text.push(' ');
            }
            text.push_str(&entry.text);
            text.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn render_rows(&self, range: std::ops::Range<usize>, cx: &Context<Self>) -> Vec<AnyElement> {
        let theme = Theme::of(cx);
        let needle = self.buffer.needle();
        range
            .filter_map(|ix| self.buffer.get(ix))
            .map(|entry| log_row(entry, needle, self.timestamps, false, theme))
            .collect()
    }

    fn render_wrapped_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        self.buffer.get(ix).map_or_else(
            || div().into_any_element(),
            |entry| log_row(entry, self.buffer.needle(), self.timestamps, true, theme),
        )
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let status = match &self.state {
            _ if self.buffer.scanning() => format!("Filtering… {}", count_label(&self.buffer)),
            StreamState::Streaming => count_label(&self.buffer),
            StreamState::Ended => format!("{} · ended", count_label(&self.buffer)),
            StreamState::Failed(err) => err.clone(),
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_XS))
            .px(px(Theme::SPACE_LG))
            .pb(px(Theme::SPACE_SM))
            .child(
                div()
                    .w(px(260.0))
                    .child(SearchField::new(&self.search).on_clear(cx.listener(
                        |this, _, window, cx| {
                            this.search
                                .update(cx, |state, cx| state.set_value("", window, cx));
                        },
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .px(px(Theme::SPACE_SM))
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(match self.state {
                        StreamState::Failed(_) => theme.danger,
                        _ => theme.text_faint,
                    })
                    .child(status),
            )
            .child(
                IconButton::new("logs-timestamps", icons::CLOCK)
                    .active(self.timestamps)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.timestamps = !this.timestamps;
                        if this.wrap {
                            // The timestamp column changes how lines wrap.
                            this.lines.remeasure();
                        }
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new("logs-wrap", icons::WRAP)
                    .active(self.wrap)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_wrap(cx))),
            )
            .child(
                IconButton::new("logs-copy", icons::COPY)
                    .disabled(self.buffer.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
            )
            .child(
                IconButton::new("logs-clear", icons::TRASH)
                    .disabled(self.buffer.total() == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.buffer.clear();
                        if this.wrap {
                            this.lines.reset(0);
                        }
                        this.follow = true;
                        cx.notify();
                    })),
            )
    }
}

impl Render for LogsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.wrap && !self.follow {
            Self::watch_tail(window, cx);
        }
        let theme = Theme::of(cx);
        let body: AnyElement = if self.buffer.is_empty() {
            let (title, detail) = match (&self.state, self.buffer.needle()) {
                (_, Some(_)) => ("No matches", "No line contains the filter text."),
                (StreamState::Streaming, None) => ("Waiting for output", "New lines appear here."),
                (_, None) => ("No logs", "This container hasn't written anything."),
            };
            berth_ui::empty_state(title, detail, theme).into_any_element()
        } else if self.wrap {
            list(
                self.lines.clone(),
                cx.processor(|this, ix: usize, _, cx| this.render_wrapped_row(ix, cx)),
            )
            .size_full()
            .pb(px(Theme::SPACE_SM))
            .into_any_element()
        } else {
            uniform_list(
                "log-lines",
                self.buffer.len(),
                cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .pb(px(Theme::SPACE_SM))
            .into_any_element()
        };
        let jump = (!self.follow).then(|| {
            div()
                .id("logs-jump")
                .absolute()
                .bottom(px(Theme::SPACE_LG))
                .right(px(Theme::SPACE_LG + 4.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(28.0))
                .px(px(Theme::SPACE_MD))
                .rounded_full()
                .font_family(theme.font_sans.clone())
                .bg(theme.surface_overlay)
                .border_1()
                .border_color(theme.border_strong)
                .shadow_md()
                .cursor_pointer()
                .hover(|s| s.bg(theme.surface_raised))
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text)
                .child(
                    icon(icons::ARROW_DOWN)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
                .child("Jump to latest")
                .on_click(cx.listener(|this, _, _, cx| this.jump_to_latest(cx)))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("logs-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .font_family(theme.font_mono.clone())
                    .text_size(px(Theme::TEXT_SM))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .child(body)
                    .children(jump),
            )
    }
}

fn log_row(
    entry: &LogEntry,
    needle: Option<&Needle>,
    timestamps: bool,
    wrap: bool,
    theme: &Theme,
) -> AnyElement {
    let text = SharedString::from(entry.text.to_string());
    let matches = needle.map(|n| n.ranges(&text)).unwrap_or_default();
    let highlights = segments(&entry.spans, &matches)
        .into_iter()
        .map(|(range, style, hit)| (range, highlight(style, hit, theme)));
    let stderr = entry.stream == LogStream::Stderr;
    div()
        .w_full()
        .flex()
        .map(|row| {
            if wrap {
                row.min_h(px(ROW_HEIGHT)).items_start()
            } else {
                row.h(px(ROW_HEIGHT))
                    .items_center()
                    .whitespace_nowrap()
                    .overflow_hidden()
            }
        })
        .pl(px(Theme::SPACE_LG - 2.0))
        .pr(px(Theme::SPACE_LG))
        .border_l_2()
        .border_color(if stderr {
            theme.danger.opacity(0.7)
        } else {
            gpui::transparent_black()
        })
        .hover(|s| s.bg(theme.row_hover))
        .when_some(entry.timestamp.filter(|_| timestamps), |row, ts| {
            row.child(
                div()
                    .flex_none()
                    .pr(px(Theme::SPACE_MD))
                    .text_color(theme.text_faint)
                    .child(format_timestamp(ts)),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(if stderr { theme.text } else { theme.text_muted })
                .child(StyledText::new(text).with_highlights(highlights)),
        )
        .into_any_element()
}

fn highlight(style: Option<SgrStyle>, hit: bool, theme: &Theme) -> HighlightStyle {
    let style = style.unwrap_or_default();
    let mut color = style.fg.map(|c| ansi_color(c, theme));
    if style.dim {
        color = Some(color.unwrap_or(theme.text_muted).opacity(0.6));
    }
    HighlightStyle {
        // Matched text keeps full contrast against the match wash.
        color: if hit { Some(theme.text) } else { color },
        background_color: if hit {
            Some(theme.warning.opacity(0.35))
        } else {
            style.bg.map(|c| ansi_color(c, theme))
        },
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        underline: style.underline.then_some(UnderlineStyle {
            thickness: px(1.0),
            ..UnderlineStyle::default()
        }),
        ..HighlightStyle::default()
    }
}

fn count_label(buffer: &LogBuffer) -> String {
    let total = buffer.total();
    let lines = |n: usize| if n == 1 { "line" } else { "lines" };
    if buffer.needle().is_some() {
        format!("{} of {total} {}", buffer.len(), lines(total))
    } else {
        format!("{total} {}", lines(total))
    }
}

/// Local wall-clock time with milliseconds.
fn format_timestamp(nanos: i64) -> String {
    chrono::DateTime::from_timestamp_nanos(nanos)
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S%.3f")
        .to_string()
}

/// `BERTH_LOG_TAIL` overrides how much history the tab requests: a line
/// count, or `all` (used with `BERTH_FIXTURE_LOG_LINES` for NFR-P4).
fn history_lines() -> Option<u32> {
    match std::env::var("BERTH_LOG_TAIL").ok().as_deref() {
        Some("all") => None,
        Some(n) => Some(n.parse().unwrap_or(TAIL)),
        None => Some(TAIL),
    }
}

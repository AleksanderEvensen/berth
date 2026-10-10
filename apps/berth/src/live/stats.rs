//! Stats tab: CPU, memory, network and disk sparklines (B7, FR-8).
//!
//! Samples arrive about once a second while the tab is visible; the last
//! [`HISTORY`] are kept. Network and disk counters are cumulative, so the
//! cards chart per-second rates between consecutive samples.

use std::collections::VecDeque;
use std::sync::Arc;

use berth_docker::{Engine, StatsSample};
use berth_ui::{ScrollPane, Series, Sparkline, Theme, empty_state};
use gpui::{
    AnyElement, Context, FontWeight, Hsla, IntoElement, Render, SharedString, Task, Window, div,
    prelude::*, px,
};
use num_traits::ToPrimitive as _;

use crate::format;

/// Samples kept and drawn: two minutes at the engine's 1 Hz.
pub const HISTORY: usize = 120;

enum StreamState {
    Waiting,
    Streaming,
    Ended,
    Failed(String),
}

pub struct StatsView {
    samples: VecDeque<StatsSample>,
    state: StreamState,
    _stream: Task<()>,
}

impl StatsView {
    pub fn new(engine: &Arc<dyn Engine>, id: &str, cx: &Context<Self>) -> Self {
        let stream = engine.stats(id);
        Self {
            samples: VecDeque::with_capacity(HISTORY.saturating_add(1)),
            state: StreamState::Waiting,
            _stream: cx.spawn(async move |this, cx| {
                use futures::StreamExt as _;
                let mut stream = stream;
                while let Some(item) = stream.next().await {
                    let alive = this.update(cx, |view, cx| {
                        match item {
                            Ok(sample) => view.push(sample),
                            Err(err) => view.state = StreamState::Failed(err.to_string()),
                        }
                        cx.notify();
                    });
                    if alive.is_err() {
                        return;
                    }
                }
                this.update(cx, |view, cx| {
                    if !matches!(view.state, StreamState::Failed(_)) {
                        view.state = StreamState::Ended;
                    }
                    cx.notify();
                })
                .ok();
            }),
        }
    }

    fn push(&mut self, sample: StatsSample) {
        self.state = StreamState::Streaming;
        if self.samples.len() > HISTORY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
}

impl Render for StatsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let Some(latest) = self.samples.back() else {
            let (title, detail) = match &self.state {
                StreamState::Failed(err) => ("Stats unavailable", err.clone()),
                StreamState::Ended => (
                    "Not running",
                    "Start the container to see live usage.".into(),
                ),
                StreamState::Waiting | StreamState::Streaming => (
                    "Waiting for samples",
                    "The engine reports usage about once a second.".into(),
                ),
            };
            return empty_state(title, detail, theme).into_any_element();
        };
        let series = History::from(&self.samples);
        let net = series.net.last().copied().unwrap_or_default();
        let disk = series.disk.last().copied().unwrap_or_default();
        let (primary, secondary) = (theme.accent, theme.success);
        let cards = [
            card(
                "CPU",
                format!("{:.1}%", latest.cpu_percent),
                Some(format!("{} CPUs", latest.online_cpus)),
                Sparkline::new(HISTORY)
                    .series(Series::new(series.cpu, primary))
                    .into_any_element(),
                theme,
            ),
            card(
                "Memory",
                format::bytes(latest.mem_used),
                Some(format!("of {}", format::bytes(latest.mem_limit))),
                Sparkline::new(HISTORY)
                    .series(Series::new(series.mem, primary))
                    .into_any_element(),
                theme,
            ),
            io_card(
                "Network",
                ("↓", "↑"),
                net,
                &series.net,
                (primary, secondary),
                theme,
            ),
            io_card(
                "Disk",
                ("R", "W"),
                disk,
                &series.disk,
                (primary, secondary),
                theme,
            ),
        ];
        let state = match &self.state {
            StreamState::Ended => Some("Stopped — showing the last samples.".to_string()),
            StreamState::Failed(err) => Some(err.clone()),
            StreamState::Waiting | StreamState::Streaming => None,
        };
        let mut cards = cards.into_iter();
        let mut pair = || {
            div()
                .flex()
                .gap(px(Theme::SPACE_MD))
                .children(cards.by_ref().take(2))
        };
        ScrollPane::new(
            "stats-scroll",
            div()
                .w_full()
                .max_w(px(880.0))
                .mx_auto()
                .px(px(Theme::SPACE_LG + 8.0))
                .pt(px(Theme::SPACE_SM))
                .pb(px(Theme::SPACE_LG * 2.0))
                .flex()
                .flex_col()
                .gap(px(Theme::SPACE_MD))
                .child(pair())
                .child(pair())
                .when_some(state, |d, state| {
                    d.child(
                        div()
                            .px(px(Theme::SPACE_XS))
                            .text_size(px(Theme::TEXT_SM))
                            .text_color(theme.text_faint)
                            .child(state),
                    )
                }),
        )
        .into_any_element()
    }
}

pub fn card(
    title: &'static str,
    value: String,
    detail: Option<String>,
    chart: AnyElement,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(Theme::SPACE_MD))
        .p(px(Theme::SPACE_LG))
        .rounded(px(Theme::PANEL_RADIUS))
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(Theme::SPACE_SM))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(Theme::TEXT_MD))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_muted)
                        .child(title),
                )
                .children(detail.map(|d| {
                    div()
                        .text_size(px(Theme::TEXT_SM))
                        .text_color(theme.text_faint)
                        .child(d)
                }))
                .child(
                    div()
                        .text_size(px(Theme::TEXT_LG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(SharedString::from(value)),
                ),
        )
        .child(chart)
        .into_any_element()
}

pub fn io_card(
    title: &'static str,
    (in_label, out_label): (&str, &str),
    (rate_in, rate_out): (u64, u64),
    rates: &[(u64, u64)],
    (in_color, out_color): (Hsla, Hsla),
    theme: &Theme,
) -> AnyElement {
    let ins = rates.iter().map(|r| chart_value(r.0)).collect();
    let outs = rates.iter().map(|r| chart_value(r.1)).collect();
    let rate = |n: u64| format!("{}/s", format::bytes(n));
    card(
        title,
        format!(
            "{in_label} {}  {out_label} {}",
            rate(rate_in),
            rate(rate_out)
        ),
        None,
        Sparkline::new(HISTORY)
            .series(Series::new(ins, in_color))
            .series(Series::new(outs, out_color))
            .into_any_element(),
        theme,
    )
}

/// Chart-ready series derived from raw samples.
#[derive(Debug, Default, PartialEq)]
struct History {
    cpu: Vec<f32>,
    mem: Vec<f32>,
    /// (rx, tx) bytes/s between consecutive samples.
    net: Vec<(u64, u64)>,
    /// (read, write) bytes/s.
    disk: Vec<(u64, u64)>,
}

impl From<&VecDeque<StatsSample>> for History {
    fn from(samples: &VecDeque<StatsSample>) -> Self {
        let mut history = Self {
            cpu: samples
                .iter()
                .map(|s| s.cpu_percent.to_f32().unwrap_or(0.0))
                .collect(),
            mem: samples.iter().map(|s| chart_value(s.mem_used)).collect(),
            ..Self::default()
        };
        for (prev, cur) in samples.iter().zip(samples.iter().skip(1)) {
            let elapsed = cur.time_nanos.saturating_sub(prev.time_nanos);
            history.net.push((
                rate(prev.net_rx, cur.net_rx, elapsed),
                rate(prev.net_tx, cur.net_tx, elapsed),
            ));
            history.disk.push((
                rate(prev.blk_read, cur.blk_read, elapsed),
                rate(prev.blk_write, cur.blk_write, elapsed),
            ));
        }
        history
    }
}

/// Bytes per second between two cumulative counter readings. A counter
/// that went backwards (container restart) or a non-positive interval
/// reads as zero.
pub fn rate(prev: u64, cur: u64, elapsed_nanos: i64) -> u64 {
    let Ok(elapsed) = u128::try_from(elapsed_nanos) else {
        return 0;
    };
    u128::from(cur.saturating_sub(prev))
        .saturating_mul(1_000_000_000)
        .checked_div(elapsed)
        .and_then(|r| u64::try_from(r).ok())
        .unwrap_or(0)
}

pub fn chart_value(n: u64) -> f32 {
    n.to_f32().unwrap_or(f32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(secs: i64, rx: u64, read: u64) -> StatsSample {
        StatsSample {
            time_nanos: secs.saturating_mul(1_000_000_000),
            cpu_percent: 12.5,
            online_cpus: 4,
            mem_used: 1024,
            mem_limit: 4096,
            net_rx: rx,
            net_tx: 0,
            blk_read: read,
            blk_write: 0,
        }
    }

    #[test]
    fn rates_between_samples() {
        assert_eq!(rate(1000, 3000, 2_000_000_000), 1000);
        assert_eq!(rate(3000, 1000, 1_000_000_000), 0, "counter reset");
        assert_eq!(rate(0, 1000, 0), 0, "no interval");
        assert_eq!(rate(0, 1000, -5), 0);
        assert_eq!(rate(0, 1, 1), 1_000_000_000);
    }

    #[test]
    fn history_derives_rates_and_values() {
        let samples: VecDeque<_> = [sample(0, 0, 0), sample(1, 500, 0), sample(3, 1500, 4096)]
            .into_iter()
            .collect();
        let history = History::from(&samples);
        assert_eq!(history.cpu, [12.5, 12.5, 12.5]);
        assert_eq!(history.mem, [1024.0, 1024.0, 1024.0]);
        assert_eq!(history.net, [(500, 0), (500, 0)]);
        assert_eq!(history.disk, [(0, 0), (2048, 0)]);
    }
}

//! Live container views — Stats, Logs, Terminal — and their lifetimes.
//!
//! At most one live view exists, for the visible tab of the selected
//! container. Each owns its engine stream, so dropping the view cancels the
//! stream: switching tab, container or screen leaves nothing running in the
//! background (NFR-P5).

mod ansi;
mod log_buffer;
mod logs;
mod stats;
mod terminal;

use std::sync::Arc;

use berth_docker::Engine;
use gpui::{AnyView, AppContext as _, Context, Entity, Window};

pub use logs::LogsView;
pub use stats::StatsView;
pub use stats::{HISTORY, card, chart_value, io_card, rate};
pub use terminal::ExecView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTab {
    Stats,
    Logs,
    Terminal,
}

/// What a live view was opened for; any change replaces the view.
#[derive(Clone)]
pub struct LiveKey {
    pub tab: LiveTab,
    pub container: String,
    /// Restart streams when the container starts or stops.
    pub running: bool,
    /// Also when it restarts faster than its state is observed to change.
    pub starts: u64,
    pub engine: Arc<dyn Engine>,
}

impl PartialEq for LiveKey {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab
            && self.container == other.container
            && self.running == other.running
            && self.starts == other.starts
            && Arc::ptr_eq(&self.engine, &other.engine)
    }
}

enum View {
    Stats(Entity<StatsView>),
    Logs(Entity<LogsView>),
    Terminal(Entity<ExecView>),
}

/// The currently open live view, if any.
#[derive(Default)]
pub struct Live {
    current: Option<(LiveKey, Option<View>)>,
}

impl Live {
    /// Make the open view match `want`, creating or dropping as needed.
    pub fn sync<T: 'static>(
        &mut self,
        want: Option<LiveKey>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) {
        if self.current.as_ref().map(|(key, _)| key) == want.as_ref() {
            return;
        }
        // Drop first: the old stream is cancelled before a new one opens.
        self.current = None;
        let Some(key) = want else { return };
        let engine = key.engine.clone();
        let id = key.container.clone();
        let view = match key.tab {
            LiveTab::Logs => Some(View::Logs(
                cx.new(|cx| LogsView::new(&engine, &id, window, cx)),
            )),
            // Stats and exec need a running container; the screen explains.
            _ if !key.running => None,
            LiveTab::Stats => Some(View::Stats(cx.new(|cx| StatsView::new(&engine, &id, cx)))),
            LiveTab::Terminal => Some(View::Terminal(
                cx.new(|cx| ExecView::new(engine, id, window, cx)),
            )),
        };
        self.current = Some((key, view));
    }

    pub fn view(&self) -> Option<AnyView> {
        let (_, view) = self.current.as_ref()?;
        view.as_ref().map(|view| match view {
            View::Stats(v) => v.clone().into(),
            View::Logs(v) => v.clone().into(),
            View::Terminal(v) => v.clone().into(),
        })
    }
}

//! Live container views — Stats, Logs, Terminal, Files — and their lifetimes.
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

use berth_docker::{Engine, FilesTarget, Mount, MountKind};
use gpui::{AnyView, App, AppContext as _, Context, Entity, SharedString, Window};

use crate::files::FilesTab;

pub use logs::LogsView;
pub use stats::StatsView;
pub use stats::{HISTORY, card, chart_value, io_card, rate};
pub use terminal::ExecView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTab {
    Stats,
    Logs,
    Terminal,
    Files,
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
    /// Marked in the Files tab at their mount points.
    pub mounts: Vec<Mount>,
}

impl PartialEq for LiveKey {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab
            && self.container == other.container
            && self.running == other.running
            && self.starts == other.starts
            && self.mounts == other.mounts
            && Arc::ptr_eq(&self.engine, &other.engine)
    }
}

enum View {
    Stats(Entity<StatsView>),
    Logs(Entity<LogsView>),
    Terminal(Entity<ExecView>),
    Files(Entity<FilesTab>),
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
            // Stats, exec and file listings need a running container; the
            // screen explains.
            _ if !key.running => None,
            LiveTab::Stats => Some(View::Stats(cx.new(|cx| StatsView::new(&engine, &id, cx)))),
            LiveTab::Terminal => Some(View::Terminal(
                cx.new(|cx| ExecView::new(engine, id, window, cx)),
            )),
            LiveTab::Files => Some(View::Files(cx.new(|cx| {
                let tags = key.mounts.iter().map(mount_tag).collect();
                FilesTab::new(engine, FilesTarget::Container(id), tags, cx)
            }))),
        };
        self.current = Some((key, view));
    }

    /// Focus the open terminal, if that is the view shown. Returns whether
    /// it was.
    pub fn focus_terminal(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some((_, Some(View::Terminal(view)))) = &self.current else {
            return false;
        };
        let focus = view.read(cx).terminal_focus(cx);
        window.focus(&focus, cx);
        true
    }

    pub fn view(&self) -> Option<AnyView> {
        let (_, view) = self.current.as_ref()?;
        view.as_ref().map(|view| match view {
            View::Stats(v) => v.clone().into(),
            View::Logs(v) => v.clone().into(),
            View::Terminal(v) => v.clone().into(),
            View::Files(v) => v.clone().into(),
        })
    }
}

/// The note on a mount point in the Files tab.
fn mount_tag(mount: &Mount) -> (String, SharedString) {
    let kind = match mount.kind {
        MountKind::Volume => format!("volume {}", mount.source),
        MountKind::Bind => format!("bind {}", mount.source),
        MountKind::Tmpfs => "tmpfs".into(),
    };
    let tag = if mount.read_only {
        format!("{kind} · read-only")
    } else {
        kind
    };
    (
        berth_docker::escape_name(mount.destination.as_bytes()),
        tag.into(),
    )
}

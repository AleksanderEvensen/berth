//! Terminal tab: an exec session with a TTY wired to the embedded terminal
//! (B8, FR-9). The session lives as long as this view; "Reconnect" starts a
//! fresh one after the shell exits.

use std::sync::Arc;

use berth_docker::{Engine, ExecOptions, TtyResizer, TtySize};
use berth_terminal::{GridSize, TerminalEvent, TerminalView};
use berth_ui::{Button, Theme};
use futures::channel::mpsc;
use futures::{SinkExt as _, StreamExt as _};
use gpui::{
    Context, Entity, Focusable as _, IntoElement, Render, Subscription, Task, Window, div,
    prelude::*, px,
};

enum Session {
    Starting,
    Running {
        input: mpsc::UnboundedSender<Vec<u8>>,
        resizer: TtyResizer,
    },
    Exited,
    Failed(String),
}

pub struct ExecView {
    engine: Arc<dyn Engine>,
    id: String,
    terminal: Entity<TerminalView>,
    session: Session,
    task: Task<()>,
    _events: Subscription,
}

impl ExecView {
    pub fn new(
        engine: Arc<dyn Engine>,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let terminal = cx.new(TerminalView::new);
        let events = cx.subscribe(&terminal, |this, _, event: &TerminalEvent, _| match event {
            TerminalEvent::Input(bytes) => {
                if let Session::Running { input, .. } = &this.session {
                    input.unbounded_send(bytes.clone()).ok();
                }
            }
            TerminalEvent::Resize(size) => {
                if let Session::Running { resizer, .. } = &this.session {
                    resizer.resize(tty_size(*size));
                }
            }
        });
        terminal.update(cx, |terminal, cx| {
            window.focus(&terminal.focus_handle(cx), cx);
        });
        let mut view = Self {
            engine,
            id,
            terminal,
            session: Session::Starting,
            task: Task::ready(()),
            _events: events,
        };
        view.start(cx);
        view
    }

    fn start(&mut self, cx: &Context<Self>) {
        self.session = Session::Starting;
        let size = tty_size(self.terminal.read(cx).size());
        let exec = self.engine.exec(&self.id, ExecOptions::shell(size));
        let terminal = self.terminal.clone();
        self.task = cx.spawn(async move |this, cx| {
            let session = match exec.await {
                Ok(session) => session,
                Err(err) => {
                    this.update(cx, |view, cx| {
                        view.session = Session::Failed(err.to_string());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            // Keystrokes queue here unbounded and drain in order into the
            // session's bounded stdin, so the UI never blocks on a slow shell.
            let (input, mut queued) = mpsc::unbounded::<Vec<u8>>();
            let mut stdin = session.stdin;
            let pump = cx.background_executor().spawn(async move {
                while let Some(bytes) = queued.next().await {
                    if stdin.send(bytes).await.is_err() {
                        break;
                    }
                }
            });
            let started = this.update(cx, |view, cx| {
                // The pane may have been laid out while the exec started.
                session
                    .resizer
                    .resize(tty_size(view.terminal.read(cx).size()));
                view.session = Session::Running {
                    input,
                    resizer: session.resizer,
                };
                cx.notify();
            });
            if started.is_err() {
                return;
            }
            let mut stdout = session.stdout;
            let mut failure = None;
            while let Some(chunk) = stdout.next().await {
                match chunk {
                    Ok(bytes) => terminal.update(cx, |t, cx| t.feed(&bytes, cx)),
                    Err(err) => {
                        failure = Some(err.to_string());
                        break;
                    }
                }
            }
            drop(pump);
            this.update(cx, |view, cx| {
                view.session = failure.map_or(Session::Exited, Session::Failed);
                cx.notify();
            })
            .ok();
        });
    }

    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.reset(cx);
            window.focus(&terminal.focus_handle(cx), cx);
        });
        self.start(cx);
        cx.notify();
    }
}

impl Render for ExecView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let banner = match &self.session {
            Session::Running { .. } => None,
            Session::Starting => Some(("Starting shell…".to_string(), false)),
            Session::Exited => Some(("Session ended.".to_string(), true)),
            Session::Failed(err) => Some((format!("Couldn't start a shell: {err}"), true)),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.terminal.clone()),
            )
            .when_some(banner, |d, (message, can_retry)| {
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_MD))
                        .h(px(44.0))
                        .px(px(Theme::SPACE_LG))
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.surface)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(Theme::TEXT_SM))
                                .text_color(theme.text_muted)
                                .child(message),
                        )
                        .when(can_retry, |d| {
                            d.child(Button::new("exec-reconnect", "Reconnect").on_click(
                                cx.listener(|this, _, window, cx| this.reconnect(window, cx)),
                            ))
                        }),
                )
            })
    }
}

const fn tty_size(size: GridSize) -> TtySize {
    TtySize {
        cols: size.cols,
        rows: size.rows,
    }
}

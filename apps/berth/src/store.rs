//! The app's view of the engine: an event-driven, in-memory model that
//! screens observe.
//!
//! Flow (docs/plan/03-architecture.md, "State flow"):
//! 1. Subscribe to engine events, then load `info` + the container list.
//! 2. Each container event triggers a targeted re-inspect of that container.
//! 3. User operations mark the container *pending* immediately; when the
//!    engine call returns the container is re-inspected and the mark cleared.
//!    Failures clear the mark and surface as a [`StoreEvent::Error`].
//! 4. When the event stream ends, the store reconnects with backoff and
//!    resyncs.
//!
//! Engine futures are executor-agnostic, so they are awaited directly on
//! gpui's foreground executor; a live engine does its I/O on its own runtime.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use berth_docker::{
    Container, ContainerAction, Engine, EngineError, EngineEvent, EngineInfo, EventAction,
    ObjectKind, RemoveOptions,
};
use futures::StreamExt as _;
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};

const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub enum Connection {
    Connecting,
    Connected(EngineInfo),
    Disconnected { error: String },
}

/// An operation in flight on a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Start,
    Stop,
    Restart,
    Remove,
}

impl Op {
    pub const fn progress_label(self) -> &'static str {
        match self {
            Self::Start => "Starting...",
            Self::Stop => "Stopping...",
            Self::Restart => "Restarting...",
            Self::Remove => "Removing...",
        }
    }

    const fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Remove => "remove",
        }
    }
}

/// Produces a connected engine. Runs on a background thread at the start of
/// every session, so a reconnect also re-runs endpoint discovery.
pub type Connector = Arc<dyn Fn() -> Result<Arc<dyn Engine>, String> + Send + Sync>;

pub enum StoreEvent {
    Error { title: String, detail: String },
}

pub struct Store {
    connector: Connector,
    /// The engine of the current session; `None` while disconnected.
    engine: Option<Arc<dyn Engine>>,
    connection: Connection,
    containers: Vec<Container>,
    pending: HashMap<String, Op>,
    session: Task<()>,
}

impl EventEmitter<StoreEvent> for Store {}

impl Store {
    pub fn new(connector: Connector, cx: &Context<Self>) -> Self {
        let session = Self::spawn_session(connector.clone(), cx);
        Self {
            connector,
            engine: None,
            connection: Connection::Connecting,
            containers: Vec::new(),
            pending: HashMap::new(),
            session,
        }
    }

    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn containers(&self) -> &[Container] {
        &self.containers
    }

    pub fn pending(&self, id: &str) -> Option<Op> {
        self.pending.get(id).copied()
    }

    pub fn running_count(&self) -> usize {
        self.containers
            .iter()
            .filter(|c| c.state.is_active())
            .count()
    }

    /// Drop the current session and connect again immediately.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.connection = Connection::Connecting;
        self.session = Self::spawn_session(self.connector.clone(), cx);
        cx.notify();
    }

    /// Run `op` on each container. Already-pending containers are skipped.
    pub fn run(&mut self, ids: Vec<String>, op: Op, cx: &mut Context<Self>) {
        let Some(engine) = self.engine.clone() else {
            cx.emit(StoreEvent::Error {
                title: format!("Couldn't {}", op.verb()),
                detail: "Not connected to a container engine.".into(),
            });
            return;
        };
        for id in ids {
            if self.pending.contains_key(&id) {
                continue;
            }
            self.pending.insert(id.clone(), op);
            let call = match op {
                Op::Start => engine.container_action(&id, ContainerAction::Start),
                Op::Stop => {
                    engine.container_action(&id, ContainerAction::Stop { timeout_secs: None })
                }
                Op::Restart => {
                    engine.container_action(&id, ContainerAction::Restart { timeout_secs: None })
                }
                // The UI confirms before removing, and says running
                // containers will be stopped — hence `force`.
                Op::Remove => engine.remove_container(
                    &id,
                    RemoveOptions {
                        force: true,
                        volumes: false,
                    },
                ),
            };
            let engine = engine.clone();
            cx.spawn(async move |this, cx| {
                let result = call.await;
                let refreshed = match (&result, op) {
                    (Ok(()), Op::Remove) | (Err(_), _) => None,
                    (Ok(()), _) => Some(engine.inspect_container(&id).await),
                };
                this.update(cx, |store, cx| {
                    store.pending.remove(&id);
                    match result {
                        Ok(()) if op == Op::Remove => store.forget(&id),
                        Ok(()) => {
                            if let Some(refreshed) = refreshed {
                                store.apply_inspect(&id, refreshed);
                            }
                        }
                        Err(err) => {
                            let name = store.display_name(&id);
                            cx.emit(StoreEvent::Error {
                                title: format!("Couldn't {} {name}", op.verb()),
                                detail: err.to_string(),
                            });
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn display_name(&self, id: &str) -> String {
        self.containers
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| id.chars().take(12).collect(), |c| c.name.clone())
    }

    fn forget(&mut self, id: &str) {
        self.containers.retain(|c| c.id != id);
        self.pending.remove(id);
    }

    fn upsert(&mut self, container: Container) {
        match self.containers.iter_mut().find(|c| c.id == container.id) {
            Some(slot) => *slot = container,
            None => self.containers.push(container),
        }
    }

    fn apply_inspect(&mut self, id: &str, result: Result<Container, EngineError>) {
        match result {
            Ok(container) => self.upsert(container),
            Err(EngineError::NotFound(_)) => self.forget(id),
            Err(err) => log::warn!("inspect {id} failed: {err}"),
        }
    }

    fn spawn_session(connector: Connector, cx: &Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut backoff = Duration::from_secs(1);
            loop {
                let connect = connector.clone();
                let connected = cx
                    .background_executor()
                    .spawn(async move { connect() })
                    .await;
                let end = match connected {
                    Err(error) => SessionEnd::Failed(error),
                    Ok(engine) => {
                        let installed = this.update(cx, |store, _| {
                            store.engine = Some(engine.clone());
                        });
                        if installed.is_err() {
                            return;
                        }
                        run_session(&engine, &this, cx).await
                    }
                };
                match end {
                    SessionEnd::Dropped => return,
                    SessionEnd::Failed(error) => {
                        let alive = this
                            .update(cx, |store, cx| {
                                store.engine = None;
                                store.connection = Connection::Disconnected { error };
                                cx.notify();
                            })
                            .is_ok();
                        if !alive {
                            return;
                        }
                        cx.background_executor().timer(backoff).await;
                        backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
                    }
                    SessionEnd::StreamClosed => {
                        // Was connected; retry promptly.
                        backoff = Duration::from_secs(1);
                        cx.background_executor().timer(backoff).await;
                    }
                }
            }
        })
    }
}

enum SessionEnd {
    /// The store was dropped.
    Dropped,
    /// Couldn't connect / sync.
    Failed(String),
    /// Connected, then the event stream ended.
    StreamClosed,
}

#[expect(
    clippy::future_not_send,
    reason = "GPUI AsyncApp stays on the foreground executor"
)]
async fn run_session(
    engine: &Arc<dyn Engine>,
    this: &WeakEntity<Store>,
    cx: &mut AsyncApp,
) -> SessionEnd {
    // Subscribe first so nothing that happens during the initial load is lost.
    let mut events = engine.events();
    let info = match engine.info().await {
        Ok(info) => info,
        Err(err) => return SessionEnd::Failed(err.to_string()),
    };
    let containers = match engine.list_containers().await {
        Ok(list) => list,
        Err(err) => return SessionEnd::Failed(err.to_string()),
    };
    let synced = this.update(cx, |store, cx| {
        store.connection = Connection::Connected(info);
        store.containers = containers;
        store.pending.clear();
        cx.notify();
    });
    if synced.is_err() {
        return SessionEnd::Dropped;
    }

    while let Some(event) = events.next().await {
        let event = match event {
            Ok(event) => event,
            Err(err) => {
                log::warn!("engine event stream error: {err}");
                break;
            }
        };
        if !handle_event(engine, this, cx, event).await {
            return SessionEnd::Dropped;
        }
    }
    SessionEnd::StreamClosed
}

/// Returns `false` once the store is gone.
#[expect(
    clippy::future_not_send,
    reason = "GPUI AsyncApp stays on the foreground executor"
)]
async fn handle_event(
    engine: &Arc<dyn Engine>,
    this: &WeakEntity<Store>,
    cx: &mut AsyncApp,
    event: EngineEvent,
) -> bool {
    if event.kind != ObjectKind::Container {
        return true;
    }
    if event.action == EventAction::Destroy {
        return this
            .update(cx, |store, cx| {
                store.forget(&event.id);
                cx.notify();
            })
            .is_ok();
    }
    let result = engine.inspect_container(&event.id).await;
    this.update(cx, |store, cx| {
        store.apply_inspect(&event.id, result);
        cx.notify();
    })
    .is_ok()
}

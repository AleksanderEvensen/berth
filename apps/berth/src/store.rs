//! The app's view of the engine: an event-driven, in-memory model that
//! screens observe.
//!
//! Flow:
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
    Container, ContainerAction, DiscoveryEnv, EndpointSource, Engine, EngineError, EngineEvent,
    EngineInfo, EventAction, ObjectKind, RemoveOptions,
};
use berth_settings::EngineChoice;
use futures::StreamExt as _;
use gpui::{AsyncApp, Context, EventEmitter, Task, WeakEntity};

mod networks;
mod storage;
use networks::Networks;
pub use storage::PruneTarget;
use storage::Storage;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub enum Connection {
    Connecting,
    Connected(EngineInfo),
    Disconnected {
        error: EngineError,
        /// Set when the endpoint was fixed rather than discovered: a socket
        /// that appears elsewhere won't be found until the pin changes.
        pin: Option<Pin>,
    },
}

/// What fixed the engine endpoint, bypassing socket discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pin {
    /// Settings → Engine names a context or a custom host.
    Settings,
    /// The `DOCKER_HOST` environment variable.
    DockerHost,
    /// The `DOCKER_CONTEXT` environment variable selects this context.
    ContextVar(String),
    /// The docker CLI's current context (`docker context use`).
    CurrentContext(String),
}

impl Pin {
    /// The pin behind `source`, discovered from `env` under `choice`.
    pub fn of(choice: &EngineChoice, env: &DiscoveryEnv, source: &EndpointSource) -> Option<Self> {
        match (choice, source) {
            (EngineChoice::Context { .. } | EngineChoice::Host { .. }, _) => Some(Self::Settings),
            (EngineChoice::Auto, EndpointSource::DockerHost) => Some(Self::DockerHost),
            (EngineChoice::Auto, EndpointSource::Context(name)) if env.docker_context.is_some() => {
                Some(Self::ContextVar(name.clone()))
            }
            (EngineChoice::Auto, EndpointSource::Context(name)) => {
                Some(Self::CurrentContext(name.clone()))
            }
            (EngineChoice::Auto, EndpointSource::Default(_)) => None,
        }
    }
}

/// A failed connect, with the pin that chose the endpoint if any.
#[derive(Debug, Clone)]
pub struct ConnectFailure {
    pub error: EngineError,
    pub pin: Option<Pin>,
}

impl From<EngineError> for ConnectFailure {
    fn from(error: EngineError) -> Self {
        Self { error, pin: None }
    }
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
pub type Connector = Arc<dyn Fn() -> Result<Arc<dyn Engine>, ConnectFailure> + Send + Sync>;

pub enum StoreEvent {
    Error { title: String, detail: String },
    Info { title: String, detail: String },
}

pub struct Store {
    connector: Connector,
    /// The engine of the current session; `None` while disconnected.
    engine: Option<Arc<dyn Engine>>,
    connection: Connection,
    containers: Vec<Container>,
    /// In-flight operations, keyed by container/image/network id or volume name.
    pending: HashMap<String, Op>,
    /// Start events seen per container, so views can tell a restart apart
    /// even when every inspect reports "running".
    starts: HashMap<String, u64>,
    storage: Storage,
    networks: Networks,
    /// Bumped whenever a different engine is installed. Async work captures
    /// it and drops its result if the engine changed meanwhile, so a slow
    /// reply from the old engine can't land in the new engine's state.
    generation: u64,
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
            starts: HashMap::new(),
            storage: Storage::default(),
            networks: Networks::default(),
            generation: 0,
            session,
        }
    }

    /// The connected engine, for views that open their own streams.
    pub fn engine(&self) -> Option<Arc<dyn Engine>> {
        self.engine.clone()
    }

    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn containers(&self) -> &[Container] {
        &self.containers
    }

    /// How many times the container has (re)started since the store saw it.
    pub fn start_count(&self, id: &str) -> u64 {
        self.starts.get(id).copied().unwrap_or_default()
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

    /// The engine target changed: forget everything learned from the old
    /// engine, then connect to the new one.
    pub fn switch_engine(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.engine = None;
        self.containers.clear();
        self.pending.clear();
        self.starts.clear();
        self.storage = Storage::default();
        self.networks = Networks::default();
        self.reconnect(cx);
    }

    /// Bumped whenever an engine is installed (connect, reconnect, switch).
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether async work started at `generation` still talks to the
    /// installed engine.
    const fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
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
            let generation = self.generation;
            cx.spawn(async move |this, cx| {
                let result = call.await;
                let refreshed = match (&result, op) {
                    (Ok(()), Op::Remove) | (Err(_), _) => None,
                    (Ok(()), _) => Some(engine.inspect_container(&id).await),
                };
                this.update(cx, |store, cx| {
                    if !store.is_current(generation) {
                        return;
                    }
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
        self.starts.remove(id);
    }

    fn upsert(&mut self, container: Container) {
        // Berth's own helpers (volume browsing) aren't the user's containers.
        if container.is_helper() {
            return;
        }
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
                            store.generation = store.generation.wrapping_add(1);
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
                                store.connection = Connection::Disconnected {
                                    error: error.error,
                                    pin: error.pin,
                                };
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
    Failed(ConnectFailure),
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
        Err(err) => return SessionEnd::Failed(err.into()),
    };
    let containers = match engine.list_containers().await {
        Ok(list) => list,
        Err(err) => return SessionEnd::Failed(err.into()),
    };
    let synced = this.update(cx, |store, cx| {
        store.connection = Connection::Connected(info);
        store.containers = containers.into_iter().filter(|c| !c.is_helper()).collect();
        store.pending.clear();
        store.storage.refresh_queued = false;
        store.refresh_storage(cx);
        store.networks.cancel_refresh();
        store.refresh_networks(cx);
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
    // Image/volume changes, and containers appearing or disappearing (which
    // changes usage), refresh the storage lists.
    let affects_storage = match event.kind {
        ObjectKind::Image | ObjectKind::Volume => true,
        ObjectKind::Container => matches!(event.action, EventAction::Create | EventAction::Destroy),
        ObjectKind::Network => false,
    };
    if affects_storage && this.update(cx, Store::refresh_storage).is_err() {
        return false;
    }
    // Endpoints come and go as containers start and stop.
    let affects_networks = match event.kind {
        ObjectKind::Network => true,
        ObjectKind::Container => matches!(
            event.action,
            EventAction::Start | EventAction::Die | EventAction::Restart | EventAction::Destroy
        ),
        ObjectKind::Image | ObjectKind::Volume => false,
    };
    if affects_networks && this.update(cx, Store::refresh_networks).is_err() {
        return false;
    }
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
    let started = matches!(event.action, EventAction::Start | EventAction::Restart);
    let result = engine.inspect_container(&event.id).await;
    this.update(cx, |store, cx| {
        if started {
            let count = store.starts.entry(event.id.clone()).or_default();
            *count = count.saturating_add(1);
        }
        store.apply_inspect(&event.id, result);
        cx.notify();
    })
    .is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use berth_docker::fixtures::FixtureEngine;
    use gpui::{AppContext as _, TestAppContext};

    use super::*;

    #[test]
    fn only_discovered_sockets_are_unpinned() {
        let auto = EngineChoice::Auto;
        let env = DiscoveryEnv::default();
        let rootful = EndpointSource::Default("rootful Docker");
        assert_eq!(Pin::of(&auto, &env, &rootful), None);
        assert_eq!(
            Pin::of(&auto, &env, &EndpointSource::DockerHost),
            Some(Pin::DockerHost)
        );
        let ci = EndpointSource::Context("ci".into());
        assert_eq!(
            Pin::of(&auto, &env, &ci),
            Some(Pin::CurrentContext("ci".into()))
        );
        let from_var = DiscoveryEnv {
            docker_context: Some("ci".into()),
            ..DiscoveryEnv::default()
        };
        assert_eq!(
            Pin::of(&auto, &from_var, &ci),
            Some(Pin::ContextVar("ci".into()))
        );
        // A choice in Settings wins, even over the variables it overrides.
        for choice in [
            EngineChoice::Host {
                host: "unix:///var/run/docker.sock".into(),
            },
            EngineChoice::Context { name: "ci".into() },
        ] {
            for source in [EndpointSource::DockerHost, ci.clone()] {
                assert_eq!(Pin::of(&choice, &from_var, &source), Some(Pin::Settings));
            }
        }
    }

    /// A slow reply from the old engine must not touch the new engine's
    /// state — here, a volume with the same name on both.
    #[gpui::test]
    fn stale_engine_results_are_dropped(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let slow: Arc<dyn Engine> =
            Arc::new(FixtureEngine::default().with_latency(Duration::from_millis(150)));
        let fast: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let current = Arc::new(Mutex::new(slow));
        let target = current.clone();
        let connector: Connector = Arc::new(move || {
            Ok(target
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone())
        });
        let store = cx.new(|cx| Store::new(connector, cx));
        let settle = |cx: &mut TestAppContext| {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
        };
        settle(cx);
        let volume = store.read_with(cx, |store, _| {
            assert!(matches!(store.connection(), Connection::Connected(_)));
            store
                .volumes()
                .iter()
                .find(|v| !store.volume_in_use(v))
                .map(|v| v.name.clone())
                .unwrap()
        });

        // Start a slow removal on the old engine, then switch.
        store.update(cx, |store, cx| {
            store.remove_volumes(vec![volume.clone()], cx);
        });
        *current.lock().unwrap() = fast;
        store.update(cx, Store::switch_engine);
        settle(cx);
        store.read_with(cx, |store, _| {
            assert!(store.volumes().iter().any(|v| v.name == volume));
            assert_eq!(store.pending(&volume), None);
        });

        // The old engine's reply arrives after the switch. Check before the
        // clock moves: a later refresh from the new engine would mask it.
        std::thread::sleep(Duration::from_millis(300));
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(
                store.volumes().iter().any(|v| v.name == volume),
                "a stale removal pruned the new engine's volume"
            );
            assert_eq!(store.pending(&volume), None);
        });
    }

    #[gpui::test]
    fn networks_follow_container_state_and_removals(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let connector: Connector = Arc::new(move || Ok(engine.clone()));
        let store = cx.new(|cx| Store::new(connector, cx));
        let settle = |cx: &mut TestAppContext| {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
        };
        let network =
            |store: &Store, name: &str| store.networks().iter().find(|n| n.name == name).cloned();
        settle(cx);
        let (rustfs, photon) = store.read_with(cx, |store, _| {
            assert!(store.networks_loaded());
            let bridge = network(store, "bridge").unwrap();
            assert_eq!(bridge.endpoints.len(), 1);
            let unused: Vec<_> = store.unused_networks().map(|n| n.name.as_str()).collect();
            assert_eq!(
                unused,
                [
                    "blue-zone-workshop_default",
                    "photon_dev_default",
                    "scratch_default"
                ]
            );
            assert_eq!(store.prune_count(PruneTarget::UnusedNetworks), Some(3));
            (
                bridge.endpoints[0].container_id.clone(),
                network(store, "photon_dev_default").unwrap().id,
            )
        });

        store.update(cx, |store, cx| {
            store.run(vec![rustfs], Op::Stop, cx);
            store.remove_networks(vec![photon.clone()], cx);
        });
        settle(cx);
        store.read_with(cx, |store, _| {
            assert_eq!(network(store, "bridge").unwrap().endpoints, []);
            assert!(network(store, "photon_dev_default").is_none());
            assert_eq!(store.pending(&photon), None);
        });
    }

    /// A change while a list is in flight must not be lost: the outdated
    /// reply is dropped and a follow-up refresh picks the change up.
    #[gpui::test]
    fn network_changes_during_a_refresh_trigger_a_follow_up(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let latency = Duration::from_millis(100);
        let fixture = Arc::new(FixtureEngine::default().with_list_latency(latency));
        let engine: Arc<dyn Engine> = fixture.clone();
        let connector: Connector = Arc::new(move || Ok(engine.clone()));
        let store = cx.new(|cx| Store::new(connector, cx));
        // Past the debounce, then until the delayed reply has landed.
        let settle = |cx: &mut TestAppContext| {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            std::thread::sleep(latency.saturating_mul(2));
            cx.run_until_parked();
        };
        settle(cx);
        let photon = store.read_with(cx, |store, _| {
            store
                .networks()
                .iter()
                .find(|n| n.name == "photon_dev_default")
                .map(|n| n.id.clone())
                .unwrap()
        });

        // Start a refresh and let it get past the debounce to the engine.
        store.update(cx, Store::refresh_networks);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        // Removed behind the store's back while that list is in flight.
        futures::executor::block_on(fixture.remove_network(&photon)).unwrap();
        settle(cx);
        settle(cx);
        store.read_with(cx, |store, _| {
            assert!(
                !store.networks().iter().any(|n| n.id == photon),
                "the removal's event was swallowed by the in-flight refresh"
            );
        });
    }
}

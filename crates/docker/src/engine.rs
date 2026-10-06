//! The contract between the UI and a container engine.
//!
//! Implementations: [`crate::fixtures::FixtureEngine`] (in-memory, for the UI
//! prototype and tests) and the bollard-backed live client (M1).
//!
//! Design rules (docs/plan/03-architecture.md):
//! - Methods return Berth's own model types, never client-library types.
//! - Futures and streams are `'static + Send` so the caller can drive them on
//!   any executor; implementations clone whatever handle they need.
//! - No runtime is assumed. An implementation that needs tokio owns its own
//!   runtime and must not require the caller to be inside one.

use std::fmt;
use std::path::PathBuf;

use futures::future::BoxFuture;
use futures::stream::BoxStream;

use crate::Container;

pub type EngineResult<T> = Result<T, EngineError>;

/// Where an engine API is reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `unix:///var/run/docker.sock`, rootless or Podman sockets.
    Unix(PathBuf),
    /// Windows named pipe, e.g. `//./pipe/docker_engine`.
    NamedPipe(String),
    /// `tcp://host:port` / `http(s)://…`. `tls` is set for 2376-style setups
    /// or when `DOCKER_TLS_VERIFY` asks for it.
    Tcp { url: String, tls: Option<TlsConfig> },
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
            Self::NamedPipe(pipe) => write!(f, "npipe://{pipe}"),
            Self::Tcp { url, .. } => f.write_str(url),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsConfig {
    /// Directory holding `ca.pem`, `cert.pem`, `key.pem` (`DOCKER_CERT_PATH`).
    pub cert_dir: PathBuf,
    pub verify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Docker,
    Podman,
    /// Anything else speaking the Docker API (`OrbStack`, Colima, …) or unknown.
    Other,
}

/// Identity of the connected engine — shown in the sidebar footer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInfo {
    pub kind: EngineKind,
    /// Human name, e.g. "Docker Engine", "Podman".
    pub name: String,
    pub version: String,
    pub api_version: String,
    pub os: String,
    pub arch: String,
    pub rootless: bool,
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerAction {
    Start,
    /// Graceful stop; `timeout_secs` before SIGKILL (`None` = engine default).
    Stop {
        timeout_secs: Option<u32>,
    },
    Restart {
        timeout_secs: Option<u32>,
    },
    Kill,
    Pause,
    Unpause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoveOptions {
    /// Kill a running container instead of failing.
    pub force: bool,
    /// Also remove anonymous volumes attached to the container.
    pub volumes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Container,
    Image,
    Volume,
    Network,
}

/// A change notification from the engine's `/events` stream.
///
/// Events only say *what* changed; consumers re-inspect the object to get its
/// new state. That keeps the event mapping small and the model authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEvent {
    pub kind: ObjectKind,
    /// Container/image/network ID, or volume name.
    pub id: String,
    pub action: EventAction,
    /// Unix time in nanoseconds, as reported by the engine.
    pub time_nanos: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAction {
    Create,
    Start,
    /// Process exited (Docker's `die`).
    Die,
    Stop,
    Kill,
    Pause,
    Unpause,
    Restart,
    Destroy,
    Rename,
    HealthStatus(String),
    /// Any other action, verbatim (`exec_start: sh`, `pull`, `mount`, …).
    Other(String),
}

/// Why connecting to an engine failed — specific enough to show a fix.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    #[error("no container engine found (tried: {tried})", tried = .tried.join(", "))]
    NotFound { tried: Vec<String> },
    #[error("permission denied on {endpoint}")]
    PermissionDenied { endpoint: String },
    #[error("{endpoint} refused the connection")]
    Refused { endpoint: String },
    #[error("invalid engine configuration: {0}")]
    InvalidConfig(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Connect(#[from] ConnectError),
    /// The object doesn't exist (HTTP 404).
    #[error("not found: {0}")]
    NotFound(String),
    /// The engine refused because of current state, e.g. removing a running
    /// container or an in-use volume (HTTP 409).
    #[error("{0}")]
    Conflict(String),
    /// Any other engine-reported failure; `message` is the engine's text.
    #[error("engine error {status}: {message}")]
    Engine { status: u16, message: String },
    /// Transport failure, stream closed, timeout.
    #[error("{0}")]
    Transport(String),
}

/// A container engine speaking (a dialect of) the Docker Engine API.
pub trait Engine: Send + Sync + 'static {
    fn info(&self) -> BoxFuture<'static, EngineResult<EngineInfo>>;

    /// All containers, running or not. List-level data: `env` is `None`.
    fn list_containers(&self) -> BoxFuture<'static, EngineResult<Vec<Container>>>;

    /// One container with full details (`env` populated).
    fn inspect_container(&self, id: &str) -> BoxFuture<'static, EngineResult<Container>>;

    fn container_action(
        &self,
        id: &str,
        action: ContainerAction,
    ) -> BoxFuture<'static, EngineResult<()>>;

    fn remove_container(
        &self,
        id: &str,
        options: RemoveOptions,
    ) -> BoxFuture<'static, EngineResult<()>>;

    /// Live change notifications. The stream ends on disconnect; the caller
    /// resyncs and resubscribes.
    fn events(&self) -> BoxStream<'static, EngineResult<EngineEvent>>;
}

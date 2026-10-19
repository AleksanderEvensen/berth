//! The contract between the UI and a container engine.
//!
//! Implementations: [`crate::fixtures::FixtureEngine`] (in-memory, for the UI
//! prototype and tests) and the bollard-backed live client (M1).
//!
//! Design rules:
//! - Methods return Berth's own model types, never client-library types.
//! - Futures and streams are `'static + Send` so the caller can drive them on
//!   any executor; implementations clone whatever handle they need.
//! - No runtime is assumed. An implementation that needs tokio owns its own
//!   runtime and must not require the caller to be inside one.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::stream::BoxStream;

use crate::{
    Container, DiskUsage, FileEntry, FileMount, FilesTarget, Image, ImageDetails, ImagePrune,
    Network, PruneReport, Volume,
};

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
    /// `ssh://[user@]host[:port]`: the system `ssh` client runs
    /// `docker system dial-stdio` there, so `destination` may be an alias
    /// from the user's ssh config.
    Ssh {
        /// `host` or `user@host`, passed to `ssh` as is (IPv6 unbracketed).
        destination: String,
        port: Option<u16>,
        /// The real host name from `ssh -G`, which the live client fills in
        /// on connect; published ports live there. `None` before connecting.
        hostname: Option<String>,
    },
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
            Self::NamedPipe(pipe) => write!(f, "npipe://{pipe}"),
            Self::Tcp { url, .. } => f.write_str(url),
            Self::Ssh {
                destination, port, ..
            } => {
                let (user, host) = destination
                    .rsplit_once('@')
                    .map_or(("", destination.as_str()), |(user, host)| (user, host));
                let at = if user.is_empty() { "" } else { "@" };
                let (open, close) = if host.contains(':') {
                    ("[", "]")
                } else {
                    ("", "")
                };
                write!(f, "ssh://{user}{at}{open}{host}{close}")?;
                port.map_or(Ok(()), |port| write!(f, ":{port}"))
            }
        }
    }
}

impl Endpoint {
    /// The engine's host when it runs on another machine, where its published
    /// ports live. Local sockets and loopback TCP give `None`.
    #[must_use]
    pub fn remote_host(&self) -> Option<&str> {
        let host = match self {
            Self::Tcp { url, .. } => {
                let authority = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
                let authority = authority.split(['/', '?']).next()?;
                match authority.strip_prefix('[') {
                    Some(v6) => v6.split(']').next()?,
                    None => authority
                        .rsplit_once(':')
                        .map_or(authority, |(host, _)| host),
                }
            }
            Self::Ssh {
                destination,
                hostname,
                ..
            } => hostname.as_deref().unwrap_or_else(|| {
                destination
                    .rsplit_once('@')
                    .map_or(destination.as_str(), |(_, host)| host)
            }),
            Self::Unix(_) | Self::NamedPipe(_) => return None,
        };
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        (!host.is_empty() && !loopback).then_some(host)
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

/// Which output stream a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One line of container output.
///
/// For non-TTY containers the engine multiplexes stdout and stderr into
/// frames; implementations demultiplex them and reassemble lines that span
/// frames. TTY containers have a single combined stream, reported as
/// [`LogStream::Stdout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub stream: LogStream,
    /// Unix time in nanoseconds, as stamped by the engine. `None` if the
    /// engine sent no (or an unparseable) timestamp.
    pub timestamp: Option<i64>,
    /// Text without the trailing `\n` (or `\r\n`). Invalid UTF-8 is replaced
    /// lossily. ANSI escape sequences are passed through untouched.
    pub line: String,
}

/// What to fetch from a container's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogOptions {
    /// Keep the stream open and deliver new lines as they are written.
    /// Without it the stream ends after the existing log.
    pub follow: bool,
    /// Only the last `n` existing lines; `None` = the whole log.
    pub tail: Option<u32>,
    /// Only lines written at or after this unix time (seconds).
    pub since: Option<i64>,
}

impl Default for LogOptions {
    /// Follow, starting from the last 1 000 lines.
    fn default() -> Self {
        Self {
            follow: true,
            tail: Some(1000),
            since: None,
        }
    }
}

/// One resource-usage sample, about once a second while streaming.
///
/// Network and block I/O are cumulative counters since the container
/// started; consumers derive rates from consecutive samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsSample {
    /// Unix time in nanoseconds when the engine took the sample.
    pub time_nanos: i64,
    /// Computed as the Docker CLI does: `Δcontainer_cpu / Δsystem_cpu ×
    /// online_cpus × 100`, so a container using two full cores reads 200.
    /// `0.0` when there is no previous reading to diff against.
    pub cpu_percent: f64,
    /// CPUs available to the container; `cpu_percent` tops out at
    /// `online_cpus × 100`.
    pub online_cpus: u32,
    /// Memory in use excluding page cache (Docker CLI semantics:
    /// `usage − inactive_file`).
    pub mem_used: u64,
    /// The cgroup memory limit, or host memory if unlimited.
    pub mem_limit: u64,
    /// Bytes received/sent, summed over all interfaces.
    pub net_rx: u64,
    pub net_tx: u64,
    /// Bytes read/written by block devices.
    pub blk_read: u64,
    pub blk_write: u64,
}

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtySize {
    pub cols: u16,
    pub rows: u16,
}

/// A process to run inside a container, attached to a TTY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOptions {
    /// Program and arguments. See [`ExecOptions::shell`].
    pub cmd: Vec<String>,
    /// Extra `KEY=value` pairs on top of the container's environment.
    pub env: Vec<String>,
    /// Defaults to the container's working directory.
    pub working_dir: Option<String>,
    /// Defaults to the container's user.
    pub user: Option<String>,
    /// Initial TTY size, applied before any output is produced.
    pub size: TtySize,
}

impl ExecOptions {
    /// An interactive login shell: the first available of `bash`, `sh` (B8).
    #[must_use]
    pub fn shell(size: TtySize) -> Self {
        let pick = "if command -v bash >/dev/null 2>&1; then exec bash; else exec sh; fi";
        Self {
            cmd: vec!["/bin/sh".into(), "-c".into(), pick.into()],
            env: vec!["TERM=xterm-256color".into(), "COLORTERM=truecolor".into()],
            working_dir: None,
            user: None,
            size,
        }
    }
}

/// Resizes the TTY of a running exec. Cheap to clone and non-blocking:
/// implementations coalesce bursts so only the latest size is sent.
#[derive(Clone)]
pub struct TtyResizer(Arc<dyn Fn(TtySize) + Send + Sync>);

impl TtyResizer {
    pub fn new(resize: impl Fn(TtySize) + Send + Sync + 'static) -> Self {
        Self(Arc::new(resize))
    }

    pub fn resize(&self, size: TtySize) {
        (self.0)(size);
    }
}

impl fmt::Debug for TtyResizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TtyResizer")
    }
}

/// A running exec with a TTY.
///
/// Dropping `stdout` cancels the session: the engine-side task stops and
/// the process loses its terminal. Dropping `stdin` sends EOF.
pub struct ExecSession {
    /// Raw bytes to the process (keystrokes, pastes). Bounded: a full
    /// channel applies backpressure to the sender.
    pub stdin: mpsc::Sender<Vec<u8>>,
    /// Raw TTY output (stdout and stderr combined, as a TTY does). Chunks
    /// are arbitrary; they may split UTF-8 and escape sequences. The stream
    /// ends when the process exits.
    pub stdout: BoxStream<'static, EngineResult<Vec<u8>>>,
    pub resizer: TtyResizer,
}

impl fmt::Debug for ExecSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecSession").finish_non_exhaustive()
    }
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
    /// `ssh` couldn't reach the remote engine; the message says why and
    /// what to do, e.g. trust the host key or install the docker CLI there.
    #[error("{0}")]
    Ssh(String),
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
    /// The object can't do what was asked, e.g. listing files in a
    /// container that has no shell.
    #[error("{0}")]
    Unsupported(String),
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

    /// Fast image list; container counts are included when the API reports them.
    fn list_images(&self) -> BoxFuture<'static, EngineResult<Vec<Image>>>;

    /// One image's config: platform, command, env, volumes, exposed ports.
    fn inspect_image(&self, id: &str) -> BoxFuture<'static, EngineResult<ImageDetails>>;

    /// Fast volume list, without size or reference counts.
    fn list_volumes(&self) -> BoxFuture<'static, EngineResult<Vec<Volume>>>;

    /// Potentially slow `/system/df` request with sizes and reference counts.
    fn disk_usage(&self) -> BoxFuture<'static, EngineResult<DiskUsage>>;

    fn remove_image(&self, id: &str, force: bool) -> BoxFuture<'static, EngineResult<()>>;
    fn remove_volume(&self, name: &str, force: bool) -> BoxFuture<'static, EngineResult<()>>;

    /// All networks with their connected (running) containers.
    fn list_networks(&self) -> BoxFuture<'static, EngineResult<Vec<Network>>>;
    /// Fails with a conflict while containers are connected.
    fn remove_network(&self, id: &str) -> BoxFuture<'static, EngineResult<()>>;

    /// Remove stopped containers.
    fn prune_containers(&self) -> BoxFuture<'static, EngineResult<PruneReport>>;
    fn prune_images(&self, mode: ImagePrune) -> BoxFuture<'static, EngineResult<PruneReport>>;
    /// Remove unused volumes, including named volumes.
    fn prune_volumes(&self) -> BoxFuture<'static, EngineResult<PruneReport>>;
    fn prune_networks(&self) -> BoxFuture<'static, EngineResult<PruneReport>>;
    fn prune_build_cache(&self) -> BoxFuture<'static, EngineResult<PruneReport>>;

    /// Live change notifications. The stream ends on disconnect; the caller
    /// resyncs and resubscribes.
    fn events(&self) -> BoxStream<'static, EngineResult<EngineEvent>>;

    // Live views (M3). Streams are produced on bounded channels, so a slow
    // consumer slows the producer rather than growing memory; dropping a
    // stream cancels its engine-side work.

    /// A container's log. With `follow` the stream stays open until the
    /// container stops or the stream is dropped. Unknown containers yield
    /// a single [`EngineError::NotFound`].
    fn logs(&self, id: &str, options: LogOptions) -> BoxStream<'static, EngineResult<LogLine>>;

    /// Resource usage samples for a running container until it stops or the
    /// stream is dropped.
    fn stats(&self, id: &str) -> BoxStream<'static, EngineResult<StatsSample>>;

    /// Start `options.cmd` in a running container with a TTY attached.
    fn exec(&self, id: &str, options: ExecOptions)
    -> BoxFuture<'static, EngineResult<ExecSession>>;

    // Files (B9, C5).

    /// Start a helper container that sees `target`'s files, pulling its
    /// image if needed. Nothing runs in a browsed container, so any image
    /// can be browsed, but only while it runs.
    fn open_files(&self, target: &FilesTarget) -> BoxFuture<'static, EngineResult<FileMount>>;

    /// The entries of the folder at real (link-free, see
    /// [`crate::resolve_path`]) path `path` of `mount`'s tree. One that
    /// can't be read is [`EngineError::NotFound`], with the reason.
    fn list_files(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<Vec<FileEntry>>>;

    /// The target (escaped) of the symlink at real path `path` of
    /// `mount`'s tree.
    fn read_link(&self, mount: &FileMount, path: &str) -> BoxFuture<'static, EngineResult<String>>;
}

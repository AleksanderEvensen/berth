#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
}

impl ContainerState {
    /// Running, paused and restarting containers all hold resources and sit
    /// in the "Running" section.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            ContainerState::Running | ContainerState::Paused | ContainerState::Restarting
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            ContainerState::Created => "Created",
            ContainerState::Running => "Running",
            ContainerState::Paused => "Paused",
            ContainerState::Restarting => "Restarting",
            ContainerState::Removing => "Removing",
            ContainerState::Exited => "Exited",
            ContainerState::Dead => "Dead",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Tcp,
    Udp,
    Sctp,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Tcp => "TCP",
            Protocol::Udp => "UDP",
            Protocol::Sctp => "SCTP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortBinding {
    pub host_ip: Option<String>,
    /// `None` when the port is exposed but not published.
    pub host_port: Option<u16>,
    pub container_port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKind {
    Volume,
    Bind,
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub kind: MountKind,
    /// Volume name for volumes, host path for binds.
    pub source: String,
    pub destination: String,
    pub read_only: bool,
}

/// Compose metadata, from the `com.docker.compose.*` labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeRef {
    pub project: String,
    pub service: String,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    /// Engine's human status, e.g. "Up 2 hours", "Exited (0) 3 days ago".
    pub status: String,
    pub ip: Option<String>,
    pub ports: Vec<PortBinding>,
    pub mounts: Vec<Mount>,
    pub labels: Vec<(String, String)>,
    pub compose: Option<ComposeRef>,
    /// Creation time, unix seconds.
    pub created: i64,
    /// Command line, as the engine reports it (entrypoint + args).
    pub command: Option<String>,
    /// `KEY=value` pairs. Only populated by inspect; `None` from list calls.
    pub env: Option<Vec<String>>,
}

impl Container {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(12)]
    }

    /// A browser URL for the container's most web-like published TCP port,
    /// preferring common HTTP ports. Wildcard bindings map to `localhost`.
    pub fn web_url(&self) -> Option<String> {
        let tcp = || {
            self.ports
                .iter()
                .filter(|p| p.protocol == Protocol::Tcp && p.host_port.is_some())
        };
        let binding = tcp()
            .find(|p| {
                matches!(
                    p.container_port,
                    80 | 443 | 3000 | 5173 | 8000 | 8025 | 8080 | 8443 | 9001
                )
            })
            .or_else(|| tcp().next())?;
        let port = binding.host_port?;
        let host = match binding.host_ip.as_deref() {
            None | Some("" | "0.0.0.0" | "::") => "localhost".to_string(),
            Some(ip) if ip.contains(':') => format!("[{ip}]"),
            Some(ip) => ip.to_string(),
        };
        let scheme = if matches!(binding.container_port, 443 | 8443) {
            "https"
        } else {
            "http"
        };
        Some(format!("{scheme}://{host}:{port}"))
    }
}

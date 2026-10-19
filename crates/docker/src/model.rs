use crate::{Endpoint, EngineKind};

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
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Restarting)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Restarting => "Restarting",
            Self::Removing => "Removing",
            Self::Exited => "Exited",
            Self::Dead => "Dead",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Protocol {
    Tcp,
    Udp,
    Sctp,
}

impl Protocol {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
            Self::Sctp => "SCTP",
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

impl PortBinding {
    /// Container ports that usually serve HTTP(S).
    const fn is_web_like(&self) -> bool {
        matches!(
            self.container_port,
            80 | 443 | 3000 | 5173 | 8000 | 8025 | 8080 | 8443 | 9001
        )
    }

    /// A browser URL for a published TCP port on `engine`'s host. Wildcard
    /// bindings map to that host; a remote engine's loopback bindings have no
    /// URL, being unreachable from this machine without a tunnel.
    #[must_use]
    pub fn url(&self, engine: &Endpoint) -> Option<String> {
        if self.protocol != Protocol::Tcp {
            return None;
        }
        let port = self.host_port?;
        let remote = engine.remote_host();
        let host = match (self.host_ip.as_deref(), remote) {
            (None | Some("" | "0.0.0.0" | "::"), None) => "localhost",
            (None | Some("" | "0.0.0.0" | "::"), Some(remote)) => remote,
            (Some(ip), Some(_))
                if ip
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()) =>
            {
                return None;
            }
            (Some(ip), _) => ip,
        };
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        let scheme = if matches!(self.container_port, 443 | 8443) {
            "https"
        } else {
            "http"
        };
        Some(format!("{scheme}://{host}:{port}"))
    }
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
    /// One of Berth's own helper containers (volume browsing).
    #[must_use]
    pub fn is_helper(&self) -> bool {
        self.labels
            .iter()
            .any(|(key, _)| key == crate::HELPER_LABEL)
    }

    #[must_use]
    pub fn short_id(&self) -> &str {
        self.id
            .get(..self.id.floor_char_boundary(self.id.len().min(12)))
            .unwrap_or(&self.id)
    }

    /// Published TCP ports that can be opened in a browser, most web-like
    /// first: common HTTP ports lead, the rest keep the engine's order.
    #[must_use]
    pub fn web_ports(&self, engine: &Endpoint) -> Vec<&PortBinding> {
        let mut ports: Vec<_> = self
            .ports
            .iter()
            .filter(|p| p.url(engine).is_some())
            .collect();
        ports.sort_by_key(|p| !p.is_web_like());
        ports
    }
}

/// A locally stored image, including references from stopped containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub size: u64,
    pub created: i64,
    /// Containers (running or not) using this image; unknown if not reported.
    pub containers: Option<u32>,
    /// Sorted by key.
    pub labels: Vec<(String, String)>,
}

impl Image {
    #[must_use]
    pub const fn is_dangling(&self) -> bool {
        self.repo_tags.is_empty()
    }

    #[must_use]
    pub fn short_id(&self) -> &str {
        let id = self.id.strip_prefix("sha256:").unwrap_or(&self.id);
        id.get(..id.floor_char_boundary(id.len().min(12)))
            .unwrap_or(id)
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        self.repo_tags
            .first()
            .cloned()
            .unwrap_or_else(|| self.short_id().to_owned())
    }
}

/// A port an image declares with `EXPOSE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExposedPort {
    pub port: u16,
    pub protocol: Protocol,
}

/// Image config from inspect — the parts the list call doesn't report (D4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImageDetails {
    pub id: String,
    /// `os/arch[/variant]`, e.g. `linux/arm64/v8`.
    pub platform: Option<String>,
    pub entrypoint: Vec<String>,
    pub command: Vec<String>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
    /// `KEY=value` pairs.
    pub env: Vec<String>,
    /// Paths declared with `VOLUME`, sorted.
    pub volumes: Vec<String>,
    /// Sorted by port, then protocol.
    pub exposed_ports: Vec<ExposedPort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    /// RFC3339 creation time reported by the engine.
    pub created: Option<String>,
    pub labels: Vec<(String, String)>,
    /// From `/system/df`; unknown until disk usage is fetched.
    pub size: Option<u64>,
    /// Containers referencing this volume; unknown if not reported.
    pub ref_count: Option<u32>,
}

impl Volume {
    #[must_use]
    pub fn compose_project(&self) -> Option<&str> {
        self.labels
            .iter()
            .find(|(key, _)| key == "com.docker.compose.project")
            .map(|(_, value)| value.as_str())
    }
}

/// One IPAM pool of a network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subnet {
    /// CIDR, e.g. `172.19.0.0/16`.
    pub subnet: String,
    pub gateway: Option<String>,
}

/// A container's endpoint on a network. Only running containers have one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEndpoint {
    pub container_id: String,
    pub name: String,
    /// CIDR as reported by the engine, e.g. `172.19.0.2/16`.
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

/// Special-purpose networks that `network prune` leaves alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkRole {
    #[default]
    Regular,
    /// Swarm routing-mesh network.
    Ingress,
    /// A template for other networks; never has endpoints of its own.
    ConfigOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: String,
    /// `local`, `swarm` or `global`.
    pub scope: String,
    /// RFC3339 creation time reported by the engine.
    pub created: Option<String>,
    pub internal: bool,
    pub attachable: bool,
    pub role: NetworkRole,
    pub ipv6: bool,
    pub subnets: Vec<Subnet>,
    /// Sorted by key.
    pub labels: Vec<(String, String)>,
    /// Sorted by container name.
    pub endpoints: Vec<NetworkEndpoint>,
}

impl Network {
    /// Networks the engine creates itself and refuses to remove.
    #[must_use]
    pub fn is_builtin(&self, engine: EngineKind) -> bool {
        match engine {
            EngineKind::Podman => matches!(self.name.as_str(), "podman" | "host" | "none"),
            EngineKind::Docker | EngineKind::Other => {
                matches!(self.name.as_str(), "bridge" | "host" | "none")
            }
        }
    }

    /// Swarm networks can be used by services on other nodes, which local
    /// endpoints don't show.
    #[must_use]
    pub fn is_swarm(&self) -> bool {
        self.scope == "swarm"
    }

    /// What `network prune` removes: custom, local networks without
    /// endpoints, excluding ingress and config-only networks (as Docker does).
    #[must_use]
    pub fn is_prunable(&self, engine: EngineKind) -> bool {
        self.endpoints.is_empty()
            && !self.is_builtin(engine)
            && !self.is_swarm()
            && self.role == NetworkRole::Regular
    }

    #[must_use]
    pub fn short_id(&self) -> &str {
        self.id
            .get(..self.id.floor_char_boundary(self.id.len().min(12)))
            .unwrap_or(&self.id)
    }

    #[must_use]
    pub fn compose_project(&self) -> Option<&str> {
        self.labels
            .iter()
            .find(|(key, _)| key == "com.docker.compose.project")
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskUsage {
    pub images: Vec<Image>,
    pub volumes: Vec<Volume>,
    pub build_cache_bytes: u64,
    pub build_cache_reclaimable_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    pub deleted: Vec<String>,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePrune {
    Dangling,
    AllUnused,
}

#[cfg(test)]
mod tests {
    use crate::EngineKind;

    #[test]
    fn builtin_networks_depend_on_the_engine_and_prune_rules_match_docker() {
        let mut network = crate::fixtures::networks().into_iter().next().unwrap();
        assert_eq!(network.name, "bridge");
        assert!(network.is_builtin(EngineKind::Docker));
        assert!(!network.is_builtin(EngineKind::Podman));
        network.name = "podman".into();
        assert!(!network.is_builtin(EngineKind::Docker));
        assert!(network.is_builtin(EngineKind::Podman));

        network.name = "custom".into();
        assert!(network.is_prunable(EngineKind::Docker));
        for protect in [
            |n: &mut crate::Network| n.role = crate::NetworkRole::Ingress,
            |n: &mut crate::Network| n.role = crate::NetworkRole::ConfigOnly,
            |n: &mut crate::Network| n.scope = "swarm".into(),
        ] {
            let mut network = network.clone();
            protect(&mut network);
            assert!(!network.is_prunable(EngineKind::Docker));
        }
    }

    #[test]
    fn image_names_and_volume_projects() {
        let mut image = crate::fixtures::images().into_iter().next().unwrap();
        assert!(!image.is_dangling());
        assert_eq!(image.display_name(), *image.repo_tags.first().unwrap());
        image.repo_tags.clear();
        image.id = "sha256:0123456789abcdef".into();
        assert!(image.is_dangling());
        assert_eq!(image.short_id(), "0123456789ab");
        assert_eq!(image.display_name(), "0123456789ab");
        for (id, expected) in [
            ("sha256:abc", "abc"),
            ("", ""),
            ("sha256:01234567890é", "01234567890"),
        ] {
            image.id = id.into();
            assert_eq!(image.short_id(), expected);
        }
        let mut volume = crate::fixtures::volumes().into_iter().next().unwrap();
        assert_eq!(volume.compose_project(), Some("tihlde-voting"));
        volume.labels.clear();
        assert_eq!(volume.compose_project(), None);
    }

    #[test]
    fn short_ids_preserve_utf8_boundaries() {
        let mut container = crate::fixtures::containers().into_iter().next().unwrap();
        for (id, expected) in [
            ("", ""),
            ("abc", "abc"),
            ("0123456789abcdef", "0123456789ab"),
            ("0123456789érest", "0123456789é"),
            ("01234567890érest", "01234567890"),
            ("🚢🚢🚢🚢", "🚢🚢🚢"),
        ] {
            container.id = id.into();
            assert_eq!(container.short_id(), expected);
        }
    }

    #[test]
    fn web_ports_prefer_http_and_skip_udp_and_unpublished() {
        use crate::{PortBinding, Protocol};
        let port = |host_ip: Option<&str>, host_port, container_port, protocol| PortBinding {
            host_ip: host_ip.map(Into::into),
            host_port,
            container_port,
            protocol,
        };
        let mut container = crate::fixtures::containers().into_iter().next().unwrap();
        container.ports = vec![
            port(None, Some(5432), 5432, Protocol::Tcp),
            port(None, Some(53), 53, Protocol::Udp),
            port(None, None, 9000, Protocol::Tcp),
            port(Some("127.0.0.1"), Some(9443), 8443, Protocol::Tcp),
            port(Some("::1"), Some(8081), 8080, Protocol::Tcp),
        ];
        let local = crate::Endpoint::Unix("/var/run/docker.sock".into());
        let urls = |engine: &crate::Endpoint| -> Vec<_> {
            container
                .web_ports(engine)
                .iter()
                .filter_map(|p| p.url(engine))
                .collect()
        };
        assert_eq!(
            urls(&local),
            [
                "https://127.0.0.1:9443",
                "http://[::1]:8081",
                "http://localhost:5432",
            ]
        );
        // A remote engine's wildcard ports live on its host; its loopback
        // ports aren't reachable from here.
        let tcp = |url: &str| crate::Endpoint::Tcp {
            url: url.into(),
            tls: None,
        };
        assert_eq!(
            urls(&tcp("tcp://build.lan:2376")),
            ["http://build.lan:5432"]
        );
        assert_eq!(
            urls(&tcp("https://[fd00::1]:2376")),
            ["http://[fd00::1]:5432"]
        );
        assert_eq!(urls(&tcp("tcp://127.0.0.1:2375")), urls(&local));
        assert_eq!(urls(&tcp("http://localhost:2375")), urls(&local));
        // Over SSH they live on the host `ssh -G` resolved the alias to.
        let ssh = |hostname: Option<&str>| crate::Endpoint::Ssh {
            destination: "me@prod".into(),
            port: None,
            hostname: hostname.map(Into::into),
        };
        assert_eq!(urls(&ssh(None)), ["http://prod:5432"]);
        assert_eq!(
            urls(&ssh(Some("prod.example.com"))),
            ["http://prod.example.com:5432"]
        );
        container.ports.clear();
        assert_eq!(container.web_ports(&local), Vec::<&PortBinding>::new());
    }
}

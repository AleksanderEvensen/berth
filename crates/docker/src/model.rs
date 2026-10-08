use crate::EngineKind;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    #[must_use]
    pub fn short_id(&self) -> &str {
        self.id
            .get(..self.id.floor_char_boundary(self.id.len().min(12)))
            .unwrap_or(&self.id)
    }

    /// A browser URL for the container's most web-like published TCP port,
    /// preferring common HTTP ports. Wildcard bindings map to `localhost`.
    #[must_use]
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
}

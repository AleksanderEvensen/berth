//! Realistic fixture data for UI prototyping and tests.

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::future::{self, BoxFuture};
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};

use crate::{
    ComposeRef, Container, ContainerAction, ContainerState, DiskUsage, Endpoint, Engine,
    EngineError, EngineEvent, EngineInfo, EngineKind, EngineResult, EventAction, ExecOptions,
    ExecSession, ExposedPort, FileEntry, FileMount, FilesTarget, Image, ImageDetails, ImagePrune,
    LogLine, LogOptions, Mount, MountKind, Network, NetworkEndpoint, NetworkRole, ObjectKind,
    PortBinding, Protocol, PruneReport, RemoveOptions, StatsSample, Subnet, Volume,
};

fn port(host: u16, container: u16) -> PortBinding {
    PortBinding {
        host_ip: Some("0.0.0.0".into()),
        host_port: Some(host),
        container_port: container,
        protocol: Protocol::Tcp,
    }
}

fn volume(source: &str, destination: &str) -> Mount {
    Mount {
        kind: MountKind::Volume,
        source: source.into(),
        destination: destination.into(),
        read_only: false,
    }
}

fn bind(source: &str, destination: &str, read_only: bool) -> Mount {
    Mount {
        kind: MountKind::Bind,
        source: source.into(),
        destination: destination.into(),
        read_only,
    }
}

fn compose_labels(project: &str, service: &str, dir: &str) -> Vec<(String, String)> {
    vec![
        ("com.docker.compose.project".into(), project.into()),
        ("com.docker.compose.service".into(), service.into()),
        ("com.docker.compose.project.working_dir".into(), dir.into()),
        ("com.docker.compose.version".into(), "2.39.2".into()),
    ]
}

struct Spec<'a> {
    id: &'a str,
    name: &'a str,
    image: &'a str,
    state: ContainerState,
    status: &'a str,
    ip: Option<&'a str>,
    ports: Vec<PortBinding>,
    mounts: Vec<Mount>,
    labels: Vec<(String, String)>,
    compose: Option<(&'a str, &'a str, &'a str)>,
}

impl Spec<'_> {
    fn build(self) -> Container {
        let mut labels = self.labels;
        let compose = self.compose.map(|(project, service, dir)| {
            labels.extend(compose_labels(project, service, dir));
            ComposeRef {
                project: project.into(),
                service: service.into(),
                working_dir: Some(dir.into()),
            }
        });
        labels.sort();
        Container {
            id: self.id.into(),
            name: self.name.into(),
            image: self.image.into(),
            state: self.state,
            status: self.status.into(),
            ip: self.ip.map(Into::into),
            ports: self.ports,
            mounts: self.mounts,
            labels,
            compose,
            created: 1_791_000_000,
            command: None,
            env: Some(vec![
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
            ]),
        }
    }
}

#[must_use]
pub fn containers() -> Vec<Container> {
    active_containers()
        .into_iter()
        .chain(inactive_containers())
        .collect()
}

fn active_containers() -> Vec<Container> {
    use ContainerState::{Restarting, Running};
    let s = |k: &str, v: &str| (k.to_string(), v.to_string());
    vec![
        Spec {
            id: "9f2c1a7be0d4c1f3a8e6b2d9e4f1a0c7b3d5e8f2a1c4b7d0e3f6a9c2b5d8e1f4",
            name: "tihlde-voting-postgres-1",
            image: "postgres:17-alpine",
            state: Running,
            status: "Up 3 hours (healthy)",
            ip: Some("172.19.0.2"),
            ports: vec![port(5432, 5432)],
            mounts: vec![volume("tihlde-voting_pgdata", "/var/lib/postgresql/data")],
            labels: vec![],
            compose: Some(("tihlde-voting", "postgres", "/home/dev/code/tihlde-voting")),
        }
        .build(),
        Spec {
            id: "e3a58e962b46d1c0f9a7b8e2d4c6a1f3e5b7d9c0a2f4e6b8d1c3a5f7e9b0d2c4",
            name: "rustfs",
            image: "rustfs/rustfs:latest",
            state: Running,
            status: "Up 12 minutes",
            ip: Some("172.17.0.2"),
            ports: vec![port(9000, 9000), port(9001, 9001)],
            mounts: vec![volume("rustfs_data", "/data")],
            labels: vec![
                s(
                    "description",
                    "RustFS is a distributed object storage system written in Rust",
                ),
                s("license", "Apache-2.0"),
                s("maintainer", "RustFS Team <dev@rustfs.com>"),
                s("org.opencontainers.image.title", "RustFS"),
                s("release", "1.0.0"),
                s("url", "https://rustfs.com"),
                s("vendor", "RustFS Team"),
                s("version", "v1.0.0"),
            ],
            compose: None,
        }
        .build(),
        Spec {
            id: "1b4d7e0a3c6f9b2e5d8a1c4f7b0e3d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e",
            name: "berth-dev-web-1",
            image: "nginx:1.29-alpine",
            state: Running,
            status: "Up 2 days",
            ip: Some("172.21.0.4"),
            ports: vec![port(8080, 80)],
            mounts: vec![bind(
                "/home/dev/code/berth-dev/nginx.conf",
                "/etc/nginx/nginx.conf",
                true,
            )],
            labels: vec![s(
                "maintainer",
                "NGINX Docker Maintainers <docker-maint@nginx.com>",
            )],
            compose: Some(("berth-dev", "web", "/home/dev/code/berth-dev")),
        }
        .build(),
        Spec {
            id: "7c0f3b6e9d2a5c8f1b4e7d0a3c6f9b2e5d8a1c4f7b0e3d6a9c2f5b8e1d4a7c0f",
            name: "berth-dev-api-1",
            image: "ghcr.io/berth/api:dev",
            state: Running,
            status: "Up 2 days",
            ip: Some("172.21.0.3"),
            ports: vec![port(3000, 3000)],
            mounts: vec![bind("/home/dev/code/berth-dev/api", "/app", false)],
            labels: vec![],
            compose: Some(("berth-dev", "api", "/home/dev/code/berth-dev")),
        }
        .build(),
        Spec {
            id: "4e7d0a3c6f9b2e5d8a1c4f7b0e3d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e7d",
            name: "berth-dev-worker-1",
            image: "ghcr.io/berth/api:dev",
            state: Restarting,
            status: "Restarting (1) 4 seconds ago",
            ip: None,
            ports: vec![],
            mounts: vec![bind("/home/dev/code/berth-dev/api", "/app", false)],
            labels: vec![],
            compose: Some(("berth-dev", "worker", "/home/dev/code/berth-dev")),
        }
        .build(),
    ]
}

fn inactive_containers() -> Vec<Container> {
    use ContainerState::{Created, Exited};
    vec![
        Spec {
            id: "a2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e7d0a3c6f9b2e5d8a1c4f7b0e3d6a9c2f5",
            name: "photon_dev_db",
            image: "postgres:17.6",
            state: Exited,
            status: "Exited (0) 2 days ago",
            ip: None,
            ports: vec![port(5433, 5432)],
            mounts: vec![volume("photon_dev_db_data", "/var/lib/postgresql/data")],
            labels: vec![],
            compose: Some(("photon_dev", "db", "/home/dev/code/photon")),
        }
        .build(),
        Spec {
            id: "c8f1b4e7d0a3c6f9b2e5d8a1c4f7b0e3d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1",
            name: "photon_dev_mail_sink",
            image: "axllent/mailpit:v1.27",
            state: Exited,
            status: "Exited (0) 2 days ago",
            ip: None,
            ports: vec![port(8025, 8025), port(1025, 1025)],
            mounts: vec![volume("photon_dev_mail_sink_data", "/data")],
            labels: vec![],
            compose: Some(("photon_dev", "mail_sink", "/home/dev/code/photon")),
        }
        .build(),
        Spec {
            id: "d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e7d0a3c6f9b2e5d8a1c4f7b0e3d6a9",
            name: "photon_dev_redis",
            image: "redis:7.4-alpine",
            state: Exited,
            status: "Exited (0) 2 days ago",
            ip: None,
            ports: vec![port(6379, 6379)],
            mounts: vec![volume("photon_dev_redis_data", "/data")],
            labels: vec![],
            compose: Some(("photon_dev", "redis", "/home/dev/code/photon")),
        }
        .build(),
        Spec {
            id: "b0e3d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e7d0a3c6f9b2e5d8a1c4f7b0e3",
            name: "photon_dev_rustfs",
            image: "rustfs/rustfs:latest",
            state: Exited,
            status: "Exited (137) 2 days ago",
            ip: None,
            ports: vec![port(9100, 9000)],
            mounts: vec![volume("photon_dev_rustfs_data", "/data")],
            labels: vec![],
            compose: Some(("photon_dev", "rustfs", "/home/dev/code/photon")),
        }
        .build(),
        Spec {
            id: "f4c7b0e3d6a9c2f5b8e1d4a7c0f3b6e9d2a5c8f1b4e7d0a3c6f9b2e5d8a1c4f7",
            name: "mailpit-scratch",
            image: "axllent/mailpit:latest",
            state: Created,
            status: "Created",
            ip: None,
            ports: vec![],
            mounts: vec![],
            labels: vec![],
            compose: None,
        }
        .build(),
    ]
}

/// Image inventory includes each fixture container's image, unused tags and
/// four untagged build results. IDs are stable content-shaped SHA256 digests.
#[must_use]
pub fn images() -> Vec<Image> {
    images_for(&containers())
}

fn images_for(containers: &[Container]) -> Vec<Image> {
    use sha2::{Digest as _, Sha256};
    let mut tags: Vec<_> = containers.iter().map(|c| c.image.clone()).collect();
    tags.extend(
        [
            "apache/kafka:4.1.0",
            "provectuslabs/kafka-ui:latest",
            "crystaldba/postgres-mcp:latest",
            "blue-zone-workshop-api:latest",
            "blue-zone-workshop-worker:latest",
            "blue-zone-workshop-web:latest",
        ]
        .map(str::to_owned),
    );
    tags.sort();
    tags.dedup();
    let mut images: Vec<_> = tags
        .into_iter()
        .map(|tag| {
            let size = if tag.contains("kafka") {
                640_000_000
            } else if tag.contains("postgres") {
                275_000_000
            } else if tag.contains("nginx") || tag.contains("redis") || tag.contains("mailpit") {
                48_000_000
            } else {
                185_000_000
            };
            Image {
                id: format!("sha256:{:x}", Sha256::digest(tag.as_bytes())),
                containers: Some(count(containers.iter().filter(|c| c.image == tag).count())),
                repo_tags: vec![tag],
                size,
                created: 1_790_000_000,
                labels: vec![],
            }
        })
        .collect();
    for index in 0..4 {
        images.push(Image {
            id: format!("sha256:{:x}", Sha256::digest(format!("dangling-{index}"))),
            repo_tags: vec![],
            size: 115_000_000,
            created: 1_785_000_000,
            containers: Some(0),
            labels: vec![],
        });
    }
    images
}

/// Named mounts, old project caches and two anonymous volumes.
#[must_use]
pub fn volumes() -> Vec<Volume> {
    volumes_for(&containers())
}

fn volumes_for(containers: &[Container]) -> Vec<Volume> {
    let mut volumes = Vec::new();
    for container in containers {
        for mount in container
            .mounts
            .iter()
            .filter(|m| m.kind == MountKind::Volume)
        {
            if volumes.iter().any(|v: &Volume| v.name == mount.source) {
                continue;
            }
            let labels = container.compose.as_ref().map_or_else(Vec::new, |compose| {
                vec![("com.docker.compose.project".into(), compose.project.clone())]
            });
            volumes.push(fixture_volume(&mount.source, 240_000_000, labels));
        }
    }
    for name in [
        "blue-zone-workshop_maven-cache",
        "photon-old_maven-cache",
        "scratch_maven-cache",
    ] {
        if !volumes.iter().any(|v| v.name == name) {
            volumes.push(fixture_volume(name, 350_000_000, vec![]));
        }
    }
    if !volumes
        .iter()
        .any(|v| v.name == "blue-zone-workshop_pgdata")
    {
        volumes.push(fixture_volume(
            "blue-zone-workshop_pgdata",
            180_000_000,
            vec![(
                "com.docker.compose.project".into(),
                "blue-zone-workshop".into(),
            )],
        ));
    }
    for name in ["a".repeat(64), "b".repeat(64)] {
        if !volumes.iter().any(|v| v.name == name) {
            volumes.push(fixture_volume(&name, 65_000_000, vec![]));
        }
    }
    for volume in &mut volumes {
        volume.ref_count = Some(volume_references(containers, &volume.name));
    }
    volumes
}

fn fixture_volume(name: &str, size: u64, labels: Vec<(String, String)>) -> Volume {
    Volume {
        name: name.into(),
        driver: "local".into(),
        mountpoint: format!("/var/lib/docker/volumes/{name}/_data"),
        created: Some("2026-09-22T09:30:00Z".into()),
        labels,
        size: Some(size),
        ref_count: Some(0),
    }
}

/// The engine's built-in networks, one per Compose project and two
/// leftovers from projects that are gone. Endpoints are filled in on list.
#[must_use]
pub fn networks() -> Vec<Network> {
    networks_for(&containers())
}

fn networks_for(containers: &[Container]) -> Vec<Network> {
    let projects: std::collections::BTreeSet<&str> = containers
        .iter()
        .filter_map(|c| c.compose.as_ref())
        .map(|c| c.project.as_str())
        .chain(["blue-zone-workshop", "scratch"])
        .collect();
    let mut networks = vec![
        fixture_network("bridge", "bridge", Some("172.17"), vec![]),
        fixture_network("host", "host", None, vec![]),
        fixture_network("none", "null", None, vec![]),
    ];
    let mut spare = 30_u8..;
    for project in projects {
        // Reuse the subnet the project's containers already have addresses in.
        let prefix = containers
            .iter()
            .filter(|c| c.compose.as_ref().is_some_and(|c| c.project == project))
            .find_map(|c| c.ip.as_deref())
            .and_then(|ip| ip.rsplitn(3, '.').nth(2))
            .map_or_else(
                || format!("172.{}", spare.next().unwrap_or(u8::MAX)),
                str::to_owned,
            );
        networks.push(fixture_network(
            &format!("{project}_default"),
            "bridge",
            Some(&prefix),
            vec![
                ("com.docker.compose.network".into(), "default".into()),
                ("com.docker.compose.project".into(), project.into()),
            ],
        ));
    }
    networks
}

fn fixture_network(
    name: &str,
    driver: &str,
    prefix: Option<&str>,
    labels: Vec<(String, String)>,
) -> Network {
    // A stable, unique, 64-hex ID derived from the name.
    let hex: String = name
        .bytes()
        .flat_map(|b| [b >> 4, b & 0xf])
        .map(|n| char::from_digit(u32::from(n), 16).unwrap_or('0'))
        .collect();
    let id: String = hex.chars().cycle().take(64).collect();
    Network {
        id,
        name: name.into(),
        driver: driver.into(),
        scope: "local".into(),
        created: Some("2026-09-22T09:30:00Z".into()),
        internal: false,
        attachable: false,
        role: NetworkRole::Regular,
        ipv6: false,
        subnets: prefix
            .map(|prefix| Subnet {
                subnet: format!("{prefix}.0.0/16"),
                gateway: Some(format!("{prefix}.0.1")),
            })
            .into_iter()
            .collect(),
        labels,
        endpoints: Vec::new(),
    }
}

/// Running containers attached to `network`: the default bridge for
/// standalone containers, `<project>_default` for Compose services.
fn network_endpoints(containers: &[Container], network: &Network) -> Vec<NetworkEndpoint> {
    let mut endpoints: Vec<NetworkEndpoint> = containers
        .iter()
        .filter(|c| c.state.is_active())
        .filter(|c| {
            c.compose.as_ref().map_or_else(
                || network.name == "bridge",
                |compose| network.name == format!("{}_default", compose.project),
            )
        })
        .map(|c| NetworkEndpoint {
            container_id: c.id.clone(),
            name: c.name.clone(),
            ipv4: c.ip.as_ref().map(|ip| format!("{ip}/16")),
            ipv6: None,
        })
        .collect();
    endpoints.sort_by(|a, b| a.name.cmp(&b.name));
    endpoints
}

fn with_endpoints(containers: &[Container], mut network: Network) -> Network {
    network.endpoints = network_endpoints(containers, &network);
    network
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Plausible inspect config for a fixture image, keyed off its tag.
#[must_use]
pub fn image_details(image: &Image) -> ImageDetails {
    let tag = image.repo_tags.first().map_or("", String::as_str);
    let tcp = |port| ExposedPort {
        port,
        protocol: Protocol::Tcp,
    };
    let strings = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let path = "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
    let mut details = ImageDetails {
        id: image.id.clone(),
        platform: Some("linux/amd64".into()),
        command: strings(&["/bin/sh"]),
        env: strings(&[path]),
        ..ImageDetails::default()
    };
    if tag.contains("postgres") {
        details.entrypoint = strings(&["docker-entrypoint.sh"]);
        details.command = strings(&["postgres"]);
        details.env = strings(&[path, "PG_MAJOR=17", "PGDATA=/var/lib/postgresql/data"]);
        details.volumes = strings(&["/var/lib/postgresql/data"]);
        details.exposed_ports = vec![tcp(5432)];
    } else if tag.contains("redis") {
        details.entrypoint = strings(&["docker-entrypoint.sh"]);
        details.command = strings(&["redis-server"]);
        details.working_dir = Some("/data".into());
        details.volumes = strings(&["/data"]);
        details.exposed_ports = vec![tcp(6379)];
    } else if tag.contains("nginx") {
        details.entrypoint = strings(&["/docker-entrypoint.sh"]);
        details.command = strings(&["nginx", "-g", "daemon off;"]);
        details.env = strings(&[path, "NGINX_VERSION=1.29.1"]);
        details.exposed_ports = vec![tcp(80)];
    } else if tag.contains("kafka") {
        details.platform = Some("linux/arm64/v8".into());
        details.command = strings(&["/etc/kafka/docker/run"]);
        details.working_dir = Some("/opt/kafka".into());
        details.user = Some("appuser".into());
        details.volumes = strings(&["/etc/kafka/secrets", "/var/lib/kafka/data"]);
        details.exposed_ports = vec![tcp(9092)];
    } else if !image.is_dangling() {
        details.command = strings(&["node", "dist/main.js"]);
        details.working_dir = Some("/app".into());
        details.env = strings(&[path, "NODE_ENV=production"]);
        details.exposed_ports = vec![tcp(3000)];
    }
    details
}

fn image_references(containers: &[Container], image: &Image) -> u32 {
    count(
        containers
            .iter()
            .filter(|c| c.image == image.id || image.repo_tags.contains(&c.image))
            .count(),
    )
}

fn volume_references(containers: &[Container], name: &str) -> u32 {
    count(
        containers
            .iter()
            .filter(|c| {
                c.mounts
                    .iter()
                    .any(|m| m.kind == MountKind::Volume && m.source == name)
            })
            .count(),
    )
}

// Fixture writable layers are small; image and volume storage is counted separately.
const CONTAINER_BYTES: u64 = 8_000_000;

/// An in-memory [`Engine`] over [`containers`]. Actions mutate the fixtures
/// and emit the same events a real engine would, so the UI's event-driven
/// store can be built and tested without Docker.
pub struct FixtureEngine {
    state: Arc<Mutex<FixtureState>>,
    latency: Option<Duration>,
    list_latency: Option<Duration>,
    log_history_lines: u32,
}

#[derive(Default)]
struct FixtureState {
    containers: Vec<Container>,
    images: Vec<Image>,
    volumes: Vec<Volume>,
    networks: Vec<Network>,
    build_cache: Vec<(String, u64, bool)>,
    subscribers: Vec<mpsc::Sender<EngineResult<EngineEvent>>>,
    /// File-browsing helpers started, for unique IDs.
    helpers: u32,
}

impl FixtureState {
    fn emit_deleted(&mut self, kind: ObjectKind, report: &PruneReport) {
        for id in &report.deleted {
            self.emit(kind, id, EventAction::Destroy);
        }
    }

    fn emit(&mut self, kind: ObjectKind, id: &str, action: EventAction) {
        let event = EngineEvent {
            kind,
            id: id.to_string(),
            action,
            time_nanos: 0,
        };
        self.subscribers
            .retain_mut(|tx| tx.try_send(Ok(event.clone())).is_ok());
    }
}

impl Default for FixtureEngine {
    fn default() -> Self {
        Self::new(containers())
    }
}

impl FixtureEngine {
    #[must_use]
    pub fn new(containers: Vec<Container>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FixtureState {
                images: images_for(&containers),
                volumes: volumes_for(&containers),
                networks: networks_for(&containers),
                build_cache: vec![
                    ("cache-api".into(), 220_000_000, false),
                    ("cache-workshop".into(), 380_000_000, false),
                    ("cache-active".into(), 90_000_000, true),
                ],
                containers,
                subscribers: Vec::new(),
                helpers: 0,
            })),
            latency: None,
            list_latency: None,
            log_history_lines: 240,
        }
    }

    /// Delay every mutation (and its event), plus logs/stats/exec startup.
    /// Makes pending UI states visible without requiring an async runtime.
    #[must_use]
    pub const fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
    }

    /// Delay network listings and disk usage: the state is read immediately,
    /// the reply arrives later — so changes made meanwhile are missing from it.
    #[must_use]
    pub const fn with_list_latency(mut self, latency: Duration) -> Self {
        self.list_latency = Some(latency);
        self
    }

    /// Reply with `value`, after the configured list latency if any.
    fn listed<T: Send + 'static>(&self, value: T) -> BoxFuture<'static, EngineResult<T>> {
        let Some(latency) = self.list_latency else {
            return future::ready(Ok(value)).boxed();
        };
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(latency);
            let _ = tx.send(value);
        });
        rx.map(|r| r.map_err(|_| EngineError::Transport("fixture worker gone".into())))
            .boxed()
    }

    /// Apply `f` to the state, after the configured latency if any.
    fn mutate<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut FixtureState) -> EngineResult<T> + Send + 'static,
    ) -> BoxFuture<'static, EngineResult<T>> {
        let state = self.state.clone();
        let apply = move || {
            f(&mut state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner))
        };
        match self.latency {
            None => future::ready(apply()).boxed(),
            Some(latency) => {
                let (tx, rx) = oneshot::channel();
                std::thread::spawn(move || {
                    std::thread::sleep(latency);
                    let _ = tx.send(apply());
                });
                rx.map(|r| {
                    r.unwrap_or_else(|_| Err(EngineError::Transport("fixture worker gone".into())))
                })
                .boxed()
            }
        }
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut FixtureState) -> T) -> T {
        f(&mut self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl Engine for FixtureEngine {
    fn info(&self) -> BoxFuture<'static, EngineResult<EngineInfo>> {
        future::ready(Ok(EngineInfo {
            kind: EngineKind::Docker,
            name: "Fixture Engine".into(),
            version: "28.4.0".into(),
            api_version: "1.51".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            rootless: false,
            endpoint: Endpoint::Unix("/dev/null".into()),
        }))
        .boxed()
    }

    fn list_containers(&self) -> BoxFuture<'static, EngineResult<Vec<Container>>> {
        let list = self.with_state(|s| {
            s.containers
                .iter()
                .cloned()
                .map(|mut c| {
                    c.env = None;
                    c
                })
                .collect()
        });
        future::ready(Ok(list)).boxed()
    }

    fn inspect_container(&self, id: &str) -> BoxFuture<'static, EngineResult<Container>> {
        let found = self.with_state(|s| s.containers.iter().find(|c| c.id == id).cloned());
        future::ready(found.ok_or_else(|| EngineError::NotFound(id.to_string()))).boxed()
    }

    fn container_action(
        &self,
        id: &str,
        action: ContainerAction,
    ) -> BoxFuture<'static, EngineResult<()>> {
        let id = id.to_string();
        self.mutate(move |s| {
            let c = s
                .containers
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            let (state, status, event) = match action {
                ContainerAction::Start => (
                    ContainerState::Running,
                    "Up Less than a second",
                    EventAction::Start,
                ),
                ContainerAction::Restart { .. } => (
                    ContainerState::Running,
                    "Up Less than a second",
                    EventAction::Restart,
                ),
                ContainerAction::Stop { .. } => (
                    ContainerState::Exited,
                    "Exited (0) Less than a second ago",
                    EventAction::Die,
                ),
                ContainerAction::Kill => (
                    ContainerState::Exited,
                    "Exited (137) Less than a second ago",
                    EventAction::Kill,
                ),
                ContainerAction::Pause => {
                    (ContainerState::Paused, "Up (Paused)", EventAction::Pause)
                }
                ContainerAction::Unpause => (ContainerState::Running, "Up", EventAction::Unpause),
            };
            c.state = state;
            c.status = status.into();
            if !state.is_active() {
                c.ip = None;
            }
            let id = c.id.clone();
            s.emit(ObjectKind::Container, &id, event);
            Ok(())
        })
    }

    fn remove_container(
        &self,
        id: &str,
        options: RemoveOptions,
    ) -> BoxFuture<'static, EngineResult<()>> {
        let id = id.to_string();
        self.mutate(move |s| {
            let ix = s
                .containers
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            let container = s
                .containers
                .get(ix)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            if container.state.is_active() && !options.force {
                return Err(EngineError::Conflict(format!(
                    "cannot remove container {}: container is running",
                    container.name
                )));
            }
            let removed = s.containers.remove(ix);
            s.emit(ObjectKind::Container, &removed.id, EventAction::Destroy);
            Ok(())
        })
    }

    fn list_images(&self) -> BoxFuture<'static, EngineResult<Vec<Image>>> {
        future::ready(Ok(self.with_state(|s| {
            s.images
                .iter()
                .cloned()
                .map(|mut image| {
                    image.containers = Some(image_references(&s.containers, &image));
                    image
                })
                .collect()
        })))
        .boxed()
    }

    fn inspect_image(&self, id: &str) -> BoxFuture<'static, EngineResult<ImageDetails>> {
        let found = self.with_state(|s| s.images.iter().find(|i| i.id == id).map(image_details));
        future::ready(found.ok_or_else(|| EngineError::NotFound(id.to_string()))).boxed()
    }

    fn list_volumes(&self) -> BoxFuture<'static, EngineResult<Vec<Volume>>> {
        future::ready(Ok(self.with_state(|s| {
            s.volumes
                .iter()
                .cloned()
                .map(|mut volume| {
                    volume.size = None;
                    volume.ref_count = None;
                    volume
                })
                .collect()
        })))
        .boxed()
    }

    fn disk_usage(&self) -> BoxFuture<'static, EngineResult<DiskUsage>> {
        self.listed(self.with_state(|s| {
            DiskUsage {
                images: s
                    .images
                    .iter()
                    .cloned()
                    .map(|mut image| {
                        image.containers = Some(image_references(&s.containers, &image));
                        image
                    })
                    .collect(),
                volumes: s
                    .volumes
                    .iter()
                    .cloned()
                    .map(|mut volume| {
                        volume.ref_count = Some(volume_references(&s.containers, &volume.name));
                        volume
                    })
                    .collect(),
                build_cache_bytes: s
                    .build_cache
                    .iter()
                    .fold(0_u64, |sum, (_, size, _)| sum.saturating_add(*size)),
                build_cache_reclaimable_bytes: s
                    .build_cache
                    .iter()
                    .filter(|(_, _, in_use)| !in_use)
                    .fold(0_u64, |sum, (_, size, _)| sum.saturating_add(*size)),
            }
        }))
    }

    fn remove_image(&self, id: &str, force: bool) -> BoxFuture<'static, EngineResult<()>> {
        let id = id.to_owned();
        self.mutate(move |s| {
            let index = s
                .images
                .iter()
                .position(|i| i.id == id || i.repo_tags.contains(&id))
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            let image = s
                .images
                .get(index)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            if !force && image_references(&s.containers, image) > 0 {
                return Err(EngineError::Conflict(format!(
                    "cannot remove image {id}: image is used by a container"
                )));
            }
            let image = s.images.remove(index);
            s.emit(ObjectKind::Image, &image.id, EventAction::Destroy);
            Ok(())
        })
    }

    fn remove_volume(&self, name: &str, force: bool) -> BoxFuture<'static, EngineResult<()>> {
        let name = name.to_owned();
        self.mutate(move |s| {
            let index = s
                .volumes
                .iter()
                .position(|v| v.name == name)
                .ok_or_else(|| EngineError::NotFound(name.clone()))?;
            if !force && volume_references(&s.containers, &name) > 0 {
                return Err(EngineError::Conflict(format!(
                    "cannot remove volume {name}: volume is in use"
                )));
            }
            s.volumes.remove(index);
            s.emit(ObjectKind::Volume, &name, EventAction::Destroy);
            Ok(())
        })
    }

    fn list_networks(&self) -> BoxFuture<'static, EngineResult<Vec<Network>>> {
        let list: Vec<Network> = self.with_state(|s| {
            s.networks
                .iter()
                .cloned()
                .map(|network| with_endpoints(&s.containers, network))
                .collect()
        });
        self.listed(list)
    }

    fn remove_network(&self, id: &str) -> BoxFuture<'static, EngineResult<()>> {
        let id = id.to_owned();
        self.mutate(move |s| {
            let index = s
                .networks
                .iter()
                .position(|n| n.id == id || n.name == id)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            let network = s
                .networks
                .get(index)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            if network.is_builtin(EngineKind::Docker) {
                return Err(EngineError::Engine {
                    status: 403,
                    message: format!(
                        "{} is a pre-defined network and cannot be removed",
                        network.name
                    ),
                });
            }
            if !network_endpoints(&s.containers, network).is_empty() {
                return Err(EngineError::Engine {
                    status: 403,
                    message: format!(
                        "error while removing network: network {} has active endpoints",
                        network.name
                    ),
                });
            }
            let network = s.networks.remove(index);
            s.emit(ObjectKind::Network, &network.id, EventAction::Destroy);
            Ok(())
        })
    }

    fn prune_containers(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        self.mutate(|s| {
            let mut report = PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0,
            };
            s.containers.retain(|c| {
                let remove = matches!(
                    c.state,
                    ContainerState::Created | ContainerState::Exited | ContainerState::Dead
                );
                if remove {
                    report.deleted.push(c.id.clone());
                    report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(CONTAINER_BYTES);
                }
                !remove
            });
            s.emit_deleted(ObjectKind::Container, &report);
            Ok(report)
        })
    }

    fn prune_images(&self, mode: ImagePrune) -> BoxFuture<'static, EngineResult<PruneReport>> {
        self.mutate(move |s| {
            let mut report = PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0,
            };
            s.images.retain(|image| {
                let remove = image_references(&s.containers, image) == 0
                    && (mode == ImagePrune::AllUnused || image.is_dangling());
                if remove {
                    report.deleted.push(image.id.clone());
                    report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(image.size);
                }
                !remove
            });
            s.emit_deleted(ObjectKind::Image, &report);
            Ok(report)
        })
    }

    fn prune_volumes(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        self.mutate(|s| {
            let mut report = PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0,
            };
            s.volumes.retain(|volume| {
                let remove = volume_references(&s.containers, &volume.name) == 0;
                if remove {
                    report.deleted.push(volume.name.clone());
                    report.reclaimed_bytes = report
                        .reclaimed_bytes
                        .saturating_add(volume.size.unwrap_or_default());
                }
                !remove
            });
            s.emit_deleted(ObjectKind::Volume, &report);
            Ok(report)
        })
    }

    fn prune_networks(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        self.mutate(|s| {
            let mut report = PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0,
            };
            // Like Docker, names are reported and IDs are in the events.
            let mut ids = Vec::new();
            s.networks.retain(|network| {
                let network = with_endpoints(&s.containers, network.clone());
                let keep = !network.is_prunable(EngineKind::Docker);
                if !keep {
                    report.deleted.push(network.name.clone());
                    ids.push(network.id);
                }
                keep
            });
            for id in ids {
                s.emit(ObjectKind::Network, &id, EventAction::Destroy);
            }
            Ok(report)
        })
    }

    fn prune_build_cache(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        self.mutate(|s| {
            let mut report = PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0,
            };
            s.build_cache.retain(|(id, size, in_use)| {
                if !in_use {
                    report.deleted.push(id.clone());
                    report.reclaimed_bytes = report.reclaimed_bytes.saturating_add(*size);
                }
                *in_use
            });
            Ok(report)
        })
    }

    fn events(&self) -> BoxStream<'static, EngineResult<EngineEvent>> {
        // Overflow closes this subscription so consumers can resync rather
        // than retaining an unbounded event backlog or missing events silently.
        let (tx, rx) = mpsc::channel(64);
        self.with_state(|s| s.subscribers.push(tx));
        rx.boxed()
    }

    fn logs(&self, id: &str, options: LogOptions) -> BoxStream<'static, EngineResult<LogLine>> {
        self.fixture_logs(id, options)
    }

    fn stats(&self, id: &str) -> BoxStream<'static, EngineResult<StatsSample>> {
        self.fixture_stats(id)
    }

    fn exec(
        &self,
        id: &str,
        options: ExecOptions,
    ) -> BoxFuture<'static, EngineResult<ExecSession>> {
        self.fixture_exec(id, options)
    }

    fn open_files(&self, target: &FilesTarget) -> BoxFuture<'static, EngineResult<FileMount>> {
        self.fixture_open_files(target)
    }

    fn list_files(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<Vec<FileEntry>>> {
        self.fixture_list_files(mount, path)
    }

    fn read_link(&self, mount: &FileMount, path: &str) -> BoxFuture<'static, EngineResult<String>> {
        self.fixture_read_link(mount, path)
    }

    fn copy_out(
        &self,
        mount: &FileMount,
        path: &str,
        dest: &Path,
        copied: Arc<AtomicU64>,
    ) -> BoxFuture<'static, EngineResult<()>> {
        self.fixture_copy_out(mount, path, dest, copied)
    }
}

mod files;
mod m3;

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn actions_emit_events_and_respect_conflicts() {
        let engine = FixtureEngine::default();
        let mut events = engine.events();
        let rustfs = "e3a58e962b46d1c0f9a7b8e2d4c6a1f3e5b7d9c0a2f4e6b8d1c3a5f7e9b0d2c4";

        let err = block_on(engine.remove_container(rustfs, RemoveOptions::default()));
        assert!(matches!(err, Err(EngineError::Conflict(_))));

        block_on(engine.container_action(rustfs, ContainerAction::Stop { timeout_secs: None }))
            .unwrap();
        let ev = block_on(events.next()).unwrap().unwrap();
        assert_eq!((ev.id.as_str(), ev.action), (rustfs, EventAction::Die));
        assert_eq!(
            block_on(engine.inspect_container(rustfs)).unwrap().state,
            ContainerState::Exited
        );

        block_on(engine.remove_container(rustfs, RemoveOptions::default())).unwrap();
        let ev = block_on(events.next()).unwrap().unwrap();
        assert_eq!(ev.action, EventAction::Destroy);
        assert!(matches!(
            block_on(engine.inspect_container(rustfs)),
            Err(EngineError::NotFound(_))
        ));
    }

    #[test]
    fn list_omits_env() {
        let engine = FixtureEngine::default();
        let list = block_on(engine.list_containers()).unwrap();
        assert!(list.iter().all(|c| c.env.is_none()));
    }
}

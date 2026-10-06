//! Realistic fixture data for UI prototyping and tests.

use std::sync::{Arc, Mutex};

use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::future::{self, BoxFuture};
use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};

use crate::{
    ComposeRef, Container, ContainerAction, ContainerState, Endpoint, Engine, EngineError,
    EngineEvent, EngineInfo, EngineKind, EngineResult, EventAction, Mount, MountKind, ObjectKind,
    PortBinding, Protocol, RemoveOptions,
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

/// An in-memory [`Engine`] over [`containers`]. Actions mutate the fixtures
/// and emit the same events a real engine would, so the UI's event-driven
/// store can be built and tested without Docker.
pub struct FixtureEngine {
    state: Arc<Mutex<FixtureState>>,
    latency: Option<Duration>,
}

#[derive(Default)]
struct FixtureState {
    containers: Vec<Container>,
    subscribers: Vec<mpsc::UnboundedSender<EngineResult<EngineEvent>>>,
}

impl FixtureState {
    fn emit(&mut self, id: &str, action: EventAction) {
        let event = EngineEvent {
            kind: ObjectKind::Container,
            id: id.to_string(),
            action,
            time_nanos: 0,
        };
        self.subscribers
            .retain(|tx| tx.unbounded_send(Ok(event.clone())).is_ok());
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
                containers,
                subscribers: Vec::new(),
            })),
            latency: None,
        }
    }

    /// Delay every mutation (and its event) — makes pending UI states visible.
    #[must_use]
    pub const fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
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
            s.emit(&id, event);
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
            s.emit(&removed.id, EventAction::Destroy);
            Ok(())
        })
    }

    fn events(&self) -> BoxStream<'static, EngineResult<EngineEvent>> {
        let (tx, rx) = mpsc::unbounded();
        self.with_state(|s| s.subscribers.push(tx));
        rx.boxed()
    }
}

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

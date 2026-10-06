//! Realistic fixture data for UI prototyping and tests.

use crate::{ComposeRef, Container, ContainerState, Mount, MountKind, PortBinding, Protocol};

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
        }
    }
}

pub fn containers() -> Vec<Container> {
    use ContainerState::*;
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
                s("description", "RustFS is a distributed object storage system written in Rust"),
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
            mounts: vec![bind("/home/dev/code/berth-dev/nginx.conf", "/etc/nginx/nginx.conf", true)],
            labels: vec![s("maintainer", "NGINX Docker Maintainers <docker-maint@nginx.com>")],
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

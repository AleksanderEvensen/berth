//! Pure translations from the Docker API into Berth's domain.
use std::collections::HashMap;

use bollard::models as api;
use chrono::{DateTime, Utc};

use crate::{
    ComposeRef, Container, ContainerState, Endpoint, EngineError, EngineEvent, EngineInfo,
    EngineKind, EventAction, Mount, MountKind, ObjectKind, PortBinding, Protocol,
};

pub(super) fn state(value: &str) -> ContainerState {
    match value {
        "running" | "stopping" => ContainerState::Running,
        "paused" => ContainerState::Paused,
        "restarting" => ContainerState::Restarting,
        "removing" => ContainerState::Removing,
        "exited" => ContainerState::Exited,
        "dead" => ContainerState::Dead,
        _ => ContainerState::Created,
    }
}

fn protocol(value: &str) -> Option<Protocol> {
    match value {
        "tcp" => Some(Protocol::Tcp),
        "udp" => Some(Protocol::Udp),
        "sctp" => Some(Protocol::Sctp),
        _ => None,
    }
}

fn labels(values: Option<HashMap<String, String>>) -> Vec<(String, String)> {
    let mut values: Vec<_> = values.unwrap_or_default().into_iter().collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

fn compose(labels: &[(String, String)]) -> Option<ComposeRef> {
    let get = |key| {
        labels
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    Some(ComposeRef {
        project: get("com.docker.compose.project")?,
        service: get("com.docker.compose.service")?,
        working_dir: get("com.docker.compose.project.working_dir"),
    })
}

fn ip(networks: Option<HashMap<String, api::EndpointSettings>>) -> Option<String> {
    // Docker maps have no ordering. Network-name order gives stable results.
    let mut networks: Vec<_> = networks.unwrap_or_default().into_iter().collect();
    networks.sort_by(|a, b| a.0.cmp(&b.0));
    networks
        .into_iter()
        .find_map(|(_, network)| network.ip_address.filter(|address| !address.is_empty()))
}

fn mounts(values: Option<Vec<api::MountPoint>>) -> Vec<Mount> {
    values
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let kind = match m.typ.as_deref()? {
                "volume" => MountKind::Volume,
                "bind" => MountKind::Bind,
                "tmpfs" => MountKind::Tmpfs,
                _ => return None,
            };
            Some(Mount {
                kind,
                source: if kind == MountKind::Volume {
                    m.name.or(m.source)
                } else {
                    m.source
                }
                .unwrap_or_default(),
                destination: m.destination.unwrap_or_default(),
                read_only: !m.rw.unwrap_or(false),
            })
        })
        .collect()
}

fn ports(mut values: Vec<PortBinding>) -> Vec<PortBinding> {
    // Preserve separate explicit host addresses. Only wildcard IPv4/IPv6
    // pairs describe the same dual-stack publication; prefer the IPv4 row.
    values.sort_by(|a, b| {
        (
            a.container_port,
            a.protocol.label(),
            a.host_port,
            &a.host_ip,
        )
            .cmp(&(
                b.container_port,
                b.protocol.label(),
                b.host_port,
                &b.host_ip,
            ))
    });
    values.dedup();
    let ipv4: Vec<_> = values
        .iter()
        .filter(|p| p.host_ip.as_deref() == Some("0.0.0.0"))
        .map(|p| (p.container_port, p.protocol, p.host_port))
        .collect();
    values.retain(|p| {
        p.host_ip.as_deref() != Some("::")
            || !ipv4.contains(&(p.container_port, p.protocol, p.host_port))
    });
    values
}

pub(super) fn summary(value: api::ContainerSummary) -> Container {
    let labels = labels(value.labels);
    let compose = compose(&labels);
    Container {
        id: value.id.unwrap_or_default(),
        name: value
            .names
            .unwrap_or_default()
            .first()
            .map(|n| n.trim_start_matches('/').to_owned())
            .unwrap_or_default(),
        image: value.image.unwrap_or_default(),
        state: state(value.state.as_ref().map_or("", AsRef::as_ref)),
        status: value.status.unwrap_or_default(),
        ip: ip(value.network_settings.and_then(|n| n.networks)),
        ports: ports(
            value
                .ports
                .unwrap_or_default()
                .into_iter()
                .filter_map(|p| {
                    Some(PortBinding {
                        host_ip: p.ip.filter(|s| !s.is_empty()),
                        host_port: p.public_port,
                        container_port: p.private_port,
                        protocol: protocol(p.typ.as_ref().map_or("tcp", AsRef::as_ref))?,
                    })
                })
                .collect(),
        ),
        mounts: mounts(value.mounts),
        labels,
        compose,
        created: value.created.unwrap_or_default(),
        command: value.command,
        env: None,
    }
}

fn elapsed(timestamp: Option<&str>, now: DateTime<Utc>) -> Option<String> {
    let timestamp = DateTime::parse_from_rfc3339(timestamp?).ok()?;
    // Docker's year-one zero timestamp means the process has never run.
    if timestamp.timestamp() <= 0 {
        return None;
    }
    let seconds = now.timestamp().saturating_sub(timestamp.timestamp()).max(0);
    let (amount, unit) = match seconds {
        0 => return Some("Less than a second".into()),
        1..60 => (seconds, "second"),
        60..3600 => (seconds / 60, "minute"),
        3600..86400 => (seconds / 3600, "hour"),
        _ => (seconds / 86400, "day"),
    };
    Some(format!(
        "{amount} {unit}{}",
        if amount == 1 { "" } else { "s" }
    ))
}

pub(super) fn status(value: &api::ContainerState, now: DateTime<Utc>) -> String {
    let state = state(value.status.as_ref().map_or("", AsRef::as_ref));
    let ago = || {
        elapsed(value.finished_at.as_deref(), now)
            .map(|s| format!(" {s} ago"))
            .unwrap_or_default()
    };
    match state {
        ContainerState::Running | ContainerState::Paused => {
            let mut text = elapsed(value.started_at.as_deref(), now)
                .map_or_else(|| "Up".into(), |s| format!("Up {s}"));
            if state == ContainerState::Paused {
                text.push_str(" (Paused)");
            } else if let Some(health) = value.health.as_ref().and_then(|h| h.status.as_ref())
                && !health.as_ref().is_empty()
                && health.as_ref() != "none"
            {
                text.push_str(" (");
                text.push_str(health.as_ref());
                text.push(')');
            }
            text
        }
        ContainerState::Exited => {
            format!("Exited ({}){}", value.exit_code.unwrap_or_default(), ago())
        }
        ContainerState::Restarting => format!(
            "Restarting ({}){}",
            value.exit_code.unwrap_or_default(),
            ago()
        ),
        other => other.label().into(),
    }
}

pub(super) fn inspect(value: api::ContainerInspectResponse, now: DateTime<Utc>) -> Container {
    let config = value.config.unwrap_or_default();
    let labels = labels(config.labels);
    let compose = compose(&labels);
    let network = value.network_settings.unwrap_or_default();
    let mut mapped_ports = Vec::new();
    for (port, bindings) in network.ports.unwrap_or_default() {
        let Some((number, kind)) = port.split_once('/') else {
            continue;
        };
        let (Ok(number), Some(kind)) = (number.parse::<u16>(), protocol(kind)) else {
            continue;
        };
        let bindings = bindings.unwrap_or_default();
        if bindings.is_empty() {
            mapped_ports.push(PortBinding {
                host_ip: None,
                host_port: None,
                container_port: number,
                protocol: kind,
            });
        }
        for binding in bindings {
            mapped_ports.push(PortBinding {
                host_ip: binding.host_ip.filter(|s| !s.is_empty()),
                host_port: binding.host_port.and_then(|s| s.parse().ok()),
                container_port: number,
                protocol: kind,
            });
        }
    }
    let command = value
        .path
        .filter(|s| !s.is_empty())
        .map(|path| {
            std::iter::once(path)
                .chain(value.args.unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .or_else(|| {
            let parts: Vec<_> = config
                .entrypoint
                .unwrap_or_default()
                .into_iter()
                .chain(config.cmd.unwrap_or_default())
                .collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        });
    let engine_state = value.state.unwrap_or_default();
    Container {
        id: value.id.unwrap_or_default(),
        name: value
            .name
            .unwrap_or_default()
            .trim_start_matches('/')
            .into(),
        image: config.image.or(value.image).unwrap_or_default(),
        state: state(engine_state.status.as_ref().map_or("", AsRef::as_ref)),
        status: status(&engine_state, now),
        ip: ip(network.networks),
        ports: ports(mapped_ports),
        mounts: mounts(value.mounts),
        labels,
        compose,
        created: value.created.map(|d| d.timestamp()).unwrap_or_default(),
        command,
        env: Some(config.env.unwrap_or_default()),
    }
}

pub(super) fn info(
    value: api::SystemInfo,
    version: api::SystemVersion,
    endpoint: Endpoint,
) -> EngineInfo {
    let platform = version.platform.map(|p| p.name).unwrap_or_default();
    let components = version.components.unwrap_or_default();
    let names: Vec<_> = std::iter::once(platform.as_str())
        .chain(components.iter().map(|c| c.name.as_str()))
        .collect();
    let kind = if names
        .iter()
        .any(|s| s.to_ascii_lowercase().contains("podman"))
    {
        EngineKind::Podman
    } else if names.iter().any(|s| s.contains("Docker Engine")) {
        EngineKind::Docker
    } else {
        EngineKind::Other
    };
    let name = if kind == EngineKind::Podman {
        "Podman".into()
    } else if !platform.is_empty() {
        platform
    } else if kind == EngineKind::Docker {
        "Docker Engine".into()
    } else {
        value.name.unwrap_or_else(|| "Container Engine".into())
    };
    EngineInfo {
        kind,
        name,
        version: version.version.or(value.server_version).unwrap_or_default(),
        api_version: version.api_version.unwrap_or_default(),
        os: version.os.or(value.os_type).unwrap_or_default(),
        arch: version.arch.or(value.architecture).unwrap_or_default(),
        rootless: value
            .security_options
            .unwrap_or_default()
            .iter()
            .any(|s| s.contains("name=rootless")),
        endpoint,
    }
}

pub(super) fn event(value: api::EventMessage) -> Option<EngineEvent> {
    let kind = match value.typ? {
        api::EventMessageTypeEnum::CONTAINER => ObjectKind::Container,
        api::EventMessageTypeEnum::IMAGE => ObjectKind::Image,
        api::EventMessageTypeEnum::VOLUME => ObjectKind::Volume,
        api::EventMessageTypeEnum::NETWORK => ObjectKind::Network,
        _ => return None,
    };
    let action = value.action.unwrap_or_default();
    let action = match action.as_str() {
        "create" => EventAction::Create,
        "start" => EventAction::Start,
        "die" => EventAction::Die,
        "stop" => EventAction::Stop,
        "kill" => EventAction::Kill,
        "pause" => EventAction::Pause,
        "unpause" => EventAction::Unpause,
        "restart" => EventAction::Restart,
        "destroy" => EventAction::Destroy,
        "rename" => EventAction::Rename,
        _ => action.strip_prefix("health_status:").map_or_else(
            || EventAction::Other(action.clone()),
            |status| EventAction::HealthStatus(status.trim().into()),
        ),
    };
    Some(EngineEvent {
        kind,
        id: value.actor.and_then(|a| a.id).unwrap_or_default(),
        action,
        time_nanos: value
            .time_nano
            .unwrap_or_else(|| value.time.unwrap_or_default().saturating_mul(1_000_000_000)),
    })
}

pub(super) fn error(value: bollard::errors::Error) -> EngineError {
    match value {
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            message,
        } => EngineError::NotFound(message),
        bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            message,
        } => EngineError::Conflict(message),
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message,
        } => EngineError::Engine {
            status: status_code,
            message,
        },
        other => EngineError::Transport(other.to_string()),
    }
}

#[cfg(test)]
mod tests;

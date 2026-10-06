use super::*;

fn now() -> DateTime<Utc> {
    "2026-10-06T12:00:00Z".parse().unwrap()
}
fn network(address: &str) -> api::EndpointSettings {
    api::EndpointSettings {
        ip_address: Some(address.into()),
        ..Default::default()
    }
}
fn compose_labels() -> HashMap<String, String> {
    HashMap::from([
        ("z".into(), "last".into()),
        ("com.docker.compose.project".into(), "berth".into()),
        ("com.docker.compose.service".into(), "web".into()),
        (
            "com.docker.compose.project.working_dir".into(),
            "/project".into(),
        ),
        ("a".into(), "first".into()),
    ])
}
fn publications() -> Vec<api::Port> {
    vec![
        api::Port {
            ip: Some("::".into()),
            private_port: 80,
            public_port: Some(8080),
            typ: Some(api::PortTypeEnum::TCP),
        },
        api::Port {
            ip: Some("0.0.0.0".into()),
            private_port: 80,
            public_port: Some(8080),
            typ: Some(api::PortTypeEnum::TCP),
        },
        api::Port {
            ip: Some("127.0.0.1".into()),
            private_port: 80,
            public_port: Some(8080),
            typ: Some(api::PortTypeEnum::TCP),
        },
        api::Port {
            ip: Some("::1".into()),
            private_port: 80,
            public_port: Some(8080),
            typ: Some(api::PortTypeEnum::TCP),
        },
        api::Port {
            private_port: 53,
            typ: Some(api::PortTypeEnum::UDP),
            ..Default::default()
        },
        api::Port {
            private_port: 9000,
            typ: Some(api::PortTypeEnum::SCTP),
            ..Default::default()
        },
    ]
}
fn mount_values() -> Vec<api::MountPoint> {
    vec![
        api::MountPoint {
            typ: Some(api::MountPointTypeEnum::VOLUME),
            name: Some("data".into()),
            source: Some("/var/lib/docker/volumes/data/_data".into()),
            destination: Some("/data".into()),
            rw: Some(true),
            ..Default::default()
        },
        api::MountPoint {
            typ: Some(api::MountPointTypeEnum::BIND),
            source: Some("/host".into()),
            destination: Some("/app".into()),
            rw: Some(false),
            ..Default::default()
        },
        api::MountPoint {
            typ: Some(api::MountPointTypeEnum::TMPFS),
            destination: Some("/tmp".into()),
            rw: Some(true),
            ..Default::default()
        },
        api::MountPoint {
            typ: Some(api::MountPointTypeEnum::NPIPE),
            ..Default::default()
        },
    ]
}

#[test]
fn maps_all_states_and_unknown_defaults() {
    for (value, expected) in [
        ("created", ContainerState::Created),
        ("running", ContainerState::Running),
        ("paused", ContainerState::Paused),
        ("restarting", ContainerState::Restarting),
        ("removing", ContainerState::Removing),
        ("exited", ContainerState::Exited),
        ("dead", ContainerState::Dead),
        ("stopping", ContainerState::Running),
        ("unknown", ContainerState::Created),
        ("", ContainerState::Created),
    ] {
        assert_eq!(state(value), expected);
    }
}

#[test]
fn summary_maps_fields_and_omits_environment() {
    let container = summary(api::ContainerSummary {
        id: Some("abc".into()),
        names: Some(vec!["/web".into(), "/alias".into()]),
        image: Some("nginx:latest".into()),
        state: Some(api::ContainerSummaryStateEnum::RUNNING),
        status: Some("Up 3 hours".into()),
        created: Some(123),
        command: Some("nginx -g daemon off;".into()),
        labels: Some(compose_labels()),
        ports: Some(publications()),
        mounts: Some(mount_values()),
        network_settings: Some(api::ContainerSummaryNetworkSettings {
            networks: Some(HashMap::from([
                ("a-empty".into(), network("")),
                ("b-web".into(), network("172.17.0.2")),
                ("c-other".into(), network("172.18.0.3")),
            ])),
        }),
        ..Default::default()
    });
    assert_eq!(
        (
            container.id.as_str(),
            container.name.as_str(),
            container.image.as_str()
        ),
        ("abc", "web", "nginx:latest")
    );
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.status, "Up 3 hours");
    assert_eq!(container.created, 123);
    assert_eq!(container.command.as_deref(), Some("nginx -g daemon off;"));
    assert_eq!(container.ip.as_deref(), Some("172.17.0.2"));
    assert_eq!(container.env, None);
    assert_eq!(container.labels.first().unwrap().0, "a");
    assert_eq!(container.labels.last().unwrap().0, "z");
    assert!(container.labels.windows(2).all(|p| p[0].0 < p[1].0));
    assert_eq!(
        container.compose,
        Some(ComposeRef {
            project: "berth".into(),
            service: "web".into(),
            working_dir: Some("/project".into())
        })
    );
    assert_eq!(container.ports.len(), 5);
    assert!(
        container
            .ports
            .iter()
            .any(|p| p.host_ip.as_deref() == Some("0.0.0.0"))
    );
    assert!(
        !container
            .ports
            .iter()
            .any(|p| p.host_ip.as_deref() == Some("::"))
    );
    assert!(
        container
            .ports
            .iter()
            .any(|p| p.host_ip.as_deref() == Some("::1"))
    );
    assert_eq!(
        container.mounts,
        vec![
            Mount {
                kind: MountKind::Volume,
                source: "data".into(),
                destination: "/data".into(),
                read_only: false
            },
            Mount {
                kind: MountKind::Bind,
                source: "/host".into(),
                destination: "/app".into(),
                read_only: true
            },
            Mount {
                kind: MountKind::Tmpfs,
                source: String::new(),
                destination: "/tmp".into(),
                read_only: false
            },
        ]
    );
}

#[test]
fn missing_fields_are_safe_and_compose_requires_project_and_service() {
    let container = summary(api::ContainerSummary::default());
    assert_eq!(container.state, ContainerState::Created);
    assert_eq!(container.ports, []);
    assert_eq!(container.mounts, []);
    assert!(container.ip.is_none());
    assert!(container.compose.is_none());
    assert!(
        compose(&labels(Some(HashMap::from([(
            "com.docker.compose.project".into(),
            "p".into()
        )]))))
        .is_none()
    );
    let mut values = compose_labels();
    values.remove("com.docker.compose.project.working_dir");
    assert!(
        compose(&labels(Some(values)))
            .unwrap()
            .working_dir
            .is_none()
    );
    assert_eq!(
        inspect(api::ContainerInspectResponse::default(), now()).env,
        Some(vec![])
    );
}

#[test]
fn ports_dedupe_exact_duplicates_and_preserve_ipv6_only_bindings() {
    let binding = PortBinding {
        host_ip: Some("::".into()),
        host_port: Some(8080),
        container_port: 80,
        protocol: Protocol::Tcp,
    };
    assert_eq!(ports(vec![binding.clone(), binding.clone()]), vec![binding]);
    assert_eq!(protocol("nonsense"), None);
    assert_eq!(protocol("tcp"), Some(Protocol::Tcp));
}

#[test]
fn inspect_maps_environment_creation_command_and_network() {
    let container = inspect(
        api::ContainerInspectResponse {
            id: Some("abc".into()),
            name: Some("/web".into()),
            image: Some("sha256:abc".into()),
            created: Some("2026-10-01T12:00:00Z".parse().unwrap()),
            path: Some("/bin/sh".into()),
            args: Some(vec!["-c".into(), "sleep 60".into()]),
            config: Some(api::ContainerConfig {
                image: Some("alpine:latest".into()),
                env: Some(vec!["A=B".into()]),
                labels: Some(compose_labels()),
                ..Default::default()
            }),
            state: Some(api::ContainerState {
                status: Some(api::ContainerStateStatusEnum::RUNNING),
                started_at: Some("2026-10-06T09:00:00Z".into()),
                ..Default::default()
            }),
            mounts: Some(mount_values()),
            network_settings: Some(api::NetworkSettings {
                networks: Some(HashMap::from([("web".into(), network("172.17.0.2"))])),
                ports: Some(HashMap::from([
                    (
                        "80/tcp".into(),
                        Some(vec![
                            api::PortBinding {
                                host_ip: Some("::".into()),
                                host_port: Some("8080".into()),
                            },
                            api::PortBinding {
                                host_ip: Some("0.0.0.0".into()),
                                host_port: Some("8080".into()),
                            },
                        ]),
                    ),
                    ("53/udp".into(), None),
                    ("9000/sctp".into(), Some(vec![])),
                    ("bad".into(), None),
                    ("bad/tcp".into(), None),
                    ("42/bad".into(), None),
                ])),
                ..Default::default()
            }),
            ..Default::default()
        },
        now(),
    );
    assert_eq!(container.name, "web");
    assert_eq!(container.image, "alpine:latest");
    assert_eq!(container.env, Some(vec!["A=B".into()]));
    assert_eq!(
        container.created,
        "2026-10-01T12:00:00Z"
            .parse::<DateTime<Utc>>()
            .unwrap()
            .timestamp()
    );
    assert_eq!(container.command.as_deref(), Some("/bin/sh -c sleep 60"));
    assert_eq!(container.status, "Up 3 hours");
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.ip.as_deref(), Some("172.17.0.2"));
    assert_eq!(container.ports.len(), 3);
    assert_eq!(
        container
            .ports
            .iter()
            .find(|p| p.container_port == 80)
            .unwrap()
            .host_ip
            .as_deref(),
        Some("0.0.0.0")
    );
    assert!(
        container
            .ports
            .iter()
            .find(|p| p.container_port == 53)
            .unwrap()
            .host_port
            .is_none()
    );
    assert_eq!(container.mounts.len(), 3);
    assert!(container.compose.is_some());
}

#[test]
fn inspect_falls_back_to_config_command_and_image_id() {
    let container = inspect(
        api::ContainerInspectResponse {
            image: Some("sha256:abc".into()),
            config: Some(api::ContainerConfig {
                entrypoint: Some(vec!["/entrypoint".into()]),
                cmd: Some(vec!["arg".into()]),
                ..Default::default()
            }),
            ..Default::default()
        },
        now(),
    );
    assert_eq!(container.image, "sha256:abc");
    assert_eq!(container.command.as_deref(), Some("/entrypoint arg"));
}

#[test]
fn status_uses_fixed_now_for_running_exit_pause_restart_and_health() {
    let mut value = api::ContainerState {
        status: Some(api::ContainerStateStatusEnum::RUNNING),
        started_at: Some("2026-10-06T09:00:00Z".into()),
        finished_at: Some("2026-10-04T12:00:00Z".into()),
        exit_code: Some(137),
        ..Default::default()
    };
    assert_eq!(status(&value, now()), "Up 3 hours");
    value.health = Some(api::Health {
        status: Some(api::HealthStatusEnum::HEALTHY),
        ..Default::default()
    });
    assert_eq!(status(&value, now()), "Up 3 hours (healthy)");
    value.status = Some(api::ContainerStateStatusEnum::PAUSED);
    assert_eq!(status(&value, now()), "Up 3 hours (Paused)");
    value.status = Some(api::ContainerStateStatusEnum::EXITED);
    assert_eq!(status(&value, now()), "Exited (137) 2 days ago");
    value.status = Some(api::ContainerStateStatusEnum::RESTARTING);
    assert_eq!(status(&value, now()), "Restarting (137) 2 days ago");
    value.status = Some(api::ContainerStateStatusEnum::DEAD);
    assert_eq!(status(&value, now()), "Dead");
    value.status = Some(api::ContainerStateStatusEnum::REMOVING);
    assert_eq!(status(&value, now()), "Removing");
    value.status = Some(api::ContainerStateStatusEnum::CREATED);
    assert_eq!(status(&value, now()), "Created");
}

#[test]
fn elapsed_handles_missing_zero_invalid_future_and_unit_boundaries() {
    for value in [None, Some("bad"), Some("0001-01-01T00:00:00Z")] {
        assert_eq!(elapsed(value, now()), None);
    }
    for (seconds, expected) in [
        (0, "Less than a second"),
        (1, "1 second"),
        (59, "59 seconds"),
        (60, "1 minute"),
        (120, "2 minutes"),
        (3600, "1 hour"),
        (7200, "2 hours"),
        (86400, "1 day"),
        (172_800, "2 days"),
        (-60, "Less than a second"),
    ] {
        let time = (now() - chrono::Duration::seconds(seconds)).to_rfc3339();
        assert_eq!(elapsed(Some(&time), now()).as_deref(), Some(expected));
    }
    let running = api::ContainerState {
        status: Some(api::ContainerStateStatusEnum::RUNNING),
        ..Default::default()
    };
    assert_eq!(status(&running, now()), "Up");
    let exited = api::ContainerState {
        status: Some(api::ContainerStateStatusEnum::EXITED),
        ..Default::default()
    };
    assert_eq!(status(&exited, now()), "Exited (0)");
}

#[test]
fn info_detects_docker_podman_other_and_rootless() {
    let endpoint = Endpoint::Unix("/socket".into());
    let value = api::SystemInfo {
        security_options: Some(vec![
            "name=seccomp,profile=builtin".into(),
            "name=rootless".into(),
        ]),
        ..Default::default()
    };
    let version = api::SystemVersion {
        platform: Some(api::SystemVersionPlatform {
            name: "Docker Engine - Community".into(),
        }),
        version: Some("29.0".into()),
        api_version: Some("1.53".into()),
        os: Some("linux".into()),
        arch: Some("amd64".into()),
        ..Default::default()
    };
    let mapped = info(value, version.clone(), endpoint.clone());
    assert_eq!(mapped.kind, EngineKind::Docker);
    assert_eq!(mapped.name, "Docker Engine - Community");
    assert_eq!(
        (
            mapped.version.as_str(),
            mapped.api_version.as_str(),
            mapped.os.as_str(),
            mapped.arch.as_str()
        ),
        ("29.0", "1.53", "linux", "amd64")
    );
    assert!(mapped.rootless);
    assert_eq!(mapped.endpoint, endpoint);
    let mut podman = version;
    podman.components = Some(vec![api::SystemVersionComponents {
        name: "Podman Engine".into(),
        ..Default::default()
    }]);
    assert_eq!(
        info(api::SystemInfo::default(), podman, endpoint.clone()).kind,
        EngineKind::Podman
    );
    let podman = api::SystemVersion {
        platform: Some(api::SystemVersionPlatform {
            name: "podman".into(),
        }),
        ..Default::default()
    };
    assert_eq!(
        info(api::SystemInfo::default(), podman, endpoint.clone()).name,
        "Podman"
    );
    let other = info(
        api::SystemInfo {
            name: Some("Colima".into()),
            server_version: Some("v1".into()),
            os_type: Some("linux".into()),
            architecture: Some("arm64".into()),
            ..Default::default()
        },
        api::SystemVersion::default(),
        endpoint,
    );
    assert_eq!(other.kind, EngineKind::Other);
    assert_eq!(other.name, "Colima");
    assert_eq!(
        (
            other.version.as_str(),
            other.os.as_str(),
            other.arch.as_str()
        ),
        ("v1", "linux", "arm64")
    );
    assert!(!other.rootless);
}

#[test]
fn events_map_object_types_actions_ids_and_timestamps() {
    for (typ, kind) in [
        (api::EventMessageTypeEnum::CONTAINER, ObjectKind::Container),
        (api::EventMessageTypeEnum::IMAGE, ObjectKind::Image),
        (api::EventMessageTypeEnum::VOLUME, ObjectKind::Volume),
        (api::EventMessageTypeEnum::NETWORK, ObjectKind::Network),
    ] {
        for (value, action) in [
            ("create", EventAction::Create),
            ("start", EventAction::Start),
            ("die", EventAction::Die),
            ("stop", EventAction::Stop),
            ("kill", EventAction::Kill),
            ("pause", EventAction::Pause),
            ("unpause", EventAction::Unpause),
            ("restart", EventAction::Restart),
            ("destroy", EventAction::Destroy),
            ("rename", EventAction::Rename),
            (
                "health_status: healthy",
                EventAction::HealthStatus("healthy".into()),
            ),
            (
                "exec_start: sh",
                EventAction::Other("exec_start: sh".into()),
            ),
            ("exec_create", EventAction::Other("exec_create".into())),
            ("pull", EventAction::Other("pull".into())),
        ] {
            let mapped = event(api::EventMessage {
                typ: Some(typ),
                action: Some(value.into()),
                actor: Some(api::EventActor {
                    id: Some("id".into()),
                    ..Default::default()
                }),
                time: Some(100),
                time_nano: Some(100_123),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(
                mapped,
                EngineEvent {
                    kind,
                    id: "id".into(),
                    action,
                    time_nanos: 100_123
                }
            );
        }
    }
    let mapped = event(api::EventMessage {
        typ: Some(api::EventMessageTypeEnum::VOLUME),
        time: Some(42),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(mapped.time_nanos, 42_000_000_000);
    for typ in [
        None,
        Some(api::EventMessageTypeEnum::DAEMON),
        Some(api::EventMessageTypeEnum::BUILDER),
        Some(api::EventMessageTypeEnum::SERVICE),
    ] {
        assert!(
            event(api::EventMessage {
                typ,
                ..Default::default()
            })
            .is_none()
        );
    }
}

#[test]
fn errors_preserve_engine_messages_and_classify_transport() {
    for (code, expected) in [
        (404, EngineError::NotFound("verbatim engine message".into())),
        (409, EngineError::Conflict("verbatim engine message".into())),
        (
            500,
            EngineError::Engine {
                status: 500,
                message: "verbatim engine message".into(),
            },
        ),
    ] {
        assert_eq!(
            error(bollard::errors::Error::DockerResponseServerError {
                status_code: code,
                message: "verbatim engine message".into()
            }),
            expected
        );
    }
    assert!(matches!(
        error(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into()),
        EngineError::Transport(_)
    ));
    assert!(matches!(
        error(bollard::errors::Error::RequestTimeoutError),
        EngineError::Transport(_)
    ));
}

#[test]
fn images_normalize_tags_unknown_counts_and_sorted_labels() {
    let mapped = image(api::ImageSummary {
        id: "sha256:0123456789abcdef".into(),
        repo_tags: vec!["<none>:<none>".into(), "postgres:17-alpine".into()],
        size: 275_000_000,
        created: 1_790_000_000,
        containers: 2,
        labels: HashMap::from([("z".into(), "last".into()), ("a".into(), "first".into())]),
        ..Default::default()
    });
    assert_eq!(mapped.id, "sha256:0123456789abcdef");
    assert_eq!(mapped.repo_tags, ["postgres:17-alpine"]);
    assert_eq!(mapped.size, 275_000_000);
    assert_eq!(mapped.created, 1_790_000_000);
    assert_eq!(mapped.containers, Some(2));
    assert_eq!(
        mapped.labels,
        [("a".into(), "first".into()), ("z".into(), "last".into())]
    );
    let unknown = image(api::ImageSummary {
        repo_tags: vec!["<none>:<none>".into()],
        containers: -1,
        size: -1,
        ..Default::default()
    });
    assert!(unknown.is_dangling());
    assert_eq!(unknown.containers, None);
    assert_eq!(unknown.size, 0);
    assert!(image(api::ImageSummary::default()).is_dangling());
    assert_eq!(
        image(api::ImageSummary {
            containers: i64::MAX,
            ..Default::default()
        })
        .containers,
        None
    );
}

#[test]
fn volumes_preserve_metadata_and_unknown_usage() {
    let mapped = volume(api::Volume {
        name: "berth_pgdata".into(),
        driver: "local".into(),
        mountpoint: "/var/lib/docker/volumes/berth_pgdata/_data".into(),
        created_at: Some("2026-10-06T12:00:00+02:00".parse().unwrap()),
        labels: compose_labels(),
        usage_data: Some(api::VolumeUsageData {
            size: 350_000_000,
            ref_count: 2,
        }),
        ..Default::default()
    });
    assert_eq!(mapped.name, "berth_pgdata");
    assert_eq!(mapped.driver, "local");
    assert_eq!(
        mapped.mountpoint,
        "/var/lib/docker/volumes/berth_pgdata/_data"
    );
    assert_eq!(mapped.created.as_deref(), Some("2026-10-06T10:00:00+00:00"));
    assert_eq!(mapped.compose_project(), Some("berth"));
    assert!(
        mapped
            .labels
            .windows(2)
            .all(|pair| pair.first().unwrap().0 <= pair.get(1).unwrap().0)
    );
    assert_eq!(
        (mapped.size, mapped.ref_count),
        (Some(350_000_000), Some(2))
    );
    for usage in [
        None,
        Some(api::VolumeUsageData {
            size: -1,
            ref_count: -1,
        }),
    ] {
        let mapped = volume(api::Volume {
            usage_data: usage,
            ..Default::default()
        });
        assert_eq!((mapped.size, mapped.ref_count), (None, None));
    }
    let zero = volume(api::Volume {
        usage_data: Some(api::VolumeUsageData {
            size: 0,
            ref_count: 0,
        }),
        ..Default::default()
    });
    assert_eq!((zero.size, zero.ref_count), (Some(0), Some(0)));
}

#[test]
fn disk_usage_maps_items_and_excludes_busy_or_shared_cache_from_reclaimable() {
    let cache = |size, in_use, shared| api::BuildCache {
        size: Some(size),
        in_use: Some(in_use),
        shared: Some(shared),
        ..Default::default()
    };
    let mapped = disk_usage(api::SystemDataUsageResponse {
        images: Some(vec![api::ImageSummary {
            containers: 3,
            size: 100,
            ..Default::default()
        }]),
        volumes: Some(vec![api::Volume {
            usage_data: Some(api::VolumeUsageData {
                size: 200,
                ref_count: 1,
            }),
            ..Default::default()
        }]),
        build_cache: Some(vec![
            cache(100, false, false),
            cache(200, true, false),
            cache(300, false, true),
            cache(-1, false, false),
        ]),
        ..Default::default()
    });
    assert_eq!(mapped.images.first().unwrap().containers, Some(3));
    assert_eq!(mapped.volumes.first().unwrap().size, Some(200));
    assert_eq!(mapped.volumes.first().unwrap().ref_count, Some(1));
    assert_eq!(mapped.build_cache_bytes, 600);
    assert_eq!(mapped.build_cache_reclaimable_bytes, 100);
    let empty = disk_usage(api::SystemDataUsageResponse::default());
    assert!(empty.images.is_empty() && empty.volumes.is_empty());
    assert_eq!(
        (empty.build_cache_bytes, empty.build_cache_reclaimable_bytes),
        (0, 0)
    );
    let saturated = disk_usage(api::SystemDataUsageResponse {
        build_cache: Some(vec![cache(i64::MAX, false, false); 3]),
        ..Default::default()
    });
    assert_eq!(saturated.build_cache_bytes, u64::MAX);
    assert_eq!(saturated.build_cache_reclaimable_bytes, u64::MAX);
}

#[test]
fn prune_responses_preserve_ids_bytes_and_image_untagged_fallback() {
    assert_eq!(
        pruned_images(api::ImagePruneResponse {
            images_deleted: Some(vec![
                api::ImageDeleteResponseItem {
                    deleted: Some("sha256:a".into()),
                    untagged: Some("unused:tag".into())
                },
                api::ImageDeleteResponseItem {
                    deleted: None,
                    untagged: Some("fallback:tag".into())
                },
                api::ImageDeleteResponseItem::default(),
            ]),
            space_reclaimed: Some(42),
        }),
        PruneReport {
            deleted: vec!["sha256:a".into(), "fallback:tag".into()],
            reclaimed_bytes: 42
        }
    );
    for report in [
        pruned_containers(api::ContainerPruneResponse {
            containers_deleted: Some(vec!["id".into()]),
            space_reclaimed: Some(7),
        }),
        pruned_volumes(api::VolumePruneResponse {
            volumes_deleted: Some(vec!["id".into()]),
            space_reclaimed: Some(7),
        }),
        pruned_build_cache(api::BuildPruneResponse {
            caches_deleted: Some(vec!["id".into()]),
            space_reclaimed: Some(7),
        }),
    ] {
        assert_eq!(
            report,
            PruneReport {
                deleted: vec!["id".into()],
                reclaimed_bytes: 7
            }
        );
    }
    assert_eq!(
        pruned_networks(api::NetworkPruneResponse {
            networks_deleted: Some(vec!["network".into()])
        }),
        PruneReport {
            deleted: vec!["network".into()],
            reclaimed_bytes: 0
        }
    );
    for report in [
        pruned_images(api::ImagePruneResponse::default()),
        pruned_containers(api::ContainerPruneResponse::default()),
        pruned_volumes(api::VolumePruneResponse {
            space_reclaimed: Some(-1),
            ..Default::default()
        }),
        pruned_networks(api::NetworkPruneResponse::default()),
        pruned_build_cache(api::BuildPruneResponse::default()),
    ] {
        assert_eq!(
            report,
            PruneReport {
                deleted: vec![],
                reclaimed_bytes: 0
            }
        );
    }
}

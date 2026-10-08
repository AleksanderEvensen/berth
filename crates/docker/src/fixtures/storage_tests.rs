use super::*;
use futures::executor::block_on;

fn event(events: &mut BoxStream<'static, EngineResult<EngineEvent>>, kind: ObjectKind, id: &str) {
    let event = block_on(events.next()).unwrap().unwrap();
    assert_eq!(event.kind, kind);
    assert_eq!(event.action, EventAction::Destroy);
    assert_eq!(event.id, id);
}

#[test]
fn inventory_matches_all_containers_and_fast_volume_list_omits_usage() {
    let engine = FixtureEngine::default();
    let usage = block_on(engine.disk_usage()).unwrap();
    assert_eq!(usage.images, block_on(engine.list_images()).unwrap());
    for container in containers() {
        assert!(
            usage
                .images
                .iter()
                .any(|image| image.repo_tags.contains(&container.image)
                    && image.containers.is_some_and(|n| n > 0))
        );
        for mount in container
            .mounts
            .iter()
            .filter(|m| m.kind == MountKind::Volume)
        {
            assert!(usage.volumes.iter().any(
                |volume| volume.name == mount.source && volume.ref_count.is_some_and(|n| n > 0)
            ));
        }
    }
    assert!(
        usage
            .volumes
            .iter()
            .all(|v| v.size.is_some() && v.ref_count.is_some())
    );
    assert!(
        block_on(engine.list_volumes())
            .unwrap()
            .iter()
            .all(|v| v.size.is_none() && v.ref_count.is_none())
    );
    assert_eq!(usage.images.iter().filter(|i| i.is_dangling()).count(), 4);
    assert!(usage.images.iter().all(|i| i.id.len() == 71));
    assert!(
        usage
            .volumes
            .iter()
            .any(|v| v.name.len() == 64 && v.ref_count == Some(0))
    );
}

#[test]
fn removals_enforce_references_including_stopped_containers_and_emit_events() {
    let engine = FixtureEngine::default();
    let mut events = engine.events();
    assert!(matches!(
        block_on(engine.remove_image("postgres:17.6", false)),
        Err(EngineError::Conflict(_))
    ));
    assert!(matches!(
        block_on(engine.remove_volume("photon_dev_db_data", false)),
        Err(EngineError::Conflict(_))
    ));
    assert!(events.next().now_or_never().is_none());
    let image = block_on(engine.list_images())
        .unwrap()
        .into_iter()
        .find(|i| i.repo_tags.contains(&"postgres:17.6".into()))
        .unwrap();
    block_on(engine.remove_image(&image.id, true)).unwrap();
    event(&mut events, ObjectKind::Image, &image.id);
    block_on(engine.remove_volume("photon_dev_db_data", true)).unwrap();
    event(&mut events, ObjectKind::Volume, "photon_dev_db_data");
    assert!(matches!(
        block_on(engine.remove_image(&image.id, false)),
        Err(EngineError::NotFound(_))
    ));
    assert!(matches!(
        block_on(engine.remove_volume("photon_dev_db_data", false)),
        Err(EngineError::NotFound(_))
    ));
}

#[test]
fn unused_removals_and_latency_delay_changes_and_events() {
    let engine = FixtureEngine::default().with_latency(Duration::from_millis(100));
    let mut events = engine.events();
    let image = block_on(engine.list_images())
        .unwrap()
        .into_iter()
        .find(Image::is_dangling)
        .unwrap();
    let removal = engine.remove_image(&image.id, false);
    assert!(
        block_on(engine.list_images())
            .unwrap()
            .iter()
            .any(|i| i.id == image.id)
    );
    assert!(events.next().now_or_never().is_none());
    block_on(removal).unwrap();
    event(&mut events, ObjectKind::Image, &image.id);
    block_on(engine.remove_volume("scratch_maven-cache", false)).unwrap();
    event(&mut events, ObjectKind::Volume, "scratch_maven-cache");
}

#[test]
fn image_prunes_respect_modes_report_bytes_and_emit_destroy() {
    for mode in [ImagePrune::Dangling, ImagePrune::AllUnused] {
        let engine = FixtureEngine::default();
        let mut events = engine.events();
        let before = block_on(engine.list_images()).unwrap();
        let expected: Vec<_> = before
            .iter()
            .filter(|i| {
                i.containers == Some(0) && (mode == ImagePrune::AllUnused || i.is_dangling())
            })
            .collect();
        let report = block_on(engine.prune_images(mode)).unwrap();
        assert_eq!(
            report.deleted,
            expected.iter().map(|i| i.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            report.reclaimed_bytes,
            expected.iter().map(|i| i.size).sum::<u64>()
        );
        for id in &report.deleted {
            event(&mut events, ObjectKind::Image, id);
        }
        assert_eq!(
            block_on(engine.prune_images(mode)).unwrap().deleted,
            Vec::<String>::new()
        );
        let remaining = block_on(engine.list_images()).unwrap();
        assert!(
            remaining
                .iter()
                .all(|i| i.containers != Some(0)
                    || (mode == ImagePrune::Dangling && !i.is_dangling()))
        );
    }
}

#[test]
fn pruning_containers_updates_usage_then_volumes_include_named_and_anonymous() {
    let engine = FixtureEngine::default();
    let mut events = engine.events();
    let before = block_on(engine.list_containers()).unwrap();
    let report = block_on(engine.prune_containers()).unwrap();
    assert_eq!(
        report.deleted,
        before
            .iter()
            .filter(|c| !c.state.is_active())
            .map(|c| c.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        report.reclaimed_bytes,
        u64::try_from(report.deleted.len())
            .unwrap()
            .saturating_mul(CONTAINER_BYTES)
    );
    for id in &report.deleted {
        event(&mut events, ObjectKind::Container, id);
    }
    let usage = block_on(engine.disk_usage()).unwrap();
    assert!(
        usage
            .images
            .iter()
            .any(|i| i.repo_tags.contains(&"postgres:17.6".into()) && i.containers == Some(0))
    );
    assert!(
        usage
            .volumes
            .iter()
            .any(|v| v.name == "photon_dev_db_data" && v.ref_count == Some(0))
    );
    let expected: Vec<_> = usage
        .volumes
        .iter()
        .filter(|v| v.ref_count == Some(0))
        .collect();
    let report = block_on(engine.prune_volumes()).unwrap();
    assert_eq!(
        report.deleted,
        expected.iter().map(|v| v.name.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        report.reclaimed_bytes,
        expected.iter().filter_map(|v| v.size).sum::<u64>()
    );
    assert!(report.deleted.contains(&"photon_dev_db_data".into()));
    assert!(report.deleted.contains(&"scratch_maven-cache".into()));
    assert!(report.deleted.iter().any(|name| name.len() == 64));
    for id in &report.deleted {
        event(&mut events, ObjectKind::Volume, id);
    }
    assert_eq!(
        block_on(engine.prune_volumes()).unwrap().deleted,
        Vec::<String>::new()
    );
    assert!(
        block_on(engine.disk_usage())
            .unwrap()
            .volumes
            .iter()
            .all(|v| v.ref_count.is_some_and(|n| n > 0))
    );
}

#[test]
fn network_and_cache_prunes_are_stateful_and_keep_used_resources() {
    let engine = FixtureEngine::default();
    let mut events = engine.events();
    let report = block_on(engine.prune_networks()).unwrap();
    // Stopped containers hold no endpoints, so photon_dev's network goes too.
    assert_eq!(
        report.deleted,
        [
            "blue-zone-workshop_default",
            "photon_dev_default",
            "scratch_default"
        ]
    );
    assert_eq!(report.reclaimed_bytes, 0);
    for name in &report.deleted {
        let id = networks().into_iter().find(|n| &n.name == name).unwrap().id;
        event(&mut events, ObjectKind::Network, &id);
    }
    let left: Vec<_> = block_on(engine.list_networks())
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(
        left,
        [
            "bridge",
            "host",
            "none",
            "berth-dev_default",
            "tihlde-voting_default"
        ]
    );
    assert_eq!(
        block_on(engine.prune_networks()).unwrap().deleted,
        Vec::<String>::new()
    );
    let usage = block_on(engine.disk_usage()).unwrap();
    let report = block_on(engine.prune_build_cache()).unwrap();
    assert_eq!(report.deleted, ["cache-api", "cache-workshop"]);
    assert_eq!(report.reclaimed_bytes, usage.build_cache_reclaimable_bytes);
    let after = block_on(engine.disk_usage()).unwrap();
    assert_eq!(after.build_cache_reclaimable_bytes, 0);
    assert_eq!(
        after.build_cache_bytes,
        usage
            .build_cache_bytes
            .saturating_sub(report.reclaimed_bytes)
    );
    assert_eq!(
        block_on(engine.prune_build_cache()).unwrap().deleted,
        Vec::<String>::new()
    );
}

#[test]
fn custom_container_inventory_is_consistent_and_instances_are_isolated() {
    let mut container = containers().into_iter().next().unwrap();
    container.image = "custom-image:test".into();
    container.mounts = vec![volume("custom-volume", "/data")];
    let engine = FixtureEngine::new(vec![container]);
    assert!(
        block_on(engine.list_images())
            .unwrap()
            .iter()
            .any(|i| i.repo_tags == ["custom-image:test"] && i.containers == Some(1))
    );
    assert!(
        block_on(engine.disk_usage())
            .unwrap()
            .volumes
            .iter()
            .any(|v| v.name == "custom-volume" && v.ref_count == Some(1))
    );
    let other = FixtureEngine::default();
    block_on(engine.prune_images(ImagePrune::AllUnused)).unwrap();
    assert_eq!(block_on(other.list_images()).unwrap(), images());
}

#[test]
fn dangling_images_referenced_by_id_are_kept_and_removed_containers_release_resources() {
    let engine = FixtureEngine::default();
    let dangling = block_on(engine.list_images())
        .unwrap()
        .into_iter()
        .find(Image::is_dangling)
        .unwrap();
    let container_id = engine.with_state(|state| {
        let container = state.containers.first_mut().unwrap();
        container.image.clone_from(&dangling.id);
        container.id.clone()
    });
    let report = block_on(engine.prune_images(ImagePrune::Dangling)).unwrap();
    assert!(!report.deleted.contains(&dangling.id));
    assert_eq!(
        block_on(engine.list_images())
            .unwrap()
            .into_iter()
            .find(|i| i.id == dangling.id)
            .unwrap()
            .containers,
        Some(1)
    );
    block_on(engine.remove_container(
        &container_id,
        RemoveOptions {
            force: true,
            volumes: false,
        },
    ))
    .unwrap();
    let usage = block_on(engine.disk_usage()).unwrap();
    assert_eq!(
        usage
            .images
            .into_iter()
            .find(|i| i.id == dangling.id)
            .unwrap()
            .containers,
        Some(0)
    );
    assert_eq!(
        usage
            .volumes
            .into_iter()
            .find(|v| v.name == "tihlde-voting_pgdata")
            .unwrap()
            .ref_count,
        Some(0)
    );
    assert_eq!(
        block_on(engine.prune_images(ImagePrune::Dangling))
            .unwrap()
            .deleted,
        [dangling.id]
    );
}

#[test]
fn mounted_resources_do_not_duplicate_the_unused_fixture_inventory() {
    let mut container = containers().into_iter().next().unwrap();
    container.image = "apache/kafka:4.1.0".into();
    container.mounts = vec![volume("scratch_maven-cache", "/cache")];
    let engine = FixtureEngine::new(vec![container]);
    let usage = block_on(engine.disk_usage()).unwrap();
    assert_eq!(
        usage
            .images
            .iter()
            .filter(|i| i.repo_tags.contains(&"apache/kafka:4.1.0".into()))
            .count(),
        1
    );
    assert_eq!(
        usage
            .volumes
            .iter()
            .filter(|v| v.name == "scratch_maven-cache")
            .count(),
        1
    );
    assert!(
        !block_on(engine.prune_volumes())
            .unwrap()
            .deleted
            .contains(&"scratch_maven-cache".into())
    );
}

#[test]
fn networks_list_running_endpoints_and_guard_removal() {
    let engine = FixtureEngine::default();
    let mut events = engine.events();
    let list = block_on(engine.list_networks()).unwrap();
    let find = |name: &str| list.iter().find(|n| n.name == name).unwrap().clone();

    let bridge = find("bridge");
    assert!(bridge.is_builtin(EngineKind::Docker));
    let names: Vec<_> = bridge.endpoints.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["rustfs"]);
    assert_eq!(bridge.endpoints[0].ipv4.as_deref(), Some("172.17.0.2/16"));

    let berth = find("berth-dev_default");
    assert_eq!(berth.compose_project(), Some("berth-dev"));
    assert_eq!(berth.subnets[0].subnet, "172.21.0.0/16");
    // The restarting worker counts as active.
    assert_eq!(berth.endpoints.len(), 3);
    assert_eq!(find("photon_dev_default").endpoints, []);
    assert_eq!(list.iter().filter(|n| n.id.len() == 64).count(), list.len());

    for network in [&bridge, &berth] {
        assert!(matches!(
            block_on(engine.remove_network(&network.id)),
            Err(EngineError::Engine { status: 403, .. })
        ));
    }
    let photon = find("photon_dev_default");
    block_on(engine.remove_network(&photon.id)).unwrap();
    event(&mut events, ObjectKind::Network, &photon.id);
    assert!(matches!(
        block_on(engine.remove_network(&photon.id)),
        Err(EngineError::NotFound(_))
    ));
}

#[test]
fn inspect_image_reports_config_and_unknown_ids_are_not_found() {
    let engine = FixtureEngine::default();
    let images = block_on(engine.list_images()).unwrap();
    let postgres = images
        .iter()
        .find(|i| i.repo_tags.iter().any(|t| t.contains("postgres:")))
        .unwrap();
    let details = block_on(engine.inspect_image(&postgres.id)).unwrap();
    assert_eq!(details.id, postgres.id);
    assert_eq!(details.platform.as_deref(), Some("linux/amd64"));
    assert_eq!(details.command, ["postgres"]);
    assert_eq!(
        details.exposed_ports,
        [ExposedPort {
            port: 5432,
            protocol: Protocol::Tcp
        }]
    );
    let dangling = images.iter().find(|i| i.is_dangling()).unwrap();
    let details = block_on(engine.inspect_image(&dangling.id)).unwrap();
    assert_eq!(details.exposed_ports, []);
    assert!(matches!(
        block_on(engine.inspect_image("sha256:missing")),
        Err(EngineError::NotFound(_))
    ));
}

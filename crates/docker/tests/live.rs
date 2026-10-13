//! Opt-in tests against a real Docker-compatible daemon.
//! Run: cargo test -p berth-docker --test live -- --ignored
#![cfg(feature = "live")]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use berth_docker::{
    BollardEngine, ContainerAction, ContainerState, Endpoint, Engine, EngineError, EngineEvent,
    EventAction, ObjectKind, RemoveOptions, TlsConfig,
};
use bollard::{
    API_DEFAULT_VERSION, Docker,
    models::{ContainerCreateBody, NetworkCreateRequest, VolumeCreateOptions},
    query_parameters as query,
};
use futures::{FutureExt as _, StreamExt as _, channel::oneshot, executor::block_on};
use tokio::runtime::Runtime;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn endpoint() -> TestResult<Endpoint> {
    let host =
        std::env::var("DOCKER_HOST").unwrap_or_else(|_| "unix:///var/run/docker.sock".into());
    if let Some(path) = host.strip_prefix("unix://") {
        return Ok(Endpoint::Unix(path.into()));
    }
    if let Some(pipe) = host.strip_prefix("npipe://") {
        return Ok(Endpoint::NamedPipe(pipe.into()));
    }
    let verify = std::env::var("DOCKER_TLS_VERIFY").is_ok_and(|v| !v.is_empty());
    let tls = if verify || host.starts_with("https://") {
        Some(TlsConfig {
            cert_dir: std::env::var_os("DOCKER_CERT_PATH")
                .map(Into::into)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| std::path::PathBuf::from(home).join(".docker"))
                })
                .ok_or("HOME or DOCKER_CERT_PATH is needed for TLS")?,
            verify: true,
        })
    } else {
        None
    };
    Ok(Endpoint::Tcp { url: host, tls })
}

fn direct_client(endpoint: &Endpoint) -> TestResult<Docker> {
    Ok(match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(path) => Docker::connect_with_unix(
            path.to_str().ok_or("socket path is not UTF-8")?,
            30,
            API_DEFAULT_VERSION,
        )?,
        #[cfg(windows)]
        Endpoint::NamedPipe(pipe) => {
            Docker::connect_with_named_pipe(pipe, 30, API_DEFAULT_VERSION)?
        }
        Endpoint::Tcp { url, tls: None } => {
            Docker::connect_with_http(url, 30, API_DEFAULT_VERSION)?
        }
        Endpoint::Tcp {
            url,
            tls: Some(config),
        } => Docker::connect_with_ssl(
            url,
            &config.cert_dir.join("key.pem"),
            &config.cert_dir.join("cert.pem"),
            &config.cert_dir.join("ca.pem"),
            30,
            API_DEFAULT_VERSION,
        )?,
        #[allow(unreachable_patterns)]
        _ => return Err("endpoint unsupported on this test platform".into()),
    })
}

/// Installed before create: cleanup by the unique name also covers a failed
/// create response after the daemon has actually created the container.
struct Cleanup<'a> {
    runtime: &'a Runtime,
    docker: &'a Docker,
    name: String,
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let result = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                self.docker.remove_container(
                    &self.name,
                    Some(query::RemoveContainerOptions {
                        force: true,
                        v: true,
                        ..Default::default()
                    }),
                ),
            )
            .await
        });
        match result {
            Ok(
                Ok(())
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }),
            ) => {}
            other => eprintln!("cleanup failed for {}: {other:?}", self.name),
        }
    }
}

struct Events {
    receiver: std::sync::mpsc::Receiver<berth_docker::EngineResult<EngineEvent>>,
    cancel: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Events {
    fn subscribe(engine: &impl Engine) -> Self {
        let mut stream = engine.events();
        let (sender, receiver) = std::sync::mpsc::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            block_on(async move {
                let mut cancel = cancel_rx.fuse();
                loop {
                    futures::select! {
                        _ = cancel => break,
                        event = stream.next().fuse() => match event {
                            Some(event) => if sender.send(event).is_err() { break; },
                            None => break,
                        }
                    }
                }
            });
        });
        Self {
            receiver,
            cancel: Some(cancel_tx),
            thread: Some(thread),
        }
    }
    fn expect(&self, id: &str, action: &EventAction) -> TestResult<()> {
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(15))
            .ok_or("event deadline overflow")?;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = self.receiver.recv_timeout(remaining)??;
            if event.kind == ObjectKind::Container && event.id == id && &event.action == action {
                return Ok(());
            }
        }
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("event listener thread panicked");
        }
    }
}

fn ensure_test_image(runtime: &Runtime, docker: &Docker) -> TestResult<()> {
    runtime.block_on(async {
        // Pull if absent; draining the stream completes the image download.
        if docker.inspect_image("alpine:latest").await.is_err() {
            let mut pull = docker.create_image(
                Some(query::CreateImageOptions {
                    from_image: Some("alpine".into()),
                    tag: Some("latest".into()),
                    ..Default::default()
                }),
                None,
                None,
            );
            while let Some(result) = pull.next().await {
                result?;
            }
        }
        Ok(())
    })
}

fn create_test_container(runtime: &Runtime, docker: &Docker, name: &str) -> TestResult<String> {
    let created = runtime.block_on(docker.create_container(
        Some(query::CreateContainerOptions {
            name: Some(name.to_string()),
            ..Default::default()
        }),
        ContainerCreateBody {
            image: Some("alpine:latest".into()),
            cmd: Some(vec!["sleep".into(), "300".into()]),
            env: Some(vec!["BERTH_TEST=live".into()]),
            ..Default::default()
        },
    ))?;
    Ok(created.id)
}

fn test_client(endpoint: &Endpoint) -> TestResult<(Runtime, Docker)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let docker = runtime.block_on(direct_client(endpoint)?.negotiate_version())?;
    Ok((runtime, docker))
}

fn test_container_name() -> TestResult<String> {
    Ok(format!(
        "berth-live-test-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ))
}

#[test]
#[ignore = "requires a real engine and permission to create an alpine:latest container"]
fn live_container_lifecycle_and_events_on_a_non_tokio_executor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let endpoint = endpoint().unwrap();
    let engine =
        BollardEngine::connect(&endpoint).expect("connect to DOCKER_HOST or /var/run/docker.sock");
    let info = block_on(engine.info()).unwrap();
    assert_ne!(info.version, "");
    assert_ne!(info.api_version, "");
    assert_eq!(info.endpoint, endpoint);

    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    let events = Events::subscribe(&engine);
    let name = test_container_name().unwrap();
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let id = create_test_container(&runtime, &docker, &name).unwrap();
    let listed = block_on(engine.list_containers()).unwrap();
    let listed = listed
        .iter()
        .find(|c| c.id == id)
        .expect("created container is listed");
    assert_eq!(listed.name, name);
    assert_eq!(listed.state, ContainerState::Created);
    assert!(listed.env.is_none());
    let inspected = block_on(engine.inspect_container(&id)).unwrap();
    assert!(inspected.env.unwrap().contains(&"BERTH_TEST=live".into()));

    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    events.expect(&id, &EventAction::Start).unwrap();
    // Starting an already running container must also succeed (HTTP 304).
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Running
    );
    assert!(matches!(
        block_on(engine.remove_container(&id, RemoveOptions::default())),
        Err(EngineError::Conflict(_))
    ));

    block_on(engine.container_action(&id, ContainerAction::Pause)).unwrap();
    events.expect(&id, &EventAction::Pause).unwrap();
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Paused
    );
    block_on(engine.container_action(&id, ContainerAction::Unpause)).unwrap();
    events.expect(&id, &EventAction::Unpause).unwrap();
    block_on(engine.container_action(
        &id,
        ContainerAction::Restart {
            timeout_secs: Some(1),
        },
    ))
    .unwrap();
    events.expect(&id, &EventAction::Restart).unwrap();
    block_on(engine.container_action(&id, ContainerAction::Kill)).unwrap();
    events.expect(&id, &EventAction::Die).unwrap();
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Exited
    );
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    events.expect(&id, &EventAction::Start).unwrap();
    block_on(engine.container_action(
        &id,
        ContainerAction::Stop {
            timeout_secs: Some(1),
        },
    ))
    .unwrap();
    events.expect(&id, &EventAction::Stop).unwrap();
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Exited
    );
    block_on(engine.remove_container(
        &id,
        RemoveOptions {
            force: false,
            volumes: true,
        },
    ))
    .unwrap();
    events.expect(&id, &EventAction::Destroy).unwrap();
    assert!(matches!(
        block_on(engine.inspect_container(&id)),
        Err(EngineError::NotFound(_))
    ));
    assert!(
        !block_on(engine.list_containers())
            .unwrap()
            .iter()
            .any(|c| c.id == id)
    );
}

/// Best-effort cleanup is installed before volume creation, including when the
/// create response fails after the daemon has already created the volume.
struct VolumeCleanup<'a> {
    runtime: &'a Runtime,
    docker: &'a Docker,
    name: String,
}

impl Drop for VolumeCleanup<'_> {
    fn drop(&mut self) {
        let result = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                self.docker.remove_volume(
                    &self.name,
                    Some(query::RemoveVolumeOptions { force: false }),
                ),
            )
            .await
        });
        match result {
            Ok(
                Ok(())
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }),
            ) => {}
            other => eprintln!("volume cleanup failed for {}: {other:?}", self.name),
        }
    }
}

#[test]
#[ignore = "requires a real engine and a uniquely named throwaway volume"]
fn live_volume_disk_usage_on_a_non_tokio_executor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    let name = test_container_name()
        .unwrap()
        .replace("berth-live-test-", "berth-live-volume-");
    let label = "dev.berth.integration-test";
    let _cleanup = VolumeCleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    runtime
        .block_on(docker.create_volume(VolumeCreateOptions {
            name: Some(name.clone()),
            labels: Some(std::collections::HashMap::from([(
                label.into(),
                name.clone(),
            )])),
            ..Default::default()
        }))
        .unwrap();
    let volumes = block_on(engine.list_volumes()).unwrap();
    let volume = volumes
        .iter()
        .find(|v| v.name == name)
        .ok_or("throwaway volume not listed")
        .unwrap();
    assert!(volume.labels.contains(&(label.into(), name.clone())));
    assert_eq!((volume.size, volume.ref_count), (None, None));
    let usage = block_on(engine.disk_usage()).unwrap();
    let volume = usage
        .volumes
        .iter()
        .find(|v| v.name == name)
        .ok_or("throwaway volume missing from disk usage")
        .unwrap();
    assert_eq!(volume.size, Some(0));
    assert_eq!(volume.ref_count, Some(0));
    assert!(volume.labels.contains(&(label.into(), name.clone())));
    let images = block_on(engine.list_images()).unwrap();
    assert!(
        images
            .iter()
            .all(|i| !i.repo_tags.iter().any(|tag| tag == "<none>:<none>"))
    );
    block_on(engine.remove_volume(&name, false)).unwrap();
    assert!(
        !block_on(engine.list_volumes())
            .unwrap()
            .iter()
            .any(|v| v.name == name)
    );
}

struct NetworkCleanup<'a> {
    runtime: &'a Runtime,
    docker: &'a Docker,
    name: String,
}

impl Drop for NetworkCleanup<'_> {
    fn drop(&mut self) {
        // Usually already removed by the test; a 404 here is expected.
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                self.docker.remove_network(&self.name),
            )
            .await
        });
    }
}

#[test]
#[ignore = "requires a real engine and a uniquely named throwaway network"]
fn live_network_list_and_remove_on_a_non_tokio_executor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let kind = block_on(engine.info()).unwrap().kind;
    let (runtime, docker) = test_client(&endpoint).unwrap();
    let name = test_container_name()
        .unwrap()
        .replace("berth-live-test-", "berth-live-network-");
    let label = "dev.berth.integration-test";
    let _cleanup = NetworkCleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    runtime
        .block_on(docker.create_network(NetworkCreateRequest {
            name: name.clone(),
            internal: Some(true),
            labels: Some(std::collections::HashMap::from([(
                label.into(),
                name.clone(),
            )])),
            ..Default::default()
        }))
        .unwrap();
    let networks = block_on(engine.list_networks()).unwrap();
    let network = networks
        .iter()
        .find(|n| n.name == name)
        .ok_or("throwaway network not listed")
        .unwrap();
    assert!(!network.is_builtin(kind) && network.internal);
    assert!(network.is_prunable(kind));
    assert_eq!(network.endpoints, []);
    assert!(network.labels.contains(&(label.into(), name.clone())));
    let bridge = networks
        .iter()
        .find(|n| n.name == "bridge")
        .ok_or("default bridge not listed")
        .unwrap();
    assert!(bridge.is_builtin(kind) && !bridge.subnets.is_empty());
    assert!(block_on(engine.remove_network(&bridge.id)).is_err());

    block_on(engine.remove_network(&network.id)).unwrap();
    assert!(
        !block_on(engine.list_networks())
            .unwrap()
            .iter()
            .any(|n| n.name == name)
    );
}

#[path = "live/files.rs"]
mod files;
#[path = "live/m3.rs"]
mod m3;
#[path = "live/tls.rs"]
mod tls;

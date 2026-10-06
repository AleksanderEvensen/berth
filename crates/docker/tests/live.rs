//! Opt-in tests against a real Docker-compatible daemon.
//! Run: cargo test -p berth-docker --test live -- --ignored
#![cfg(feature = "live")]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use berth_docker::{
    BollardEngine, ContainerAction, ContainerState, Endpoint, Engine, EngineError, EngineEvent,
    EventAction, ObjectKind, RemoveOptions, TlsConfig,
};
use bollard::{
    API_DEFAULT_VERSION, Docker, models::ContainerCreateBody, query_parameters as query,
};
use futures::{FutureExt as _, StreamExt as _, channel::oneshot, executor::block_on};
use tokio::runtime::Runtime;

fn endpoint() -> Endpoint {
    let host =
        std::env::var("DOCKER_HOST").unwrap_or_else(|_| "unix:///var/run/docker.sock".into());
    if let Some(path) = host.strip_prefix("unix://") {
        return Endpoint::Unix(path.into());
    }
    if let Some(pipe) = host.strip_prefix("npipe://") {
        return Endpoint::NamedPipe(pipe.into());
    }
    let verify = std::env::var("DOCKER_TLS_VERIFY").is_ok_and(|v| !v.is_empty());
    let tls = (verify || host.starts_with("https://")).then(|| TlsConfig {
        cert_dir: std::env::var_os("DOCKER_CERT_PATH")
            .map(Into::into)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(
                    std::env::var_os("HOME").expect("HOME or DOCKER_CERT_PATH is needed for TLS"),
                )
                .join(".docker")
            }),
        verify: true,
    });
    Endpoint::Tcp { url: host, tls }
}

fn direct_client(endpoint: &Endpoint) -> Docker {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(path) => {
            Docker::connect_with_unix(path.to_str().unwrap(), 30, API_DEFAULT_VERSION).unwrap()
        }
        #[cfg(windows)]
        Endpoint::NamedPipe(pipe) => {
            Docker::connect_with_named_pipe(pipe, 30, API_DEFAULT_VERSION).unwrap()
        }
        Endpoint::Tcp { url, tls: None } => {
            Docker::connect_with_http(url, 30, API_DEFAULT_VERSION).unwrap()
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
        )
        .unwrap(),
        #[allow(unreachable_patterns)]
        _ => panic!("endpoint unsupported on this test platform"),
    }
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
            Ok(Ok(()))
            | Ok(Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            })) => {}
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
            })
        });
        Self {
            receiver,
            cancel: Some(cancel_tx),
            thread: Some(thread),
        }
    }
    fn expect(&self, id: &str, action: EventAction) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let event = self
                .receiver
                .recv_timeout(remaining)
                .expect("expected engine event before timeout")
                .expect("event transport error");
            if event.kind == ObjectKind::Container && event.id == id && event.action == action {
                return;
            }
        }
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        let _ = self.cancel.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
#[ignore = "requires a real engine and permission to create an alpine:latest container"]
fn live_container_lifecycle_and_events_on_a_non_tokio_executor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let endpoint = endpoint();
    let engine =
        BollardEngine::connect(&endpoint).expect("connect to DOCKER_HOST or /var/run/docker.sock");
    let info = block_on(engine.info()).unwrap();
    assert!(!info.version.is_empty());
    assert!(!info.api_version.is_empty());
    assert_eq!(info.endpoint, endpoint);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let docker = runtime
        .block_on(direct_client(&endpoint).negotiate_version())
        .unwrap();
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
                result.expect("pull alpine:latest");
            }
        }
    });
    let events = Events::subscribe(&engine);
    let name = format!(
        "berth-live-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let created = runtime
        .block_on(docker.create_container(
            Some(query::CreateContainerOptions {
                name: Some(name.clone()),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some("alpine:latest".into()),
                cmd: Some(vec!["sleep".into(), "300".into()]),
                env: Some(vec!["BERTH_TEST=live".into()]),
                ..Default::default()
            },
        ))
        .unwrap();
    let id = created.id;
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
    events.expect(&id, EventAction::Start);
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
    events.expect(&id, EventAction::Pause);
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Paused
    );
    block_on(engine.container_action(&id, ContainerAction::Unpause)).unwrap();
    events.expect(&id, EventAction::Unpause);
    block_on(engine.container_action(
        &id,
        ContainerAction::Restart {
            timeout_secs: Some(1),
        },
    ))
    .unwrap();
    events.expect(&id, EventAction::Restart);
    block_on(engine.container_action(&id, ContainerAction::Kill)).unwrap();
    events.expect(&id, EventAction::Die);
    assert_eq!(
        block_on(engine.inspect_container(&id)).unwrap().state,
        ContainerState::Exited
    );
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    events.expect(&id, EventAction::Start);
    block_on(engine.container_action(
        &id,
        ContainerAction::Stop {
            timeout_secs: Some(1),
        },
    ))
    .unwrap();
    events.expect(&id, EventAction::Stop);
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
    events.expect(&id, EventAction::Destroy);
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

use super::*;
use futures::executor::block_on;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Small HTTP daemon substitute: verifies the public trait without Docker or
/// a Tokio context on the test thread, including actions and stream EOF.
pub(super) struct TestServer {
    pub(super) endpoint: Endpoint,
    pub(super) requests: Arc<Mutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
trait ReadWrite: std::io::Read + std::io::Write {}
impl<T: std::io::Read + std::io::Write> ReadWrite for T {}

impl TestServer {
    pub(super) fn new() -> Self {
        Self::with_tls(None)
    }

    fn with_tls(tls: Option<Arc<rustls::ServerConfig>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::Tcp {
            url: format!("http://{}", listener.local_addr().unwrap()),
            tls: None,
        };
        listener.set_nonblocking(true).unwrap();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let (stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("test server accept: {e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut stream: Box<dyn ReadWrite> = match &tls {
                    Some(config) => Box::new(rustls::StreamOwned::new(
                        rustls::ServerConnection::new(config.clone()).unwrap(),
                        stream,
                    )),
                    None => Box::new(stream),
                };
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(size) => request.extend_from_slice(&buffer[..size]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let line = request.lines().next().unwrap_or_default().to_owned();
                captured.lock().unwrap().push(line.clone());
                let (status, body) = response_for(&line);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            endpoint,
            requests,
            stopping,
            thread: Some(thread),
        }
    }
}
fn response_for(line: &str) -> (&'static str, &'static str) {
    if line.contains("/version ") {
        (
            "200 OK",
            r#"{"Version":"test","ApiVersion":"1.53","Platform":{"Name":"Docker Engine"}}"#,
        )
    } else if line.contains("/_ping ") {
        ("200 OK", "OK")
    } else if line.contains("/info ") {
        ("200 OK", r#"{"SecurityOptions":["name=rootless"]}"#)
    } else if line.contains("/containers/json?") {
        ("200 OK", "[]")
    } else if line.contains("/containers/test/json ") {
        (
            "200 OK",
            r#"{"Id":"test","Name":"/test","Config":{"Env":["A=B"]},"State":{"Status":"created"}}"#,
        )
    } else if line.contains("/images/json?") {
        ("200 OK", "[]")
    } else if line.contains("/volumes ") {
        ("200 OK", r#"{"Volumes":[]}"#)
    } else if line.contains("/system/df ") {
        ("200 OK", r#"{"Images":[],"Volumes":[],"BuildCache":[]}"#)
    } else if line.contains("/images/in-use?") || line.contains("/volumes/in-use?") {
        (
            "409 Conflict",
            r#"{"message":"resource is referenced by a stopped container"}"#,
        )
    } else if line.contains("/images/prune?") {
        (
            "200 OK",
            r#"{"ImagesDeleted":[{"Deleted":"sha256:test"}],"SpaceReclaimed":123}"#,
        )
    } else if line.contains("/volumes/prune?") {
        (
            "200 OK",
            r#"{"VolumesDeleted":["test-volume"],"SpaceReclaimed":456}"#,
        )
    } else if line.contains("/containers/prune") {
        (
            "200 OK",
            r#"{"ContainersDeleted":["test-container"],"SpaceReclaimed":789}"#,
        )
    } else if line.contains("/networks/prune") {
        ("200 OK", r#"{"NetworksDeleted":["test-network"]}"#)
    } else if line.contains("/build/prune?") {
        (
            "200 OK",
            r#"{"CachesDeleted":["test-cache"],"SpaceReclaimed":987}"#,
        )
    } else if line.contains("/images/test?") {
        ("200 OK", "[]")
    } else if line.contains("/events?") {
        (
            "200 OK",
            "{\"Type\":\"container\",\"Action\":\"start\",\"Actor\":{\"ID\":\"test\"},\"timeNano\":123}\n",
        )
    } else if line.contains("/start ") {
        ("304 Not Modified", "")
    } else {
        ("204 No Content", "")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn trait_methods_work_on_futures_executor_and_events_end_on_disconnect() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let engine: &dyn Engine = &engine;
    let info = block_on(engine.info()).unwrap();
    assert_eq!(info.kind, crate::EngineKind::Docker);
    assert!(info.rootless);
    assert_eq!(info.endpoint, server.endpoint);
    assert_eq!(block_on(engine.list_containers()).unwrap(), []);
    let inspected = block_on(engine.inspect_container("test")).unwrap();
    assert_eq!(inspected.name, "test");
    assert_eq!(inspected.env, Some(vec!["A=B".into()]));
    for action in [
        ContainerAction::Start,
        ContainerAction::Stop {
            timeout_secs: Some(3),
        },
        ContainerAction::Restart {
            timeout_secs: Some(4),
        },
        ContainerAction::Stop { timeout_secs: None },
        ContainerAction::Restart { timeout_secs: None },
        ContainerAction::Kill,
        ContainerAction::Pause,
        ContainerAction::Unpause,
    ] {
        block_on(engine.container_action("test", action)).unwrap();
    }
    block_on(engine.remove_container(
        "test",
        RemoveOptions {
            force: true,
            volumes: true,
        },
    ))
    .unwrap();
    let mut events = engine.events();
    let event = block_on(events.next()).unwrap().unwrap();
    assert_eq!(event.id, "test");
    assert_eq!(event.action, crate::EventAction::Start);
    assert!(block_on(events.next()).is_none());
    let requests = server.requests.lock().unwrap();
    for expected in [
        "all=true",
        "/stop?t=3",
        "/restart?t=4",
        "/kill",
        "/pause",
        "/unpause",
        "force=true",
        "v=true",
        "container",
        "image",
        "volume",
        "network",
    ] {
        assert!(
            requests.iter().any(|s| s.contains(expected)),
            "missing {expected}: {requests:?}"
        );
    }
    assert!(
        requests
            .iter()
            .any(|s| s.contains("/stop?") && !s.contains("t="))
    );
    assert!(
        requests
            .iter()
            .any(|s| s.contains("/restart?") && !s.contains("t="))
    );
    drop(requests);
}

#[test]
fn runtime_work_has_own_tokio_context_and_named_threads() {
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let name = block_on(engine.spawn(async {
        assert!(tokio::runtime::Handle::try_current().is_ok());
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok(std::thread::current().name().unwrap().to_owned())
    }))
    .unwrap();
    assert_eq!(name, "berth-engine");
}

struct Dropped(std::sync::mpsc::Sender<()>);
impl Drop for Dropped {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn dropping_an_unpolled_future_cancels_its_task() {
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let probe = Dropped(sender);
    let future = engine.spawn(async move {
        let _probe = probe;
        futures::future::pending::<EngineResult<()>>().await
    });
    drop(future);
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn dropping_a_pending_future_cancels_its_task() {
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let (started_tx, started_rx) = oneshot::channel();
    let probe = Dropped(sender);
    let future = engine.spawn(async move {
        let _probe = probe;
        let _ = started_tx.send(());
        futures::future::pending::<EngineResult<()>>().await
    });
    block_on(started_rx).unwrap();
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(futures::task::noop_waker_ref());
    assert!(future.as_mut().poll(&mut context).is_pending());
    drop(future);
    receiver.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn dropping_stream_cancels_an_idle_subscription_task() {
    let runtime = EngineRuntime::new().unwrap();
    let (sender, receiver) = mpsc::channel(1);
    let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
    let probe = Dropped(dropped_tx);
    let task = runtime.get().spawn(async move {
        let _probe = probe;
        let _sender = sender;
        futures::future::pending::<()>().await;
    });
    let stream = EventStream {
        receiver,
        _task: AbortOnDrop(task.abort_handle()),
    };
    drop(stream);
    dropped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn engine_drop_cancels_outstanding_work_without_blocking_inside_tokio() {
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let future = engine.spawn(futures::future::pending::<EngineResult<()>>());
    let events = engine.events();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        drop(engine);
    });
    assert!(matches!(block_on(future), Err(EngineError::Transport(_))));
    drop(events);
}

#[test]
fn start_304_is_success_and_other_errors_are_preserved() {
    let error = || BollardError::DockerResponseServerError {
        status_code: 304,
        message: "already started".into(),
    };
    assert_eq!(action_result(ContainerAction::Start, Err(error())), Ok(()));
    assert_eq!(
        action_result(ContainerAction::Pause, Err(error())),
        Err(EngineError::Engine {
            status: 304,
            message: "already started".into()
        })
    );
    assert_eq!(timeout(None), Ok(None));
    assert_eq!(timeout(Some(0)), Ok(Some(0)));
    assert_eq!(timeout(Some(3)), Ok(Some(3)));
    assert!(timeout(Some(u32::MAX)).is_err());
}

#[test]
fn connect_errors_classify_socket_failures() {
    let endpoint = Endpoint::Unix("/socket".into());
    for (kind, expected) in [
        (
            std::io::ErrorKind::PermissionDenied,
            ConnectError::PermissionDenied {
                endpoint: "unix:///socket".into(),
            },
        ),
        (
            std::io::ErrorKind::ConnectionRefused,
            ConnectError::Refused {
                endpoint: "unix:///socket".into(),
            },
        ),
        (
            std::io::ErrorKind::NotFound,
            ConnectError::NotFound {
                tried: vec!["unix:///socket".into()],
            },
        ),
    ] {
        let error: BollardError = std::io::Error::from(kind).into();
        assert_eq!(connect_error(&error, &endpoint), expected);
    }
    assert_eq!(
        connect_error(
            &BollardError::SocketNotFoundError("/socket".into()),
            &endpoint
        ),
        ConnectError::NotFound {
            tried: vec!["unix:///socket".into()]
        }
    );
    #[cfg(unix)]
    assert!(matches!(
        BollardEngine::connect(&Endpoint::Unix(
            format!("/berth-no-such-socket-{}", std::process::id()).into()
        )),
        Err(ConnectError::NotFound { .. })
    ));
}

#[test]
fn connect_refused_tcp_is_distinguishable_through_hyper_sources() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Endpoint::Tcp {
        url: format!("http://{}", listener.local_addr().unwrap()),
        tls: None,
    };
    drop(listener);
    assert!(matches!(
        BollardEngine::connect(&endpoint),
        Err(ConnectError::Refused { .. })
    ));
}

#[test]
fn invalid_transport_and_tls_configuration_return_errors() {
    for endpoint in [
        Endpoint::Tcp {
            url: "https://localhost:2376".into(),
            tls: None,
        },
        Endpoint::Tcp {
            url: "ftp://localhost:2375".into(),
            tls: None,
        },
        Endpoint::Tcp {
            url: "tcp://localhost:2376".into(),
            tls: Some(crate::TlsConfig {
                cert_dir: "/berth-missing-certs".into(),
                verify: true,
            }),
        },
        Endpoint::Tcp {
            url: "ftp://localhost:2376".into(),
            tls: Some(crate::TlsConfig {
                cert_dir: "/berth-missing-certs".into(),
                verify: false,
            }),
        },
    ] {
        assert!(matches!(
            BollardEngine::connect(&endpoint),
            Err(ConnectError::InvalidConfig(_))
        ));
    }
    #[cfg(not(windows))]
    assert!(matches!(
        BollardEngine::connect(&Endpoint::NamedPipe("//./pipe/docker_engine".into())),
        Err(ConnectError::InvalidConfig(_))
    ));
}

#[test]
fn tls_authenticates_client_and_honors_server_verification_in_tls12_and_tls13() {
    use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject as _};
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/live/test-certs");
    let certs = |name: &str| {
        CertificateDer::pem_file_iter(directory.join(name))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    for protocol in [&rustls::version::TLS12, &rustls::version::TLS13] {
        let mut roots = rustls::RootCertStore::empty();
        for cert in certs("ca.pem") {
            roots.add(cert).unwrap();
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            provider.clone(),
        )
        .build()
        .unwrap();
        let key = PrivateKeyDer::from_pem_file(directory.join("server-key.pem")).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[protocol])
            .unwrap()
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs("server.pem"), key)
            .unwrap();
        let server = TestServer::with_tls(Some(Arc::new(config)));
        let Endpoint::Tcp { url, .. } = &server.endpoint else {
            panic!("TLS test server must have a TCP endpoint")
        };
        let verified_endpoint = Endpoint::Tcp {
            url: url.replace("http://127.0.0.1", "https://localhost"),
            tls: Some(crate::TlsConfig {
                cert_dir: directory.clone(),
                verify: true,
            }),
        };
        let engine = BollardEngine::connect(&verified_endpoint)
            .expect("trusted CA, matching hostname and client certificate");
        assert_eq!(block_on(engine.info()).unwrap().version, "test");
        let wrong_name = Endpoint::Tcp {
            url: url.replace("http://", "tcp://"),
            tls: Some(crate::TlsConfig {
                cert_dir: directory.clone(),
                verify: true,
            }),
        };
        assert!(
            BollardEngine::connect(&wrong_name).is_err(),
            "server certificate excludes the IP address"
        );
        let unverified = Endpoint::Tcp {
            url: url.replace("http://", "tcp://"),
            tls: Some(crate::TlsConfig {
                cert_dir: directory.clone(),
                verify: false,
            }),
        };
        let engine = BollardEngine::connect(&unverified)
            .expect("verify=false permits a mismatched server name");
        assert_eq!(block_on(engine.info()).unwrap().version, "test");
    }
}

#[test]
fn storage_methods_bridge_runtime_and_send_the_required_filters() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let server = TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let engine: &dyn Engine = &engine;
    assert_eq!(block_on(engine.list_images()).unwrap(), Vec::<Image>::new());
    assert_eq!(
        block_on(engine.list_volumes()).unwrap(),
        Vec::<Volume>::new()
    );
    assert_eq!(
        block_on(engine.disk_usage()).unwrap().volumes,
        Vec::<Volume>::new()
    );
    for force in [false, true] {
        block_on(engine.remove_image("test", force)).unwrap();
        block_on(engine.remove_volume("test", force)).unwrap();
    }
    let expected = EngineError::Conflict("resource is referenced by a stopped container".into());
    assert_eq!(
        block_on(engine.remove_image("in-use", false)),
        Err(expected.clone())
    );
    assert_eq!(
        block_on(engine.remove_volume("in-use", false)),
        Err(expected)
    );
    assert_eq!(
        block_on(engine.prune_containers()).unwrap().reclaimed_bytes,
        789
    );
    for mode in [ImagePrune::Dangling, ImagePrune::AllUnused] {
        assert_eq!(
            block_on(engine.prune_images(mode)).unwrap().deleted,
            ["sha256:test"]
        );
    }
    assert_eq!(
        block_on(engine.prune_volumes()).unwrap().reclaimed_bytes,
        456
    );
    assert_eq!(
        block_on(engine.prune_networks()).unwrap().deleted,
        ["test-network"]
    );
    assert_eq!(
        block_on(engine.prune_build_cache())
            .unwrap()
            .reclaimed_bytes,
        987
    );
    let requests = server.requests.lock().unwrap();
    for expected in [
        "/images/json?all=true",
        "/volumes HTTP",
        "/system/df HTTP",
        "/images/test?force=false",
        "/images/test?force=true",
        "/volumes/test?force=false",
        "/volumes/test?force=true",
        // The JSON filter values are URL-encoded on the wire.
        "/images/prune?filters=%7B%22dangling%22%3A%5B%22true%22%5D%7D",
        "/images/prune?filters=%7B%22dangling%22%3A%5B%22false%22%5D%7D",
        "/volumes/prune?filters=%7B%22all%22%3A%5B%22true%22%5D%7D",
        "/build/prune?all=true",
    ] {
        assert!(
            requests.iter().any(|s| s.contains(expected)),
            "missing {expected}: {requests:?}"
        );
    }
    drop(requests);
}

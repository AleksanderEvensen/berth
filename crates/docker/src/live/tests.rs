// The mock daemon must outlive the engine under test; dropping it early is
// exactly what these tests must not do.
#![allow(clippy::significant_drop_tightening)]
use super::*;
use futures::executor::block_on;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Daemon substitute on mockito: verifies the public trait without Docker or
/// a Tokio context on the test thread, including actions and stream EOF.
/// Every request is recorded as an HTTP request line for assertions.
pub(super) struct TestServer {
    pub(super) endpoint: Endpoint,
    pub(super) requests: Arc<Mutex<Vec<String>>>,
    _server: mockito::ServerGuard,
    _mocks: Vec<mockito::Mock>,
}

impl TestServer {
    pub(super) fn new() -> Self {
        let mut server = mockito::Server::new();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let line = |request: &mockito::Request| {
            format!("{} {} HTTP/1.1", request.method(), request.path_and_query())
        };
        let mocks = ["GET", "POST", "DELETE", "HEAD", "PUT"]
            .into_iter()
            .map(|method| {
                let captured = captured.clone();
                server
                    .mock(method, mockito::Matcher::Any)
                    .with_header("content-type", "application/json")
                    .with_status_code_from_request(move |request| {
                        let line = line(request);
                        let (status, _) = response_for(&line);
                        captured.lock().unwrap().push(line);
                        status
                            .split(' ')
                            .next()
                            .and_then(|code| code.parse().ok())
                            .unwrap_or(500)
                    })
                    .with_body_from_request(move |request| response_for(&line(request)).1.into())
                    .create()
            })
            .collect();
        Self {
            endpoint: Endpoint::Tcp {
                url: server.url(),
                tls: None,
            },
            requests,
            _server: server,
            _mocks: mocks,
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
    } else if line.contains("/containers/tty/json ") {
        ("200 OK", r#"{"Id":"tty","Config":{"Tty":true}}"#)
    } else if line.contains("/containers/tty/logs?") {
        ("200 OK", "first\npartial")
    } else if line.contains("/containers/test/logs?") {
        // Multiplexed records; the last is cut short.
        (
            "200 OK",
            "\x01\0\0\0\0\0\0\x06first\n\x02\0\0\0\0\0\0\x04err\n\x01\0\0\0\0\0\0\x09cut",
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
    let (started_tx, started_rx) = futures::channel::oneshot::channel();
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
fn tls_rejects_skip_verify_and_classifies_a_closed_port() {
    let certs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/live/test-certs");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let at = |verify| Endpoint::Tcp {
        url: format!("tcp://localhost:{port}"),
        tls: Some(crate::TlsConfig {
            cert_dir: certs.clone(),
            verify,
        }),
    };
    assert!(matches!(
        BollardEngine::connect(&at(false)),
        Err(ConnectError::InvalidConfig(_))
    ));
    assert!(matches!(
        BollardEngine::connect(&at(true)),
        Err(ConnectError::Refused { .. })
    ));
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

/// A stand-in `ssh`: logs its arguments, answers `-G`, and otherwise runs
/// `script` in bash.
#[cfg(unix)]
fn fake_ssh(dir: &std::path::Path, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join("ssh");
    let log = dir.join("args");
    std::fs::write(
        &path,
        format!(
            "#!/bin/bash\n\
             echo \"$@\" >> '{}'\n\
             for arg; do [ \"$arg\" = -G ] && {{ echo 'user me'; echo 'hostname prod.example.com'; exit 0; }}; done\n\
             {script}\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn ssh_endpoint() -> Endpoint {
    Endpoint::Ssh {
        destination: "prod".into(),
        port: None,
        hostname: None,
    }
}

#[cfg(unix)]
#[test]
fn ssh_carries_the_api_over_the_ssh_process() {
    let server = TestServer::new();
    let Endpoint::Tcp { url, .. } = &server.endpoint else {
        panic!("test server is TCP");
    };
    let port = url.rsplit(':').next().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    // What dial-stdio does, with bash's /dev/tcp standing in for the socket.
    let ssh = fake_ssh(
        dir.path(),
        &format!("exec 3<>/dev/tcp/127.0.0.1/{port}\ncat <&3 &\nexec cat >&3"),
    );
    let engine = BollardEngine::connect_with(&ssh_endpoint(), Some(&ssh)).unwrap();
    let info = block_on(engine.info()).unwrap();
    assert_eq!(
        info.endpoint,
        Endpoint::Ssh {
            destination: "prod".into(),
            port: None,
            hostname: Some("prod.example.com".into()),
        }
    );
    assert_eq!(info.endpoint.remote_host(), Some("prod.example.com"));
    assert_eq!(block_on(engine.list_containers()).unwrap(), []);
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert!(args.contains("-G -- prod"), "{args}");
    assert!(
        args.contains("-o BatchMode=yes") && args.contains("-- prod docker system dial-stdio"),
        "{args}"
    );
}

#[cfg(unix)]
#[test]
fn ssh_failures_report_what_ssh_said() {
    let dir = tempfile::TempDir::new().unwrap();
    let ssh = fake_ssh(
        dir.path(),
        "echo 'me@prod: Permission denied (publickey).' >&2\nexit 255",
    );
    let Err(ConnectError::Ssh(message)) = BollardEngine::connect_with(&ssh_endpoint(), Some(&ssh))
    else {
        panic!("expected an SSH failure");
    };
    assert!(message.contains("ssh-agent"), "{message}");

    // ssh that exits at once, before the request is even written.
    let ssh = fake_ssh(
        dir.path(),
        "echo 'bash: docker: command not found' >&2\nexit 127",
    );
    let Err(ConnectError::Ssh(message)) = BollardEngine::connect_with(&ssh_endpoint(), Some(&ssh))
    else {
        panic!("expected an SSH failure");
    };
    assert!(message.contains("docker CLI"), "{message}");

    assert_eq!(
        BollardEngine::connect_with(&ssh_endpoint(), None).err(),
        Some(ConnectError::Ssh(crate::SSH_CLIENT_MISSING.into()))
    );
}

#[test]
fn ssh_failures_after_connect_keep_their_message() {
    let error: BollardError = std::io::Error::other(ssh::SshFailure("ssh prod: why".into())).into();
    assert_eq!(
        mapping::error(error),
        EngineError::Transport("ssh prod: why".into())
    );
}

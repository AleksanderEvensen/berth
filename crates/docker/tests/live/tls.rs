//! TLS against a real daemon: a throwaway `docker:dind` container generates
//! its own CA, server and client certificates (`DOCKER_TLS_CERTDIR`) and
//! requires client authentication, like a `tcp://…:2376` production setup.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::path::Path;

use super::*;
use berth_docker::ConnectError;
use bollard::models::{HostConfig, PortBinding};

const IMAGE: &str = "docker:dind";
/// Bundled credentials from an unrelated CA (see src/live/test-certs).
const FOREIGN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/live/test-certs");

fn pull(runtime: &Runtime, docker: &Docker) {
    runtime.block_on(async {
        if docker.inspect_image(IMAGE).await.is_ok() {
            return;
        }
        let mut pull = docker.create_image(
            Some(query::CreateImageOptions {
                from_image: Some("docker".into()),
                tag: Some("dind".into()),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(result) = pull.next().await {
            result.unwrap();
        }
    });
}

/// Privileged dind with its TLS port published on an ephemeral loopback port.
#[allow(clippy::zero_sized_map_values, reason = "bollard's exposed-ports type")]
fn start_dind(runtime: &Runtime, docker: &Docker, name: &str) -> u16 {
    let port = "2376/tcp".to_string();
    let body = ContainerCreateBody {
        image: Some(IMAGE.into()),
        env: Some(vec!["DOCKER_TLS_CERTDIR=/certs".into()]),
        exposed_ports: Some(HashMap::from([(port.clone(), HashMap::new())])),
        host_config: Some(HostConfig {
            privileged: Some(true),
            port_bindings: Some(HashMap::from([(
                port.clone(),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".into()),
                    host_port: Some(String::new()),
                }]),
            )])),
            ..Default::default()
        }),
        ..Default::default()
    };
    runtime.block_on(async {
        docker
            .create_container(
                Some(query::CreateContainerOptions {
                    name: Some(name.into()),
                    ..Default::default()
                }),
                body,
            )
            .await
            .unwrap();
        docker
            .start_container(name, None::<query::StartContainerOptions>)
            .await
            .unwrap();
        let inspect = docker
            .inspect_container(name, None::<query::InspectContainerOptions>)
            .await
            .unwrap();
        inspect
            .network_settings
            .and_then(|n| n.ports)
            .and_then(|ports| ports.get(&port).cloned().flatten())
            .and_then(|bindings| bindings.into_iter().next())
            .and_then(|binding| binding.host_port)
            .and_then(|p| p.parse().ok())
            .expect("dind publishes 2376")
    })
}

/// A file from the container, or `None` until it exists.
fn read_file(runtime: &Runtime, docker: &Docker, name: &str, path: &str) -> Option<Vec<u8>> {
    runtime.block_on(async {
        let exec = docker
            .create_exec(
                name,
                bollard::exec::CreateExecOptions {
                    cmd: Some(vec!["cat", path]),
                    attach_stdout: Some(true),
                    ..Default::default()
                },
            )
            .await
            .ok()?;
        let bollard::exec::StartExecResults::Attached { mut output, .. } =
            docker.start_exec(&exec.id, None).await.ok()?
        else {
            return None;
        };
        let mut bytes = Vec::new();
        while let Some(Ok(frame)) = output.next().await {
            if let bollard::container::LogOutput::StdOut { message } = frame {
                bytes.extend_from_slice(&message);
            }
        }
        let code = docker.inspect_exec(&exec.id).await.ok()?.exit_code;
        (code == Some(0) && !bytes.is_empty()).then_some(bytes)
    })
}

/// Wait for dind to generate its client credentials, then copy them out.
fn client_certs(runtime: &Runtime, docker: &Docker, name: &str, dir: &Path) {
    let deadline = deadline();
    for file in ["ca.pem", "cert.pem", "key.pem"] {
        let bytes = loop {
            if let Some(bytes) = read_file(runtime, docker, name, &format!("/certs/client/{file}"))
            {
                break bytes;
            }
            assert!(std::time::Instant::now() < deadline, "dind wrote {file}");
            std::thread::sleep(Duration::from_millis(500));
        };
        std::fs::write(dir.join(file), bytes).unwrap();
    }
}

fn endpoint_at(port: u16, cert_dir: &Path, verify: bool) -> Endpoint {
    Endpoint::Tcp {
        url: format!("tcp://localhost:{port}"),
        tls: Some(TlsConfig {
            cert_dir: cert_dir.into(),
            verify,
        }),
    }
}

fn deadline() -> std::time::Instant {
    std::time::Instant::now()
        .checked_add(Duration::from_secs(90))
        .unwrap()
}

/// The daemon starts listening after writing its certificates; retry until
/// it accepts a verified connection.
fn connect_when_ready(endpoint: &Endpoint) -> BollardEngine {
    let deadline = deadline();
    loop {
        match BollardEngine::connect(endpoint) {
            Ok(engine) => return engine,
            Err(err) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "dind TLS daemon never accepted: {err}"
                );
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// A directory mixing `cert.pem`/`key.pem` from `credentials` with `ca.pem`
/// from `ca`.
fn mixed(credentials: &Path, ca: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for file in ["cert.pem", "key.pem"] {
        std::fs::copy(credentials.join(file), dir.path().join(file)).unwrap();
    }
    std::fs::copy(ca.join("ca.pem"), dir.path().join("ca.pem")).unwrap();
    dir
}

#[test]
#[ignore = "requires a real engine that may run a privileged throwaway docker:dind container"]
fn mutual_tls_against_a_real_daemon() {
    let runtime = Runtime::new().unwrap();
    let docker = direct_client(&endpoint().unwrap()).unwrap();
    pull(&runtime, &docker);
    let name = format!(
        "berth-test-tls-{}",
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
    let port = start_dind(&runtime, &docker, &name);
    let certs = tempfile::tempdir().unwrap();
    client_certs(&runtime, &docker, &name, certs.path());

    let engine = connect_when_ready(&endpoint_at(port, certs.path(), true));
    let info = block_on(engine.info()).unwrap();
    assert_ne!(info.version, "");
    assert_eq!(block_on(engine.list_containers()).unwrap().len(), 0);

    let foreign = Path::new(FOREIGN);
    let wrong_ca = mixed(certs.path(), foreign);
    assert!(
        BollardEngine::connect(&endpoint_at(port, wrong_ca.path(), true)).is_err(),
        "the daemon's certificate isn't signed by the trusted CA"
    );
    let wrong_client = mixed(foreign, certs.path());
    assert!(
        BollardEngine::connect(&endpoint_at(port, wrong_client.path(), true)).is_err(),
        "the daemon rejects a client certificate from another CA"
    );
    assert!(matches!(
        BollardEngine::connect(&endpoint_at(port, certs.path(), false)),
        Err(ConnectError::InvalidConfig(_))
    ));
}

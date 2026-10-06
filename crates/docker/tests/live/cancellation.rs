//! A transparent TCP -> Unix proxy observes the real Docker log request being
//! closed. This proves cancellation at the socket, with no public diagnostics.
use super::*;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub(super) struct Proxy {
    pub endpoint: Endpoint,
    pub closed: oneshot::Receiver<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Proxy {
    pub fn new(path: PathBuf) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::Tcp {
            url: format!("http://{}", listener.local_addr().unwrap()),
            tls: None,
        };
        listener.set_nonblocking(true).unwrap();
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        let (closed_tx, closed) = oneshot::channel();
        let closed_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(closed_tx)));
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            while let Ok((client, _)) = listener.accept().await {
                while tasks.try_join_next().is_some() {}
                tasks.spawn(forward(client, path.clone(), closed_tx.clone()));
            }
        });
        Self {
            endpoint,
            closed,
            task,
        }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn forward(
    client: tokio::net::TcpStream,
    path: PathBuf,
    closed: std::sync::Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
) {
    let Ok(upstream) = tokio::net::UnixStream::connect(path).await else {
        return;
    };
    let (mut client_read, mut client_write) = client.into_split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();
    let response = async {
        tokio::io::copy(&mut upstream_read, &mut client_write)
            .await
            .ok();
    };
    let request = async {
        let mut buffer = [0; 8192];
        let mut headers = Vec::new();
        let mut logs = false;
        while let Ok(size) = client_read.read(&mut buffer).await {
            if size == 0 {
                break;
            }
            let Some(bytes) = buffer.get(..size) else {
                break;
            };
            if !logs {
                headers.extend_from_slice(bytes);
                logs = String::from_utf8_lossy(&headers).contains("/logs?");
                if headers.len() > 16_384 {
                    headers.clear();
                }
            }
            if upstream_write.write_all(bytes).await.is_err() {
                break;
            }
        }
        if logs && let Some(sender) = closed.lock().unwrap().take() {
            let _ = sender.send(());
        }
    };
    // Cancelling either direction drops both sockets and their copy futures.
    futures::pin_mut!(response, request);
    futures::future::select(response, request).await;
}

#[test]
#[ignore = "requires Unix Docker socket and creates only a throwaway alpine container"]
fn dropping_follow_logs_closes_the_docker_request() {
    let endpoint = endpoint().unwrap();
    let Endpoint::Unix(path) = &endpoint else {
        return;
    };
    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    let name = test_container_name().unwrap();
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let id = create(&runtime, &docker, &name, "echo ready; sleep 300");
    runtime
        .block_on(docker.start_container(&id, None::<query::StartContainerOptions>))
        .unwrap();
    // Proxy uses its own always-driven runtime while connect blocks on its
    // handshake. It forwards only requests to this test's container.
    let proxy_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let mut proxy = {
        let _entered = proxy_runtime.enter();
        Proxy::new(path.clone())
    };
    let engine = BollardEngine::connect(&proxy.endpoint).unwrap();
    let mut stream = engine.logs(&id, options(true, None));
    assert_eq!(
        timed(&runtime, stream.next()).unwrap().unwrap().line,
        "ready"
    );
    drop(stream);
    // The container and engine are still alive: EOF was caused by dropping
    // the subscription, not by process exit or runtime/engine shutdown.
    timed(&runtime, &mut proxy.closed).unwrap();
    assert_eq!(
        timed(&runtime, engine.inspect_container(&id))
            .unwrap()
            .state,
        ContainerState::Running
    );
}

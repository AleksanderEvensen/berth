#![allow(clippy::unwrap_used)]
use super::super::EngineRuntime;
use super::*;

struct Probe(std::sync::mpsc::Sender<()>);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
#[test]
fn dropping_stream_aborts_all_session_tasks_even_when_unpolled() {
    let runtime = EngineRuntime::new().unwrap();
    let (sender, receiver) = mpsc::channel::<EngineResult<Vec<u8>>>(1);
    let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
    let tasks = (0..3)
        .map(|_| {
            let probe = Probe(dropped_tx.clone());
            let sender = sender.clone();
            let task = runtime.get().spawn(async move {
                let _probe = probe;
                let _sender = sender;
                futures::future::pending::<()>().await;
            });
            AbortOnDrop(task.abort_handle())
        })
        .collect();
    drop(sender);
    drop(TaskStream {
        receiver,
        _tasks: tasks,
    });
    for _ in 0..3 {
        dropped_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
    }
}
#[test]
fn dropping_backpressured_stream_aborts_its_producer() {
    let runtime = EngineRuntime::new().unwrap();
    let (mut sender, receiver) = mpsc::channel(1);
    let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
    let probe = Probe(dropped_tx);
    let task = runtime.get().spawn(async move {
        let _probe = probe;
        loop {
            if sender.send(Ok(())).await.is_err() {
                break;
            }
        }
    });
    drop(TaskStream {
        receiver,
        _tasks: vec![AbortOnDrop(task.abort_handle())],
    });
    dropped_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
}

fn test_docker(runtime: &EngineRuntime) -> Docker {
    let _entered = runtime.get().enter();
    Docker::connect_with_http("http://127.0.0.1:9", 1, bollard::API_DEFAULT_VERSION).unwrap()
}
#[test]
fn attached_session_stdin_drop_shuts_down_write_half() {
    let runtime = EngineRuntime::new().unwrap();
    let (input, mut read) = tokio::io::duplex(64);
    let session = attach_session(
        runtime.get(),
        test_docker(&runtime),
        "unused".into(),
        TtySize { cols: 80, rows: 24 },
        Box::pin(input),
        futures::stream::pending().boxed(),
    );
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime.get().spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut bytes = Vec::new();
        read.read_to_end(&mut bytes).await.unwrap();
        tx.send(bytes).unwrap();
    });
    let mut stdin = session.stdin;
    futures::executor::block_on(stdin.send(b"hello".to_vec())).unwrap();
    drop(stdin);
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
        b"hello"
    );
    drop(session.stdout);
}
#[test]
fn remote_output_eof_cancels_pumps_with_handles_still_retained() {
    let runtime = EngineRuntime::new().unwrap();
    let (input, _read) = tokio::io::duplex(64);
    let output = futures::stream::iter([Ok(LogOutput::Console {
        message: b"bye".to_vec().into(),
    })])
    .boxed();
    let mut session = attach_session(
        runtime.get(),
        test_docker(&runtime),
        "unused".into(),
        TtySize { cols: 80, rows: 24 },
        Box::pin(input),
        output,
    );
    assert_eq!(
        futures::executor::block_on(session.stdout.next())
            .unwrap()
            .unwrap(),
        b"bye"
    );
    assert!(futures::executor::block_on(session.stdout.next()).is_none());
    assert!(session.stdin.is_closed());
    session.resizer.resize(TtySize {
        cols: 120,
        rows: 40,
    });
}

#[test]
fn resize_skips_duplicates_and_sends_the_latest_size() {
    let server = super::super::tests::TestServer::new();
    let engine = BollardEngine::connect(&server.endpoint).unwrap();
    let (input, _read) = tokio::io::duplex(64);
    let session = attach_session(
        engine.runtime(),
        engine.docker.clone(),
        "unused".into(),
        TtySize { cols: 80, rows: 24 },
        Box::pin(input),
        futures::stream::pending().boxed(),
    );
    for _ in 0..100 {
        session.resizer.resize(TtySize { cols: 80, rows: 24 });
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("/resize?"))
    );
    for _ in 0..100 {
        session.resizer.resize(TtySize {
            cols: 120,
            rows: 40,
        });
    }
    for _ in 0..100 {
        if server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("/resize?"))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    let requests = server.requests.lock().unwrap();
    let resized: Vec<_> = requests.iter().filter(|r| r.contains("/resize?")).collect();
    assert_eq!(resized.len(), 1);
    assert!(
        resized
            .iter()
            .all(|r| r.contains("h=40") && r.contains("w=120"))
    );
    drop(requests);
    drop(session.stdout);
}

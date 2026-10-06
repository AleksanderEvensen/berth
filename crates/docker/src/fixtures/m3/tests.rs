#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
use super::*;
use crate::{ContainerAction, Engine as _};
use futures::executor::block_on;

fn engine() -> (FixtureEngine, String) {
    let engine = FixtureEngine::default().with_log_history_lines(12);
    let id = block_on(engine.list_containers())
        .unwrap()
        .into_iter()
        .find(|c| c.state == ContainerState::Running)
        .unwrap()
        .id;
    (engine, id)
}
fn history(
    engine: &FixtureEngine,
    id: &str,
    tail: Option<u32>,
    since: Option<i64>,
) -> Vec<LogLine> {
    block_on(
        engine
            .logs(
                id,
                LogOptions {
                    follow: false,
                    tail,
                    since,
                },
            )
            .map(Result::unwrap)
            .collect(),
    )
}
#[test]
fn history_is_deterministic_and_honors_tail_since_and_zero() {
    let (engine, id) = engine();
    let all = history(&engine, &id, None, None);
    assert_eq!(all.len(), 12);
    assert_eq!(all, history(&engine, &id, None, None));
    assert_eq!(
        history(&engine, &id, Some(3), None),
        all.iter().skip(9).cloned().collect::<Vec<_>>()
    );
    let since = all.get(7).unwrap().timestamp.unwrap() / 1_000_000_000;
    assert_eq!(
        history(&engine, &id, None, Some(since)),
        all.iter().skip(7).cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        history(&engine, &id, Some(3), Some(since)),
        all.iter().skip(9).cloned().collect::<Vec<_>>()
    );
    assert_eq!(history(&engine, &id, Some(0), None), Vec::<LogLine>::new());
    assert!(all.iter().any(|l| l.stream == LogStream::Stderr));
    assert!(all.iter().any(|l| l.line.contains('\x1b')));
}
#[test]
fn follow_is_new_output_and_ends_when_container_stops() {
    let (engine, id) = engine();
    let mut logs = engine.logs(
        &id,
        LogOptions {
            follow: true,
            tail: Some(0),
            since: None,
        },
    );
    let first = block_on(logs.next()).unwrap().unwrap();
    let second = block_on(logs.next()).unwrap().unwrap();
    assert!(second.timestamp > first.timestamp);
    block_on(engine.container_action(&id, ContainerAction::Stop { timeout_secs: None })).unwrap();
    assert!(block_on(logs.next()).is_none());
    let stopped = block_on(engine.logs(&id, LogOptions::default()).collect::<Vec<_>>());
    assert_eq!(stopped.len(), 12);
    assert!(block_on(engine.stats(&id).next()).is_none());
    assert!(matches!(
        block_on(engine.exec(&id, ExecOptions::shell(TtySize { cols: 80, rows: 24 }))),
        Err(EngineError::Conflict(_))
    ));
}
#[test]
fn missing_container_yields_one_error() {
    let (engine, _) = engine();
    let errors = block_on(
        engine
            .logs("missing", LogOptions::default())
            .collect::<Vec<_>>(),
    );
    assert_eq!(errors, [Err(EngineError::NotFound("missing".into()))]);
    assert_eq!(
        block_on(engine.stats("missing").collect::<Vec<_>>()),
        [Err(EngineError::NotFound("missing".into()))]
    );
}
#[test]
fn stats_counters_are_monotonic_and_stream_stops_on_pause() {
    let (engine, id) = engine();
    let mut stats = engine.stats(&id);
    let a = block_on(stats.next()).unwrap().unwrap();
    let b = block_on(stats.next()).unwrap().unwrap();
    assert!(b.time_nanos > a.time_nanos);
    assert!(b.cpu_percent.is_finite() && b.cpu_percent >= 0.0);
    assert!(b.mem_used < b.mem_limit);
    assert!(b.net_rx > a.net_rx && b.net_tx > a.net_tx);
    assert!(b.blk_read > a.blk_read && b.blk_write > a.blk_write);
    block_on(engine.container_action(&id, ContainerAction::Pause)).unwrap();
    assert!(block_on(stats.next()).is_none());
}
#[test]
fn shell_echo_backspace_resize_commands_and_exit() {
    let (engine, id) = engine();
    let mut session =
        block_on(engine.exec(&id, ExecOptions::shell(TtySize { cols: 80, rows: 24 }))).unwrap();
    let prompt = block_on(session.stdout.next()).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&prompt).contains("root@"));
    session.resizer.resize(TtySize {
        cols: 120,
        rows: 40,
    });
    block_on(
        session
            .stdin
            .send(b"echo hix\x7f\r\nstty size\rtput cols\rpwd\rwhoami\rls\rfoo\r".to_vec()),
    )
    .unwrap();
    let output = block_on(session.stdout.next()).unwrap().unwrap();
    let output = String::from_utf8_lossy(&output);
    for text in [
        "echo hix\x08 \x08\r\nhi\r\n",
        "40 120\r\n",
        "120\r\n",
        "/\r\n",
        "root\r\n",
        "bin  dev",
        "sh: foo: not found\r\n",
    ] {
        assert!(output.contains(text), "{output:?}");
    }
    block_on(session.stdin.send(b"exit\r".to_vec())).unwrap();
    assert_eq!(
        block_on(session.stdout.next()).unwrap().unwrap(),
        b"exit\r\n"
    );
    assert!(block_on(session.stdout.next()).is_none());
}
#[test]
fn dropping_stdin_sends_eof_and_stdout_cancels_pending_input() {
    let (engine, id) = engine();
    let mut session =
        block_on(engine.exec(&id, ExecOptions::shell(TtySize { cols: 80, rows: 24 }))).unwrap();
    block_on(session.stdout.next()).unwrap().unwrap();
    drop(session.stdin);
    assert!(block_on(session.stdout.next()).is_none());
    let mut session =
        block_on(engine.exec(&id, ExecOptions::shell(TtySize { cols: 80, rows: 24 }))).unwrap();
    block_on(session.stdout.next()).unwrap().unwrap();
    drop(session.stdout);
    for _ in 0..100 {
        if session.stdin.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.stdin.is_closed());
}
#[test]
fn latency_delays_first_stream_item_and_exec() {
    let (engine, id) = engine();
    let engine = engine.with_latency(Duration::from_millis(40));
    let start = std::time::Instant::now();
    block_on(engine.logs(&id, LogOptions::default()).next())
        .unwrap()
        .unwrap();
    assert!(start.elapsed() >= Duration::from_millis(40));
    let start = std::time::Instant::now();
    block_on(engine.exec(&id, ExecOptions::shell(TtySize { cols: 80, rows: 24 }))).unwrap();
    assert!(start.elapsed() >= Duration::from_millis(40));
}
#[test]
fn cancellation_wakes_quiet_timer_and_aborts_blocked_send() {
    struct Probe(std::sync::mpsc::Sender<()>);
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    for sleeping in [false, true] {
        let (tx, rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let stream = worker::<(), _, _>(1, None, move |mut sender, timer| async move {
            let _probe = Probe(tx);
            started_tx.send(()).unwrap();
            if sleeping {
                timer.wait(Duration::from_secs(60));
            } else {
                loop {
                    if sender.send(Ok(())).await.is_err() {
                        break;
                    }
                }
            }
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(stream);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}

#[test]
fn million_line_mode_generates_only_requested_tail() {
    let (engine, id) = engine();
    let engine = engine.with_log_history_lines(1_000_000);
    let tail = history(&engine, &id, Some(1), None);
    let c = block_on(engine.inspect_container(&id)).unwrap();
    assert_eq!(
        tail,
        [log_line(
            &c,
            999_999,
            c.created
                .saturating_add(999_999)
                .saturating_mul(1_000_000_000)
        )]
    );
}

#[test]
fn overflowing_event_subscription_ends_for_resync() {
    let (engine, id) = engine();
    let events = engine.events();
    for _ in 0..80 {
        block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    }
    let buffered = block_on(events.collect::<Vec<_>>());
    assert!(buffered.len() <= 65);
    assert!(buffered.iter().all(Result::is_ok));
}

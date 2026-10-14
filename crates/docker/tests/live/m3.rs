#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use berth_docker::{ExecOptions, LogLine, LogOptions, LogStream, TtySize};
use futures::{SinkExt as _, stream::BoxStream};

fn timed<T>(runtime: &Runtime, future: impl std::future::Future<Output = T>) -> T {
    runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(15), future).await })
        .expect("engine operation finishes within 15 seconds")
}
fn create(runtime: &Runtime, docker: &Docker, name: &str, command: &str) -> String {
    runtime
        .block_on(docker.create_container(
            Some(query::CreateContainerOptions {
                name: Some(name.into()),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some("alpine:latest".into()),
                cmd: Some(vec!["sh".into(), "-c".into(), command.into()]),
                ..Default::default()
            },
        ))
        .unwrap()
        .id
}
const fn options(follow: bool, tail: Option<u32>) -> LogOptions {
    LogOptions {
        follow,
        tail,
        since: None,
    }
}
fn logs(
    runtime: &Runtime,
    stream: BoxStream<'static, berth_docker::EngineResult<LogLine>>,
) -> Vec<LogLine> {
    timed(runtime, stream.map(Result::unwrap).collect())
}
fn direct_exec(runtime: &Runtime, docker: &Docker, id: &str, command: &str) {
    runtime.block_on(async {
        let exec = docker
            .create_exec(
                id,
                bollard::exec::CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", command]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        docker
            .start_exec(
                &exec.id,
                Some(bollard::exec::StartExecOptions {
                    detach: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
    });
}

#[test]
#[ignore = "requires Docker and creates only a uniquely named throwaway alpine container"]
fn live_logs_tags_order_tail_since_and_follow() {
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    let name = test_container_name().unwrap();
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let id = create(
        &runtime,
        &docker,
        &name,
        "printf 'out-one\\n'; sleep 0.1; printf 'err-one\\n' >&2; sleep 0.1; printf 'out-two\\n'; while [ ! -f /tmp/next ]; do sleep 0.1; done; printf 'follow-line\\n'; printf 'unterminated'; sleep 300",
    );
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Start),
    )
    .unwrap();
    let mut follow = engine.logs(&id, options(true, None));
    let mut initial = Vec::new();
    for _ in 0..3 {
        initial.push(timed(&runtime, follow.next()).unwrap().unwrap());
    }
    assert_eq!(
        initial
            .iter()
            .map(|l| (l.stream, l.line.as_str()))
            .collect::<Vec<_>>(),
        [
            (LogStream::Stdout, "out-one"),
            (LogStream::Stderr, "err-one"),
            (LogStream::Stdout, "out-two")
        ]
    );
    assert!(initial.iter().all(|l| l.timestamp.is_some()));
    assert!(initial.windows(2).all(
        |pair| pair.first().and_then(|l| l.timestamp) <= pair.last().and_then(|l| l.timestamp)
    ));
    let tail = logs(&runtime, engine.logs(&id, options(false, Some(2))));
    assert_eq!(tail, initial.iter().skip(1).cloned().collect::<Vec<_>>());
    assert_eq!(
        logs(&runtime, engine.logs(&id, options(false, Some(0)))),
        []
    );
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_add(3600);
    assert_eq!(
        logs(
            &runtime,
            engine.logs(
                &id,
                LogOptions {
                    since: Some(i64::try_from(since).unwrap()),
                    ..options(false, None)
                }
            )
        ),
        []
    );
    direct_exec(&runtime, &docker, &id, "touch /tmp/next");
    assert_eq!(
        timed(&runtime, follow.next()).unwrap().unwrap().line,
        "follow-line"
    );
    timed(
        &runtime,
        engine.container_action(
            &id,
            ContainerAction::Stop {
                timeout_secs: Some(1),
            },
        ),
    )
    .unwrap();
    assert_eq!(
        timed(&runtime, follow.next()).unwrap().unwrap().line,
        "unterminated"
    );
    assert!(timed(&runtime, follow.next()).is_none());
    let errors = timed(
        &runtime,
        engine
            .logs(&format!("{name}-missing"), options(false, None))
            .collect::<Vec<_>>(),
    );
    assert!(matches!(errors.as_slice(), [Err(EngineError::NotFound(_))]));
}

#[test]
#[ignore = "requires Docker and creates only a uniquely named throwaway alpine container"]
fn live_stats_samples_and_end_on_pause() {
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    let name = test_container_name().unwrap();
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let id = create(&runtime, &docker, &name, "sleep 300");
    assert!(timed(&runtime, engine.stats(&id).next()).is_none());
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Start),
    )
    .unwrap();
    let mut stats = engine.stats(&id);
    let mut samples = Vec::new();
    for _ in 0..2 {
        samples.push(timed(&runtime, stats.next()).unwrap().unwrap());
    }
    assert!(
        samples
            .iter()
            .all(|s| s.cpu_percent.is_finite() && s.cpu_percent >= 0.0 && s.mem_limit > 0)
    );
    assert!(
        samples
            .windows(2)
            .all(|p| p.first().map(|s| s.time_nanos) < p.last().map(|s| s.time_nanos))
    );
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Pause),
    )
    .unwrap();
    timed(&runtime, async {
        while let Some(sample) = stats.next().await {
            sample.unwrap();
        }
    });
    assert!(timed(&runtime, engine.stats(&id).next()).is_none());
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Unpause),
    )
    .unwrap();
}
async fn read_until(session: &mut berth_docker::ExecSession, text: &str) -> String {
    let mut bytes = Vec::new();
    while let Some(chunk) = session.stdout.next().await {
        bytes.extend(chunk.unwrap());
        if String::from_utf8_lossy(&bytes).contains(text) {
            break;
        }
    }
    let output = String::from_utf8_lossy(&bytes).into_owned();
    assert!(output.contains(text), "expected {text:?} in {output:?}");
    output
}
#[test]
#[ignore = "requires Docker and creates only a uniquely named throwaway alpine container"]
fn live_exec_shell_echo_initial_size_resize_and_exit() {
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    let name = test_container_name().unwrap();
    let _cleanup = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: name.clone(),
    };
    let id = create(&runtime, &docker, &name, "sleep 300");
    let options = ExecOptions {
        cmd: vec!["sh".into()],
        env: vec!["BERTH_EXEC_TEST=present".into()],
        working_dir: Some("/tmp".into()),
        user: Some("root".into()),
        size: TtySize { cols: 80, rows: 24 },
    };
    assert!(matches!(
        timed(&runtime, engine.exec(&id, options.clone())),
        Err(EngineError::Conflict(_))
    ));
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Start),
    )
    .unwrap();
    let mut session = timed(&runtime, engine.exec(&id, options)).unwrap();
    timed(
        &runtime,
        session
            .stdin
            .send(b"echo hi\nstty size\npwd\necho $BERTH_EXEC_TEST\n".to_vec()),
    )
    .unwrap();
    let output = timed(&runtime, read_until(&mut session, "\r\npresent\r\n"));
    for expected in ["\r\nhi\r\n", "24 80\r\n", "\r\n/tmp\r\n"] {
        assert!(output.contains(expected), "{output:?}");
    }
    // Many immediate requests coalesce; wait for Docker to apply the final
    // value by observing stty, since the public resizer intentionally has no ack.
    for cols in 81..=120 {
        session.resizer.resize(TtySize { cols, rows: 40 });
    }
    timed(&runtime, async {
        loop {
            session.stdin.send(b"stty size\n".to_vec()).await.unwrap();
            let output = read_until(&mut session, "# ").await;
            if output.contains("40 120\r\n") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    timed(&runtime, session.stdin.send(b"exit\n".to_vec())).unwrap();
    timed(&runtime, async {
        while let Some(chunk) = session.stdout.next().await {
            chunk.unwrap();
        }
    });
    // EOF also tears down session pumps even while these handles are retained.
    for _ in 0..100 {
        if session.stdin.is_closed() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.stdin.is_closed());
}

#[cfg(unix)]
mod cancellation;

#[test]
#[ignore = "requires a real engine and a throwaway alpine container"]
fn live_logs_join_long_records_and_keep_a_tty_final_fragment() {
    let endpoint = endpoint().unwrap();
    let engine = BollardEngine::connect(&endpoint).unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    ensure_test_image(&runtime, &docker).unwrap();
    // Docker splits lines over 16 KiB into records, each with a timestamp.
    let long = test_container_name().unwrap();
    let _long = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: long.clone(),
    };
    let id = create(
        &runtime,
        &docker,
        &long,
        "head -c 40000 /dev/zero | tr '\\0' x; printf '\\nshort\\n'",
    );
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Start),
    )
    .unwrap();
    timed(&runtime, async {
        while docker
            .inspect_container(&id, None::<query::InspectContainerOptions>)
            .await
            .ok()
            .and_then(|c| c.state)
            .and_then(|s| s.running)
            .unwrap_or(false)
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    let lines = logs(&runtime, engine.logs(&id, options(false, None)));
    assert_eq!(
        lines.len(),
        2,
        "{:?}",
        lines.iter().map(|l| l.line.len()).collect::<Vec<_>>()
    );
    assert_eq!(lines[0].line, "x".repeat(40_000));
    assert_eq!(lines[1].line, "short");

    // A TTY container whose last output has no newline.
    let tty = test_container_name().unwrap();
    let _tty = Cleanup {
        runtime: &runtime,
        docker: &docker,
        name: tty.clone(),
    };
    let id = runtime
        .block_on(docker.create_container(
            Some(query::CreateContainerOptions {
                name: Some(tty),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some("alpine:latest".into()),
                tty: Some(true),
                cmd: Some(vec![
                    "sh".into(),
                    "-c".into(),
                    "printf 'line\\npartial'".into(),
                ]),
                ..Default::default()
            },
        ))
        .unwrap()
        .id;
    timed(
        &runtime,
        engine.container_action(&id, ContainerAction::Start),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let stream = engine.logs(&id, options(false, None));
    let items: Vec<_> = timed(&runtime, stream.collect::<Vec<_>>());
    let lines: Vec<_> = items.into_iter().map(|item| item.unwrap().line).collect();
    assert_eq!(lines, ["line", "partial"]);
}

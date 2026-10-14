//! Bounded live streams and attached sessions, driven by the engine runtime.
mod logs;
mod stats;

use std::pin::Pin;

use bollard::{
    Docker,
    container::LogOutput,
    exec::{StartExecOptions, StartExecResults},
    models::ExecConfig,
    query_parameters as query,
};
use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, SinkExt as _, Stream, StreamExt as _};
use tokio::io::AsyncWriteExt as _;
use tokio::runtime::Handle;
use tokio::sync::watch;

use tokio_util::task::AbortOnDrop;

use super::{BollardEngine, TaskStream, mapping};
use crate::{
    EngineError, EngineResult, ExecOptions, ExecSession, LogLine, LogOptions, StatsSample,
    TtyResizer, TtySize,
};

impl BollardEngine {
    pub(super) fn live_logs(
        &self,
        id: &str,
        options: LogOptions,
    ) -> BoxStream<'static, EngineResult<LogLine>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        let (mut sender, receiver) = mpsc::channel(1024);
        let task = self.runtime().spawn(async move {
            if let Err(error) = pump_logs(&docker, &id, options, &mut sender).await {
                let _ = sender.send(Err(error)).await;
            }
        });
        TaskStream {
            receiver,
            _tasks: vec![AbortOnDrop::new(task.abort_handle())],
        }
        .boxed()
    }
    pub(super) fn live_stats(&self, id: &str) -> BoxStream<'static, EngineResult<StatsSample>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        let (mut sender, receiver) = mpsc::channel(4);
        let task = self.runtime().spawn(async move {
            if let Err(error) = pump_stats(&docker, &id, &mut sender).await {
                let _ = sender.send(Err(error)).await;
            }
        });
        TaskStream {
            receiver,
            _tasks: vec![AbortOnDrop::new(task.abort_handle())],
        }
        .boxed()
    }
    pub(super) fn live_exec(
        &self,
        id: &str,
        options: ExecOptions,
    ) -> BoxFuture<'static, EngineResult<ExecSession>> {
        let docker = self.docker.clone();
        let runtime = self.runtime().clone();
        let id = id.to_owned();
        self.spawn(async move { start_session(docker, runtime, &id, options).await })
    }
}
async fn pump_logs(
    docker: &Docker,
    id: &str,
    options: LogOptions,
    sender: &mut mpsc::Sender<EngineResult<LogLine>>,
) -> EngineResult<()> {
    let tty = docker
        .inspect_container(id, None::<query::InspectContainerOptions>)
        .await
        .map_err(mapping::error)?
        .config
        .and_then(|config| config.tty)
        .unwrap_or(false);
    let mut body = docker.export_container(&logs::path(id, &options));
    let mut assembler = logs::Assembler::new(tty);
    let mut failure = None;
    while let Some(chunk) = body.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                failure = Some(mapping::error(error));
                break;
            }
        };
        for line in assembler.feed(&chunk) {
            if logs::after_since(&line, options.since) && sender.send(Ok(line)).await.is_err() {
                return Ok(());
            }
        }
    }
    if failure.is_none() && assembler.truncated() {
        failure = Some(EngineError::Transport(
            "log stream ended inside a record".into(),
        ));
    }
    for line in assembler.finish() {
        if logs::after_since(&line, options.since) && sender.send(Ok(line)).await.is_err() {
            return Ok(());
        }
    }
    failure.map_or(Ok(()), Err)
}
async fn pump_stats(
    docker: &Docker,
    id: &str,
    sender: &mut mpsc::Sender<EngineResult<StatsSample>>,
) -> EngineResult<()> {
    // Events are requested from this instant, so a stop or pause between
    // the inspect and the subscription is replayed rather than missed.
    let subscribed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let inspected = docker
        .inspect_container(id, None::<query::InspectContainerOptions>)
        .await
        .map_err(mapping::error)?;
    let state = inspected.state.unwrap_or_default();
    if !state.running.unwrap_or(false) || state.paused.unwrap_or(false) {
        return Ok(());
    }
    let mut output = docker.stats(
        id,
        Some(query::StatsOptions {
            stream: true,
            one_shot: false,
        }),
    );
    // Docker's stats subscription can survive process exit. Events terminate
    // it without a separate polling timer (including paused containers).
    let filters = std::collections::HashMap::from([
        ("type".into(), vec!["container".into()]),
        ("container".into(), vec![id.to_owned()]),
        (
            "event".into(),
            ["die", "stop", "pause", "destroy"]
                .map(str::to_owned)
                .to_vec(),
        ),
    ]);
    let mut events = docker.events(Some(query::EventsOptions {
        since: Some(format!(
            "{}.{:09}",
            subscribed.as_secs(),
            subscribed.subsec_nanos()
        )),
        filters: Some(filters),
        ..Default::default()
    }));
    loop {
        futures::select! {
            event = events.next().fuse() => {
                if let Some(event) = event { event.map_err(mapping::error)?; }
                break;
            },
            value = output.next().fuse() => {
                let Some(value) = value else { break; };
                let sample = stats::sample(&value.map_err(mapping::error)?);
                if sender.send(Ok(sample)).await.is_err() { break; }
            }
        }
    }
    Ok(())
}
fn resize_options(size: TtySize) -> query::ResizeExecOptions {
    query::ResizeExecOptions {
        h: i32::from(size.rows),
        w: i32::from(size.cols),
    }
}
async fn start_session(
    docker: Docker,
    runtime: Handle,
    id: &str,
    options: ExecOptions,
) -> EngineResult<ExecSession> {
    let size = options.size;
    let created = docker
        .create_exec(
            id,
            ExecConfig {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                console_size: Some(vec![usize::from(size.rows), usize::from(size.cols)]),
                cmd: Some(options.cmd),
                env: Some(options.env),
                working_dir: options.working_dir,
                user: options.user,
                ..Default::default()
            },
        )
        .await
        .map_err(mapping::error)?;
    let StartExecResults::Attached { input, output } = docker
        .start_exec(
            &created.id,
            Some(StartExecOptions {
                tty: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(mapping::error)?
    else {
        return Err(EngineError::Transport("exec unexpectedly detached".into()));
    };
    if let Err(error) = docker.resize_exec(&created.id, resize_options(size)).await {
        // Short-lived commands may exit before the post-start resize. Their
        // console was sized at creation; preserve any buffered final output.
        if !docker
            .inspect_exec(&created.id)
            .await
            .is_ok_and(|exec| exec.running == Some(false))
        {
            return Err(mapping::error(error));
        }
    }
    Ok(attach_session(
        &runtime, docker, created.id, size, input, output,
    ))
}

type Input = Pin<Box<dyn tokio::io::AsyncWrite + Send>>;
type Output = Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>;

fn attach_session(
    runtime: &Handle,
    docker: Docker,
    id: String,
    size: TtySize,
    mut input: Input,
    mut output: Output,
) -> ExecSession {
    let (stdin, mut input_rx) = mpsc::channel::<Vec<u8>>(64);
    let (mut sender, receiver) = mpsc::channel(64);
    let mut input_errors = sender.clone();
    let input_task = runtime.spawn(async move {
        while let Some(bytes) = input_rx.next().await {
            if let Err(error) = input.write_all(&bytes).await {
                let _ = input_errors
                    .send(Err(EngineError::Transport(error.to_string())))
                    .await;
                return;
            }
        }
        if let Err(error) = input.shutdown().await {
            let _ = input_errors
                .send(Err(EngineError::Transport(error.to_string())))
                .await;
        }
    });
    let (resize_tx, mut resize_rx) = watch::channel(size);
    let mut resize_errors = sender.clone();
    let resize_task = runtime.spawn(async move {
        let mut previous = size;
        while resize_rx.changed().await.is_ok() {
            let latest = *resize_rx.borrow_and_update();
            if latest != previous {
                if let Err(error) = docker.resize_exec(&id, resize_options(latest)).await {
                    let _ = resize_errors.send(Err(mapping::error(error))).await;
                    break;
                }
                previous = latest;
            }
        }
    });
    let input_abort = input_task.abort_handle();
    let resize_abort = resize_task.abort_handle();
    // Also stop the input and resize pumps at remote EOF, even if the caller
    // retains an exhausted stdout stream or its stdin/resizer handles.
    let guards = vec![
        AbortOnDrop::new(input_abort.clone()),
        AbortOnDrop::new(resize_abort.clone()),
    ];
    let output_task = runtime.spawn(async move {
        let _guards = guards;
        while let Some(frame) = output.next().await {
            let value = frame
                .map(|frame| frame.into_bytes().to_vec())
                .map_err(mapping::error);
            let failed = value.is_err();
            if sender.send(value).await.is_err() || failed {
                break;
            }
        }
    });
    ExecSession {
        stdin,
        stdout: TaskStream {
            receiver,
            _tasks: vec![
                AbortOnDrop::new(input_abort),
                AbortOnDrop::new(resize_abort),
                AbortOnDrop::new(output_task.abort_handle()),
            ],
        }
        .boxed(),
        resizer: TtyResizer::new(move |size| {
            resize_tx.send_replace(size);
        }),
    }
}

#[cfg(test)]
mod tests;

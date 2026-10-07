//! Executor-independent, bounded fixture streams. Each subscription owns one
//! worker thread; dropping the stream aborts its future, including any
//! pending timer or channel send.
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::channel::mpsc;
use futures::future::{AbortHandle, Abortable};
use futures::{SinkExt as _, Stream, StreamExt as _};
use futures_timer::Delay;

use super::{FixtureEngine, FixtureState};
use crate::{
    Container, ContainerState, EngineError, EngineResult, ExecOptions, ExecSession, LogLine,
    LogOptions, LogStream, StatsSample, TtyResizer, TtySize,
};

type State = Arc<Mutex<FixtureState>>;

/// Aborts the worker's future when the stream is dropped, waking it even
/// while it sleeps in a timer or waits on a full channel.
struct Cancel(AbortHandle);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct WorkerStream<T> {
    receiver: mpsc::Receiver<EngineResult<T>>,
    _cancel: Cancel,
}
impl<T> Stream for WorkerStream<T> {
    type Item = EngineResult<T>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}
fn worker<T, F, Fut>(
    capacity: usize,
    latency: Option<Duration>,
    work: F,
) -> futures::stream::BoxStream<'static, EngineResult<T>>
where
    T: Send + 'static,
    F: FnOnce(mpsc::Sender<EngineResult<T>>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel(capacity);
    let (abort, registration) = AbortHandle::new_pair();
    std::thread::spawn(move || {
        futures::executor::block_on(Abortable::new(
            async move {
                Delay::new(latency.unwrap_or_default()).await;
                work(sender).await;
            },
            registration,
        ))
        .ok();
    });
    WorkerStream {
        receiver,
        _cancel: Cancel(abort),
    }
    .boxed()
}
fn container(state: &State, id: &str) -> EngineResult<Container> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .containers
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(id.into()))
}
fn running(state: &State, id: &str) -> bool {
    container(state, id).is_ok_and(|c| c.state == ContainerState::Running)
}
fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or_default()
}

impl FixtureEngine {
    /// Configure lazily generated history per container, without allocating it.
    /// For the million-line log-view benchmark, use
    /// `FixtureEngine::default().with_log_history_lines(1_000_000)` and request
    /// `LogOptions { follow: false, tail: None, since: None }`.
    #[must_use]
    pub const fn with_log_history_lines(mut self, lines: u32) -> Self {
        self.log_history_lines = lines;
        self
    }

    pub(super) fn fixture_logs(
        &self,
        id: &str,
        options: LogOptions,
    ) -> futures::stream::BoxStream<'static, EngineResult<LogLine>> {
        let state = self.state.clone();
        let id = id.to_owned();
        let count = self.log_history_lines;
        worker(1024, self.latency, move |mut sender| async move {
            let c = match container(&state, &id) {
                Ok(c) => c,
                Err(e) => {
                    let _ = sender.send(Err(e)).await;
                    return;
                }
            };
            let start = count.saturating_sub(options.tail.unwrap_or(count));
            let base = c.created.saturating_mul(1_000_000_000);
            for index in start..count {
                let time = base.saturating_add(i64::from(index).saturating_mul(1_000_000_000));
                if after_since(time, options.since)
                    && sender.send(Ok(log_line(&c, index, time))).await.is_err()
                {
                    return;
                }
            }
            if !options.follow
                || !matches!(c.state, ContainerState::Running | ContainerState::Paused)
            {
                return;
            }
            let mut index = count;
            let mut time = now_nanos()
                .max(base.saturating_add(i64::from(count).saturating_mul(1_000_000_000)));
            loop {
                let delay = match index % 12 {
                    0..=3 => 80,
                    4..=9 => 400,
                    _ => 1800,
                };
                Delay::new(Duration::from_millis(delay)).await;
                let Ok(current) = container(&state, &id) else {
                    break;
                };
                match current.state {
                    ContainerState::Running => {}
                    ContainerState::Paused => continue,
                    _ => break,
                }
                time = time.saturating_add(
                    i64::try_from(delay)
                        .unwrap_or_default()
                        .saturating_mul(1_000_000),
                );
                if after_since(time, options.since)
                    && sender.send(Ok(log_line(&c, index, time))).await.is_err()
                {
                    break;
                }
                index = index.saturating_add(1);
            }
        })
    }

    pub(super) fn fixture_stats(
        &self,
        id: &str,
    ) -> futures::stream::BoxStream<'static, EngineResult<StatsSample>> {
        let state = self.state.clone();
        let id = id.to_owned();
        worker(4, self.latency, move |mut sender| async move {
            if let Err(e) = container(&state, &id) {
                let _ = sender.send(Err(e)).await;
                return;
            }
            let seed = seed(&id);
            let mut tick = 0_u32;
            let mut time = now_nanos();
            while running(&state, &id) {
                if sender
                    .send(Ok(stats_sample(seed, tick, time)))
                    .await
                    .is_err()
                {
                    break;
                }
                Delay::new(Duration::from_secs(1)).await;
                tick = tick.saturating_add(1);
                time = time.saturating_add(1_000_000_000);
            }
        })
    }

    pub(super) fn fixture_exec(
        &self,
        id: &str,
        options: ExecOptions,
    ) -> futures::future::BoxFuture<'static, EngineResult<ExecSession>> {
        let id = id.to_owned();
        self.mutate(move |state| {
            let c = state
                .containers
                .iter()
                .find(|c| c.id == id)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            if c.state != ContainerState::Running {
                return Err(EngineError::Conflict(format!(
                    "container {id} is not running"
                )));
            }
            Ok(fake_shell(&id, options))
        })
    }
}
const fn after_since(time: i64, since: Option<i64>) -> bool {
    match since {
        Some(since) => time >= since.saturating_mul(1_000_000_000),
        None => true,
    }
}
fn seed(id: &str) -> u32 {
    id.bytes().fold(0_u32, |seed, b| {
        seed.wrapping_mul(31).wrapping_add(u32::from(b))
    })
}
fn log_line(c: &Container, index: u32, time: i64) -> LogLine {
    let n = index.wrapping_add(seed(&c.id));
    let stream = if n.is_multiple_of(11) {
        LogStream::Stderr
    } else {
        LogStream::Stdout
    };
    let message = if stream == LogStream::Stderr {
        "\x1b[1;33mWARN\x1b[0m slow request; retry scheduled".to_owned()
    } else if c.image.contains("postgres") {
        format!(
            "\x1b[32mLOG\x1b[0m checkpoint complete: wrote {} buffers; database ready",
            n % 128
        )
    } else if c.image.contains("redis") {
        format!(
            "1:M \x1b[38;5;81m*\x1b[0m Background saving terminated with success; {} keys",
            n % 4096
        )
    } else if c.image.contains("nginx") {
        format!(
            "172.18.0.1 - - \"GET /api/items?page={} HTTP/1.1\" 200 1432 \"Berth/1.0\"",
            n % 20
        )
    } else {
        format!(
            "\x1b[38;2;100;210;160mINFO\x1b[0m request_id={n:08x} GET /health 200 {}ms",
            n % 90
        )
    };
    let line = if n.is_multiple_of(97) {
        format!("{message} payload={}", "0123456789abcdef".repeat(256))
    } else {
        message
    };
    LogLine {
        stream,
        timestamp: Some(time),
        line,
    }
}
fn stats_sample(seed: u32, tick: u32, time_nanos: i64) -> StatsSample {
    let phase = f64::from(tick).mul_add(0.2, f64::from(seed % 100));
    let noise = f64::from(tick.wrapping_mul(17).wrapping_add(seed) % 9).mul_add(0.3, 0.0);
    let spike = if tick % 23 == 22 { 70.0 } else { 0.0 };
    let count = u64::from(tick).saturating_add(1);
    StatsSample {
        time_nanos,
        cpu_percent: phase.sin().mul_add(8.0, 18.0) + noise + spike,
        online_cpus: 4,
        mem_used: 80_000_000_u64
            .saturating_add(count.saturating_mul(4096))
            .min(400_000_000),
        mem_limit: 536_870_912,
        net_rx: count.saturating_mul(45_123),
        net_tx: count.saturating_mul(12_987),
        blk_read: count.saturating_mul(4096),
        blk_write: count.saturating_mul(8192),
    }
}

fn fake_shell(id: &str, options: ExecOptions) -> ExecSession {
    let (stdin, mut input) = mpsc::channel::<Vec<u8>>(64);
    let size = Arc::new(Mutex::new(options.size));
    let resize_size = size.clone();
    let resizer = TtyResizer::new(move |value| {
        *resize_size
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
    });
    let short: String = id.chars().take(12).collect();
    let pwd = options.working_dir.unwrap_or_else(|| "/".into());
    let user = options.user.unwrap_or_else(|| "root".into());
    let prompt = format!("\x1b[32m{user}@{short}\x1b[0m:{pwd}# ");
    let stdout = worker(64, None, move |mut sender| async move {
        if sender.send(Ok(prompt.as_bytes().to_vec())).await.is_err() {
            return;
        }
        let mut command = Vec::new();
        let mut previous_cr = false;
        while let Some(bytes) = input.next().await {
            let mut output = Vec::new();
            let mut exit = false;
            for byte in bytes {
                match byte {
                    b'\n' if previous_cr => {}
                    b'\r' | b'\n' => {
                        output.extend_from_slice(b"\r\n");
                        let text = String::from_utf8_lossy(&command);
                        exit = text.trim() == "exit";
                        if exit {
                            break;
                        }
                        let size = *size
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        output.extend_from_slice(
                            shell_command(text.trim(), &pwd, &user, size).as_bytes(),
                        );
                        output.extend_from_slice(prompt.as_bytes());
                        command.clear();
                    }
                    0x7f | 0x08 => {
                        if command.pop().is_some() {
                            output.extend_from_slice(b"\x08 \x08");
                        }
                    }
                    0x04 => {
                        exit = true;
                        break;
                    }
                    _ => {
                        if command.len() < 65_536 {
                            command.push(byte);
                            output.push(byte);
                        }
                    }
                }
                previous_cr = byte == b'\r';
            }
            if sender.send(Ok(output)).await.is_err() || exit {
                break;
            }
        }
    });
    ExecSession {
        stdin,
        stdout,
        resizer,
    }
}
fn shell_command(command: &str, pwd: &str, user: &str, size: TtySize) -> String {
    match command {
        "" => String::new(),
        "ls" => "bin  dev  etc  home  proc  root  tmp  usr  var\r\n".into(),
        "pwd" => format!("{pwd}\r\n"),
        "whoami" => format!("{user}\r\n"),
        "stty size" => format!("{} {}\r\n", size.rows, size.cols),
        "tput cols" => format!("{}\r\n", size.cols),
        "echo" => "\r\n".into(),
        _ => command.strip_prefix("echo ").map_or_else(
            || {
                format!(
                    "sh: {}: not found\r\n",
                    command.split_whitespace().next().unwrap_or_default()
                )
            },
            |text| format!("{text}\r\n"),
        ),
    }
}

#[cfg(test)]
mod tests;

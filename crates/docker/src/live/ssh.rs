//! Bollard's custom transport over SSH (see [`crate::ssh`]).
//!
//! Each pooled API connection is one `ssh … docker system dial-stdio`
//! process; hyper's pool reuses idle ones and a stream holds its own. No
//! session of our own to manage: a running `ControlMaster` from the user's
//! ssh config still shares one login.
//!
//! When `ssh` ends before the engine sent anything, its stderr says why, so
//! the stream reports that instead of a bare EOF or broken pipe.
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bollard::{API_DEFAULT_VERSION, BollardRequest, Docker};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::ConnectError;
use crate::ssh::{config_args, dial_args, failure_message, resolved_hostname};

/// A failed SSH connection, carried as the source of an [`io::Error`] so
/// [`super::connect_error`] can tell it from socket errors.
#[derive(Debug)]
pub(super) struct SshFailure(pub(super) String);

impl std::fmt::Display for SshFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SshFailure {}

impl SshFailure {
    /// The SSH failure somewhere in `error`'s source chain, e.g. under
    /// hyper's "client error (Connect)" for a connection opened mid-session.
    pub(super) fn find<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a Self> {
        let mut source = Some(error);
        while let Some(error) = source {
            if let Some(failure) = error.downcast_ref::<Self>() {
                return Some(failure);
            }
            if let Some(failure) = error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .and_then(|inner| inner.downcast_ref::<Self>())
            {
                return Some(failure);
            }
            source = error.source();
        }
        None
    }
}

pub(super) fn client(
    program: &Path,
    destination: &str,
    port: Option<u16>,
) -> Result<Docker, ConnectError> {
    let connector = Connector(Arc::new(Target {
        program: program.to_owned(),
        destination: destination.to_owned(),
        port,
    }));
    // Without a timer the pool never evicts idle connections, and each one
    // keeps an ssh process here and a dial-stdio process on the remote.
    let client = Client::builder(TokioExecutor::new())
        .pool_timer(TokioTimer::new())
        .pool_idle_timeout(Duration::from_secs(30))
        .build(connector);
    Docker::connect_with_custom_transport(
        move |request: BollardRequest| {
            let client = client.clone();
            async move {
                client
                    .request(request)
                    .await
                    .map_err(bollard::errors::Error::from)
            }
        },
        // dial-stdio forwards bytes to the remote socket; any host will do.
        Some("http://docker"),
        120,
        API_DEFAULT_VERSION,
    )
    .map_err(|e| ConnectError::InvalidConfig(e.to_string()))
}

/// The host `ssh` will actually connect to, for published port URLs.
pub(super) async fn hostname(
    program: PathBuf,
    destination: String,
    port: Option<u16>,
) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        command(&program, &config_args(&destination, port))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| resolved_hostname(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn command(program: &Path, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    // Without this every connection flashes a console window on Windows.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

struct Target {
    program: PathBuf,
    destination: String,
    port: Option<u16>,
}

#[derive(Clone)]
struct Connector(Arc<Target>);

impl tower_service::Service<http::Uri> for Connector {
    type Response = TokioIo<SshStream>;
    type Error = io::Error;
    type Future = BoxFuture<'static, io::Result<Self::Response>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: http::Uri) -> Self::Future {
        let target = self.0.clone();
        async move { SshStream::spawn(target).map(TokioIo::new) }.boxed()
    }
}

struct SshStream {
    target: Arc<Target>,
    _child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    /// Collects stderr until `ssh` exits; taken once a failure is reported.
    stderr: Option<tokio::task::JoinHandle<String>>,
    failure: Option<BoxFuture<'static, String>>,
    /// Whether the engine has sent anything: after that, EOF is just EOF.
    answered: bool,
}

impl SshStream {
    fn spawn(target: Arc<Target>) -> io::Result<Self> {
        let mut child = command(
            &target.program,
            &dial_args(&target.destination, target.port),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            io::Error::other(SshFailure(format!(
                "couldn't start {}: {e}",
                target.program.display()
            )))
        })?;
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(io::Error::other("ssh started without piped stdio"));
        };
        let stderr = tokio::spawn(async move {
            let mut text = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut text).await;
            // Keep draining so a chatty ssh never blocks on a full pipe.
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            String::from_utf8_lossy(&text).into_owned()
        });
        Ok(Self {
            target,
            _child: child,
            stdin: Some(stdin),
            stdout,
            stderr: Some(stderr),
            failure: None,
            answered: false,
        })
    }

    /// Writes or flushes stdin; `closed` is the result once it's shut down.
    /// A broken pipe before the engine answered means `ssh` exited, e.g. it
    /// couldn't resolve the host, so report why instead. Windows buffers
    /// pipe writes, so that can surface from a flush as well as a write.
    fn poll_stdin<T>(
        &mut self,
        cx: &mut Context<'_>,
        operation: impl FnOnce(Pin<&mut ChildStdin>, &mut Context<'_>) -> Poll<io::Result<T>>,
        closed: io::Result<T>,
    ) -> Poll<io::Result<T>> {
        if self.failure.is_none() {
            let Some(stdin) = self.stdin.as_mut() else {
                return Poll::Ready(closed);
            };
            match ready!(operation(Pin::new(stdin), cx)) {
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe && !self.answered => {}
                result => return Poll::Ready(result),
            }
        }
        Poll::Ready(Err(ready!(self.poll_failure(cx))))
    }

    /// The error explaining why `ssh` ended, from what it wrote to stderr.
    fn poll_failure(&mut self, cx: &mut Context<'_>) -> Poll<io::Error> {
        if self.failure.is_none() {
            let stderr = self.stderr.take();
            // A daemonised ControlMaster could hold stderr open; don't wait on it.
            self.failure = Some(
                async move {
                    match stderr {
                        Some(stderr) => tokio::time::timeout(Duration::from_secs(2), stderr)
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .unwrap_or_default(),
                        None => String::new(),
                    }
                }
                .boxed(),
            );
        }
        let Some(failure) = self.failure.as_mut() else {
            return Poll::Pending;
        };
        let stderr = ready!(failure.as_mut().poll(cx));
        self.failure = None;
        let message = failure_message(&self.target.destination, self.target.port, &stderr);
        Poll::Ready(io::Error::other(SshFailure(message)))
    }
}

impl AsyncRead for SshStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failure.is_none() {
            let before = buf.filled().len();
            ready!(Pin::new(&mut this.stdout).poll_read(cx, buf))?;
            if this.answered || buf.filled().len() > before {
                this.answered = true;
                return Poll::Ready(Ok(()));
            }
        }
        Poll::Ready(Err(ready!(this.poll_failure(cx))))
    }
}

impl AsyncWrite for SshStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut().poll_stdin(
            cx,
            |stdin, cx| stdin.poll_write(cx, data),
            Err(io::ErrorKind::BrokenPipe.into()),
        )
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut()
            .poll_stdin(cx, AsyncWrite::poll_flush, Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Closing stdin tells dial-stdio to close its half of the socket.
        self.get_mut().stdin = None;
        Poll::Ready(Ok(()))
    }
}

impl Connection for SshStream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

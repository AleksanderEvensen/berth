//! Live Docker/Podman API client with a private Tokio runtime.
//!
//! `connect` performs a bounded handshake and should be called on a background
//! thread. All [`Engine`] futures and streams can then run on any executor.
//! Dropping a result cancels its task; dropping the engine cancels remaining
//! work and shuts down the runtime without joining its threads.
mod mapping;
mod tls;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bollard::{
    API_DEFAULT_VERSION, Docker, errors::Error as BollardError, query_parameters as query,
};
use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, SinkExt as _, Stream, StreamExt as _};
use tokio::runtime::{Handle, Runtime};
use tokio::task::AbortHandle;

use crate::{
    ConnectError, Container, ContainerAction, DiskUsage, Endpoint, Engine, EngineError,
    EngineEvent, EngineInfo, EngineResult, ExecOptions, ExecSession, Image, ImagePrune, LogLine,
    LogOptions, PruneReport, RemoveOptions, StatsSample, Volume,
};

/// A live engine whose Tokio work never depends on the caller's executor.
pub struct BollardEngine {
    docker: Docker,
    endpoint: Endpoint,
    runtime: EngineRuntime,
}

// Tasks retain handles, never an Arc to this owner, so engine drop always
// initiates shutdown even while a caller still holds a future or stream.
struct EngineRuntime {
    owner: Option<Runtime>,
    handle: Handle,
}
impl EngineRuntime {
    fn new() -> Result<Self, ConnectError> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("berth-engine")
            .enable_all()
            .build()
            .map(|runtime| Self {
                handle: runtime.handle().clone(),
                owner: Some(runtime),
            })
            .map_err(|e| ConnectError::InvalidConfig(e.to_string()))
    }
    const fn get(&self) -> &Handle {
        &self.handle
    }
}
impl Drop for EngineRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.owner.take() {
            runtime.shutdown_background();
        }
    }
}

struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct EventStream {
    receiver: mpsc::Receiver<EngineResult<EngineEvent>>,
    _task: AbortOnDrop,
}
impl Stream for EventStream {
    type Item = EngineResult<EngineEvent>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl BollardEngine {
    /// Validate the endpoint, negotiate the API version and ping the daemon.
    /// The synchronous handshake is bounded to ten seconds; call this on a
    /// background thread when connecting from a UI.
    ///
    /// # Errors
    /// Returns an error for invalid endpoint or TLS settings, transport failures,
    /// or a failed or timed-out daemon handshake.
    pub fn connect(endpoint: &Endpoint) -> Result<Self, ConnectError> {
        let runtime = EngineRuntime::new()?;
        let docker = {
            let _entered = runtime.get().enter();
            client(endpoint)?
        };
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        runtime.get().spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                let docker = docker.negotiate_version().await?;
                docker.ping().await?;
                Ok::<_, BollardError>(docker)
            })
            .await;
            let _ = sender.send(result);
        });
        let docker = receiver
            .recv()
            .map_err(|e| ConnectError::InvalidConfig(e.to_string()))?
            .map_err(|_| ConnectError::Refused {
                endpoint: endpoint.to_string(),
            })?
            .map_err(|e| connect_error(&e, endpoint))?;
        Ok(Self {
            docker,
            endpoint: endpoint.clone(),
            runtime,
        })
    }

    const fn runtime(&self) -> &Handle {
        self.runtime.get()
    }

    fn spawn<T: Send + 'static>(
        &self,
        work: impl Future<Output = EngineResult<T>> + Send + 'static,
    ) -> BoxFuture<'static, EngineResult<T>> {
        let (sender, receiver) = oneshot::channel();
        let task = self.runtime().spawn(async move {
            let _ = sender.send(work.await);
        });
        // Capture the guard before returning: even an unpolled future cancels.
        let guard = AbortOnDrop(task.abort_handle());
        async move {
            let _guard = guard;
            receiver.await.unwrap_or_else(|_| {
                Err(EngineError::Transport(
                    "engine task ended before delivering a result".into(),
                ))
            })
        }
        .boxed()
    }
}

fn client(endpoint: &Endpoint) -> Result<Docker, ConnectError> {
    let result = match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(path) => {
            // Unlike Bollard's exists() check this preserves EACCES.
            std::fs::metadata(path).map_err(|e| connect_io_error(&e, endpoint))?;
            let path = path
                .to_str()
                .ok_or_else(|| ConnectError::InvalidConfig("socket path is not UTF-8".into()))?;
            Docker::connect_with_unix(path, 120, API_DEFAULT_VERSION)
        }
        #[cfg(not(unix))]
        Endpoint::Unix(_) => {
            return Err(ConnectError::InvalidConfig(
                "Unix sockets are unsupported on this platform".into(),
            ));
        }
        #[cfg(windows)]
        Endpoint::NamedPipe(pipe) => {
            Docker::connect_with_named_pipe(pipe, 120, API_DEFAULT_VERSION)
        }
        #[cfg(not(windows))]
        Endpoint::NamedPipe(_) => {
            return Err(ConnectError::InvalidConfig(
                "named pipes require Windows".into(),
            ));
        }
        Endpoint::Tcp {
            url,
            tls: Some(config),
        } => return tls::client(url, config),
        Endpoint::Tcp { url, tls: None } => {
            if url.starts_with("https://") {
                return Err(ConnectError::InvalidConfig(
                    "HTTPS requires a TLS configuration".into(),
                ));
            }
            if !url.starts_with("tcp://") && !url.starts_with("http://") {
                return Err(ConnectError::InvalidConfig(
                    "TCP endpoint must use tcp:// or http://".into(),
                ));
            }
            Docker::connect_with_http(url, 120, API_DEFAULT_VERSION)
        }
    };
    result.map_err(|e| connect_error(&e, endpoint))
}

impl Engine for BollardEngine {
    fn info(&self) -> BoxFuture<'static, EngineResult<EngineInfo>> {
        let docker = self.docker.clone();
        let endpoint = self.endpoint.clone();
        self.spawn(async move {
            let (info, version) =
                futures::try_join!(docker.info(), docker.version()).map_err(mapping::error)?;
            Ok(mapping::info(info, version, endpoint))
        })
    }

    fn list_containers(&self) -> BoxFuture<'static, EngineResult<Vec<Container>>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            let values = docker
                .list_containers(Some(query::ListContainersOptions {
                    all: true,
                    ..Default::default()
                }))
                .await
                .map_err(mapping::error)?;
            Ok(values.into_iter().map(mapping::summary).collect())
        })
    }

    fn inspect_container(&self, id: &str) -> BoxFuture<'static, EngineResult<Container>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        self.spawn(async move {
            let value = docker
                .inspect_container(&id, None::<query::InspectContainerOptions>)
                .await
                .map_err(mapping::error)?;
            Ok(mapping::inspect(value, chrono::Utc::now()))
        })
    }

    fn container_action(
        &self,
        id: &str,
        action: ContainerAction,
    ) -> BoxFuture<'static, EngineResult<()>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        self.spawn(async move {
            let result = match action {
                ContainerAction::Start => {
                    docker
                        .start_container(&id, None::<query::StartContainerOptions>)
                        .await
                }
                ContainerAction::Stop { timeout_secs } => {
                    docker
                        .stop_container(
                            &id,
                            Some(query::StopContainerOptions {
                                t: timeout(timeout_secs)?,
                                ..Default::default()
                            }),
                        )
                        .await
                }
                ContainerAction::Restart { timeout_secs } => {
                    docker
                        .restart_container(
                            &id,
                            Some(query::RestartContainerOptions {
                                t: timeout(timeout_secs)?,
                                ..Default::default()
                            }),
                        )
                        .await
                }
                ContainerAction::Kill => {
                    docker
                        .kill_container(&id, None::<query::KillContainerOptions>)
                        .await
                }
                ContainerAction::Pause => docker.pause_container(&id).await,
                ContainerAction::Unpause => docker.unpause_container(&id).await,
            };
            action_result(action, result)
        })
    }

    fn remove_container(
        &self,
        id: &str,
        options: RemoveOptions,
    ) -> BoxFuture<'static, EngineResult<()>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        self.spawn(async move {
            docker
                .remove_container(
                    &id,
                    Some(query::RemoveContainerOptions {
                        force: options.force,
                        v: options.volumes,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(mapping::error)
        })
    }

    fn list_images(&self) -> BoxFuture<'static, EngineResult<Vec<Image>>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            let values = docker
                .list_images(Some(query::ListImagesOptions {
                    all: true,
                    ..Default::default()
                }))
                .await
                .map_err(mapping::error)?;
            Ok(values.into_iter().map(mapping::image).collect())
        })
    }

    fn list_volumes(&self) -> BoxFuture<'static, EngineResult<Vec<Volume>>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            let values = docker
                .list_volumes(None::<query::ListVolumesOptions>)
                .await
                .map_err(mapping::error)?;
            Ok(values
                .volumes
                .unwrap_or_default()
                .into_iter()
                .map(|value| {
                    let mut volume = mapping::volume(value);
                    volume.size = None;
                    volume.ref_count = None;
                    volume
                })
                .collect())
        })
    }

    fn disk_usage(&self) -> BoxFuture<'static, EngineResult<DiskUsage>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            let value = docker
                .df(None::<query::DataUsageOptions>)
                .await
                .map_err(mapping::error)?;
            Ok(mapping::disk_usage(value))
        })
    }

    fn remove_image(&self, id: &str, force: bool) -> BoxFuture<'static, EngineResult<()>> {
        let docker = self.docker.clone();
        let id = id.to_owned();
        self.spawn(async move {
            docker
                .remove_image(
                    &id,
                    Some(query::RemoveImageOptions {
                        force,
                        ..Default::default()
                    }),
                    None,
                )
                .await
                .map(|_| ())
                .map_err(mapping::error)
        })
    }

    fn remove_volume(&self, name: &str, force: bool) -> BoxFuture<'static, EngineResult<()>> {
        let docker = self.docker.clone();
        let name = name.to_owned();
        self.spawn(async move {
            docker
                .remove_volume(&name, Some(query::RemoveVolumeOptions { force }))
                .await
                .map_err(mapping::error)
        })
    }

    fn prune_containers(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            docker
                .prune_containers(None::<query::PruneContainersOptions>)
                .await
                .map(mapping::pruned_containers)
                .map_err(mapping::error)
        })
    }

    fn prune_images(&self, mode: ImagePrune) -> BoxFuture<'static, EngineResult<PruneReport>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            docker
                .prune_images(Some(image_prune_options(mode)))
                .await
                .map(mapping::pruned_images)
                .map_err(mapping::error)
        })
    }

    fn prune_volumes(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            docker
                .prune_volumes(Some(volume_prune_options()))
                .await
                .map(mapping::pruned_volumes)
                .map_err(mapping::error)
        })
    }

    fn prune_networks(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            docker
                .prune_networks(None::<query::PruneNetworksOptions>)
                .await
                .map(mapping::pruned_networks)
                .map_err(mapping::error)
        })
    }

    fn prune_build_cache(&self) -> BoxFuture<'static, EngineResult<PruneReport>> {
        let docker = self.docker.clone();
        self.spawn(async move {
            docker
                .prune_build(Some(query::PruneBuildOptions {
                    all: Some(true),
                    ..Default::default()
                }))
                .await
                .map(mapping::pruned_build_cache)
                .map_err(mapping::error)
        })
    }

    fn events(&self) -> BoxStream<'static, EngineResult<EngineEvent>> {
        let docker = self.docker.clone();
        let (mut sender, receiver) = mpsc::channel(64);
        let task = self.runtime().spawn(async move {
            let filters = std::collections::HashMap::from([(
                "type".into(),
                ["container", "image", "volume", "network"]
                    .map(str::to_owned)
                    .to_vec(),
            )]);
            let mut events = docker.events(Some(query::EventsOptions {
                filters: Some(filters),
                ..Default::default()
            }));
            while let Some(value) = events.next().await {
                match value {
                    Ok(value) => {
                        if let Some(event) = mapping::event(value)
                            && sender.send(Ok(event)).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(mapping::error(error))).await;
                        break;
                    }
                }
            }
        });
        EventStream {
            receiver,
            _task: AbortOnDrop(task.abort_handle()),
        }
        .boxed()
    }

    // M3 stubs: replaced by the real implementations.
    fn logs(&self, _id: &str, _options: LogOptions) -> BoxStream<'static, EngineResult<LogLine>> {
        futures::stream::once(async { Err(EngineError::Transport("logs: not implemented".into())) })
            .boxed()
    }

    fn stats(&self, _id: &str) -> BoxStream<'static, EngineResult<StatsSample>> {
        futures::stream::once(async {
            Err(EngineError::Transport("stats: not implemented".into()))
        })
        .boxed()
    }

    fn exec(
        &self,
        _id: &str,
        _options: ExecOptions,
    ) -> BoxFuture<'static, EngineResult<ExecSession>> {
        async { Err(EngineError::Transport("exec: not implemented".into())) }.boxed()
    }
}

fn image_prune_options(mode: ImagePrune) -> query::PruneImagesOptions {
    query::PruneImagesOptions {
        filters: Some(std::collections::HashMap::from([(
            "dangling".into(),
            vec![(mode == ImagePrune::Dangling).to_string()],
        )])),
    }
}

fn volume_prune_options() -> query::PruneVolumesOptions {
    query::PruneVolumesOptions {
        // Since Docker 23, the default only prunes anonymous volumes.
        filters: Some(std::collections::HashMap::from([(
            "all".into(),
            vec!["true".into()],
        )])),
    }
}

fn timeout(seconds: Option<u32>) -> EngineResult<Option<i32>> {
    seconds.map(i32::try_from).transpose().map_err(|_| {
        EngineError::Connect(ConnectError::InvalidConfig(
            "container timeout exceeds i32::MAX seconds".into(),
        ))
    })
}

fn action_result(action: ContainerAction, result: Result<(), BollardError>) -> EngineResult<()> {
    match result {
        Err(BollardError::DockerResponseServerError {
            status_code: 304, ..
        }) if action == ContainerAction::Start => Ok(()),
        other => other.map_err(mapping::error),
    }
}

fn connect_io_error(error: &std::io::Error, endpoint: &Endpoint) -> ConnectError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ConnectError::PermissionDenied {
            endpoint: endpoint.to_string(),
        },
        std::io::ErrorKind::ConnectionRefused => ConnectError::Refused {
            endpoint: endpoint.to_string(),
        },
        std::io::ErrorKind::NotFound => ConnectError::NotFound {
            tried: vec![endpoint.to_string()],
        },
        _ => ConnectError::InvalidConfig(error.to_string()),
    }
}

fn connect_error(error: &BollardError, endpoint: &Endpoint) -> ConnectError {
    // Thiserror's transparent IOError wrapper forwards the inner source;
    // downcasting the wrapper's source alone can lose a source-less IO error.
    if let BollardError::IOError { err } = error {
        return connect_io_error(err, endpoint);
    }
    if matches!(error, BollardError::SocketNotFoundError(_)) {
        return ConnectError::NotFound {
            tried: vec![endpoint.to_string()],
        };
    }
    // Hyper wraps connect failures, sometimes more than one source deep.
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            return connect_io_error(error, endpoint);
        }
        source = error.source();
    }
    ConnectError::InvalidConfig(error.to_string())
}

#[cfg(test)]
mod tests;

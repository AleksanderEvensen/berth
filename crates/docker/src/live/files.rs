//! Browsing files through a helper container (see [`crate::files`]):
//! starting it, keeping it alive, removing it, and running its commands.

use std::collections::HashMap;
use std::time::Duration;

use bollard::{
    Docker,
    container::LogOutput,
    exec::{StartExecOptions, StartExecResults},
    models::{ContainerCreateBody, ExecConfig, HostConfig},
    query_parameters as query,
};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};

use super::{BollardEngine, client, mapping};
use crate::{
    CONTAINER_ROOT, Endpoint, EngineError, EngineResult, FileEntry, FileKind, FileMount,
    FilesTarget, HELPER_IMAGE, HELPER_LABEL, LIST_TIMEOUT_SECS, VOLUME_ROOT, apply_link_targets,
    escape_name, parse_listing,
};

/// Output larger than this fails rather than being buffered.
const MAX_OUTPUT: usize = 32 << 20;

/// Helpers stuck unstarted for this long are orphans.
const ORPHAN_AGE_SECS: i64 = 600;

/// How long a command in the helper may take in all, beyond its own
/// `timeout`: a call blocked on a hung network mount ignores that signal.
const RUN_DEADLINE: Duration = Duration::from_secs(LIST_TIMEOUT_SECS.saturating_add(10));

/// How long removing a helper may take before it is left to `auto_remove`.
const REMOVE_TIMEOUT: Duration = Duration::from_secs(15);

impl BollardEngine {
    pub(super) fn live_open_files(
        &self,
        target: &FilesTarget,
    ) -> BoxFuture<'static, EngineResult<FileMount>> {
        let endpoint = self.endpoint.clone();
        let target = target.clone();
        let (root, confined) = target_root(&target);
        let (ready, started) = oneshot::channel();
        let (release, released) = oneshot::channel::<()>();
        // The helper's keeper: on a thread, runtime and connection of its
        // own, independent of the engine (a reconnect) and of the request.
        // Dropping the request before it completes drops `release` too.
        std::thread::spawn(move || keep_helper(&endpoint, &target, ready, released));
        started
            .map(move |result| {
                let id = result.unwrap_or_else(|_| {
                    Err(EngineError::Transport(
                        "helper thread ended before delivering a result".into(),
                    ))
                })?;
                Ok(FileMount::new(id, root.into(), confined, move || {
                    drop(release);
                }))
            })
            .boxed()
    }

    pub(super) fn live_list_files(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<Vec<FileEntry>>> {
        let docker = self.docker.clone();
        let helper = mount.container.clone();
        let command = mount.list_command(path).map_err(EngineError::Unsupported);
        let targets = mount
            .link_targets_command(path)
            .map_err(EngineError::Unsupported);
        let dir = mount.helper_path(path).map_err(EngineError::Unsupported);
        // A listing cancelled midway keeps running in the helper until it
        // ends, at most `LIST_TIMEOUT_SECS`, or the helper is removed.
        self.spawn(async move {
            let dir = dir?;
            let (code, stdout, stderr) = run(&docker, &helper, command?, RUN_DEADLINE).await?;
            let mut entries = match code {
                0 => parse_listing(&dir, &stdout).map_err(EngineError::Unsupported)?,
                // `timeout` stopped it (TERM).
                124 | 143 => return Err(timed_out()),
                _ => return Err(EngineError::NotFound(command_error(&stderr))),
            };
            // Link targets, to tell links to folders from the rest. Only
            // hints: without them a link is read when it's followed.
            if entries.iter().any(|e| e.kind == FileKind::Symlink)
                && let Ok((0, output, _)) = run(&docker, &helper, targets?, RUN_DEADLINE).await
            {
                apply_link_targets(&dir, &output, &mut entries);
            }
            Ok(entries)
        })
    }

    pub(super) fn live_read_link(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<String>> {
        let docker = self.docker.clone();
        let helper = mount.container.clone();
        let command = mount
            .read_link_command(path)
            .map_err(EngineError::Unsupported);
        self.spawn(async move {
            match run(&docker, &helper, command?, RUN_DEADLINE).await? {
                (0, target, _) => Ok(escape_name(&target)),
                (124 | 143, _, _) => Err(timed_out()),
                (_, _, stderr) => Err(EngineError::NotFound(command_error(&stderr))),
            }
        })
    }
}

/// Where the helper sees `target`'s files, and whether links must stay
/// inside them.
const fn target_root(target: &FilesTarget) -> (&'static str, bool) {
    match target {
        FilesTarget::Container(_) => (CONTAINER_ROOT, false),
        FilesTarget::Volume(_) => (VOLUME_ROOT, true),
    }
}

/// Start the helper, report it through `ready`, and keep it until
/// `released` (its `FileMount` dropped), then remove it.
///
/// The helper's process (`cat`) reads a stdin this thread holds open; the
/// engine closes it when this connection goes, so if Berth quits or
/// crashes, the helper exits and removes itself.
fn keep_helper(
    endpoint: &Endpoint,
    target: &FilesTarget,
    ready: oneshot::Sender<EngineResult<String>>,
    released: oneshot::Receiver<()>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(EngineError::Transport(error.to_string())));
            return;
        }
    };
    let docker = {
        let _entered = runtime.enter();
        client(endpoint)
    };
    runtime.block_on(async move {
        let docker = match docker {
            Ok(docker) => docker.negotiate_version().await.map_err(mapping::error),
            Err(error) => Err(error.into()),
        };
        let started = match docker {
            Ok(docker) => start_helper(&docker, target, &ready)
                .await
                .map(|(id, stdin)| (docker, id, stdin)),
            Err(error) => Err(error),
        };
        let (docker, id, stdin) = match started {
            Ok(started) => started,
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        };
        // Nobody waits for it any more: remove it straight away.
        if ready.send(Ok(id.clone())).is_ok() {
            let _ = released.await;
        }
        drop(stdin);
        let _ = tokio::time::timeout(REMOVE_TIMEOUT, remove_helper(&docker, &id)).await;
    });
}

fn timed_out() -> EngineError {
    EngineError::Unsupported(format!(
        "This took longer than {LIST_TIMEOUT_SECS} seconds. The folder may be on an \
         unresponsive network mount."
    ))
}

/// Run `command` in helper `id`: its exit code, stdout and stderr. Gives up
/// after `deadline`; a process stuck in the helper ends with it.
async fn run(
    docker: &Docker,
    id: &str,
    command: Vec<String>,
    deadline: Duration,
) -> EngineResult<(i64, Vec<u8>, Vec<u8>)> {
    tokio::time::timeout(deadline, run_to_end(docker, id, command))
        .await
        .unwrap_or_else(|_| Err(timed_out()))
}

async fn run_to_end(
    docker: &Docker,
    id: &str,
    command: Vec<String>,
) -> EngineResult<(i64, Vec<u8>, Vec<u8>)> {
    let created = docker
        .create_exec(
            id,
            ExecConfig {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(false),
                cmd: Some(command),
                ..Default::default()
            },
        )
        .await
        .map_err(mapping::error)?;
    let StartExecResults::Attached { mut output, .. } = docker
        .start_exec(&created.id, None::<StartExecOptions>)
        .await
        .map_err(mapping::error)?
    else {
        return Err(EngineError::Transport("exec unexpectedly detached".into()));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while let Some(frame) = output.next().await {
        match frame.map_err(mapping::error)? {
            LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
            LogOutput::StdOut { message } | LogOutput::Console { message } => {
                stdout.extend_from_slice(&message);
            }
            LogOutput::StdIn { .. } => {}
        }
        if stdout.len() > MAX_OUTPUT {
            return Err(EngineError::Unsupported(
                "This folder has too many entries to list.".into(),
            ));
        }
    }
    drop(output);
    // The stream also ends if the connection drops: only an exit status
    // says the output is complete.
    let code = exit_code(docker, &created.id).await?;
    Ok((code, stdout, stderr))
}

/// `find: /proc/1/root/x/.: Not a directory` → `Not a directory`.
fn command_error(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr.lines().next().unwrap_or_default();
    line.rsplit(": ")
        .next()
        .filter(|reason| !reason.is_empty())
        .unwrap_or("Can't open this folder")
        .to_owned()
}

/// The exec's exit code once it has exited. The engine may record the exit
/// a moment after the output ends, so this waits briefly for it.
async fn exit_code(docker: &Docker, exec: &str) -> EngineResult<i64> {
    for _ in 0..20 {
        let inspected = docker.inspect_exec(exec).await.map_err(mapping::error)?;
        if let (Some(false), Some(code)) = (inspected.running, inspected.exit_code) {
            return Ok(code);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(EngineError::Transport(
        "the connection closed before the listing finished".into(),
    ))
}

/// Check the target can be browsed: a bind of a missing named volume
/// would quietly create it; a stopped container has no files to see; one
/// sharing another's process namespace isn't that namespace's process 1,
/// whose files the helper sees.
async fn check_target(docker: &Docker, target: &FilesTarget) -> EngineResult<()> {
    match target {
        FilesTarget::Volume(volume) => docker
            .inspect_volume(volume)
            .await
            .map(drop)
            .map_err(mapping::error),
        FilesTarget::Container(id) => {
            let inspected = docker
                .inspect_container(id, None::<query::InspectContainerOptions>)
                .await
                .map_err(mapping::error)?;
            if inspected.state.and_then(|s| s.running) != Some(true) {
                return Err(EngineError::Conflict("The container isn't running.".into()));
            }
            inspected
                .host_config
                .and_then(|h| h.pid_mode)
                .filter(|mode| !mode.is_empty() && mode != "private")
                .map_or(Ok(()), |mode| {
                    Err(EngineError::Unsupported(format!(
                        "This container shares a process namespace ({mode}), so its files \
                         can't be told apart from the namespace owner's."
                    )))
                })
        }
    }
}

/// How the helper sees the target's files.
fn target_config(target: &FilesTarget) -> HostConfig {
    match target {
        FilesTarget::Volume(volume) => HostConfig {
            binds: Some(vec![format!("{volume}:{VOLUME_ROOT}:ro")]),
            ..Default::default()
        },
        // In the container's process namespace its process 1 is the
        // helper's too; reading another process's root takes SYS_PTRACE.
        FilesTarget::Container(id) => HostConfig {
            pid_mode: Some(format!("container:{id}")),
            cap_add: Some(vec!["SYS_PTRACE".into()]),
            ..Default::default()
        },
    }
}

async fn pull_helper_image(docker: &Docker) -> EngineResult<()> {
    if docker.inspect_image(HELPER_IMAGE).await.is_ok() {
        return Ok(());
    }
    let (image, tag) = HELPER_IMAGE
        .rsplit_once(':')
        .unwrap_or((HELPER_IMAGE, "latest"));
    let mut pull = docker.create_image(
        Some(query::CreateImageOptions {
            from_image: Some(image.into()),
            tag: Some(tag.into()),
            ..Default::default()
        }),
        None,
        None,
    );
    while let Some(progress) = pull.next().await {
        progress.map_err(|e| match mapping::error(e) {
            EngineError::Engine { message, .. } | EngineError::NotFound(message) => {
                EngineError::Unsupported(format!("Couldn't pull {HELPER_IMAGE}: {message}"))
            }
            other => other,
        })?;
    }
    Ok(())
}

type Stdin = std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>;

/// Create and start the helper, attached to its stdin (see
/// [`keep_helper`]). Gives up before creating it if `ready`'s receiver is
/// gone: the request was dropped during the (possibly slow) pull.
async fn start_helper(
    docker: &Docker,
    target: &FilesTarget,
    ready: &oneshot::Sender<EngineResult<String>>,
) -> EngineResult<(String, Stdin)> {
    check_target(docker, target).await?;
    remove_orphans(docker).await;
    pull_helper_image(docker).await?;
    if ready.is_canceled() {
        return Err(EngineError::Transport("request dropped".into()));
    }
    // The target may have gone during the pull.
    check_target(docker, target).await?;
    let created = docker
        .create_container(
            None::<query::CreateContainerOptions>,
            ContainerCreateBody {
                image: Some(HELPER_IMAGE.into()),
                cmd: Some(vec!["cat".into()]),
                open_stdin: Some(true),
                stdin_once: Some(true),
                attach_stdin: Some(true),
                labels: Some(HashMap::from([(
                    HELPER_LABEL.to_owned(),
                    "file-browser".to_owned(),
                )])),
                network_disabled: Some(true),
                host_config: Some(HostConfig {
                    network_mode: Some("none".into()),
                    auto_remove: Some(true),
                    readonly_rootfs: Some(true),
                    // Files under a private SELinux label (a `:Z` mount)
                    // stay readable without relabelling them.
                    security_opt: Some(vec!["label=disable".into()]),
                    ..target_config(target)
                }),
                ..Default::default()
            },
        )
        .await
        .map_err(mapping::error)?;
    let id = created.id;
    // Attached before it starts, so there is no moment it runs unheld.
    let attached = async {
        let attached = docker
            .attach_container(
                &id,
                Some(query::AttachContainerOptions {
                    stdin: true,
                    stream: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(mapping::error)?;
        docker
            .start_container(&id, None::<query::StartContainerOptions>)
            .await
            .map_err(mapping::error)?;
        Ok::<_, EngineError>(attached.input)
    };
    match attached.await {
        Ok(stdin) => Ok((id, stdin)),
        Err(error) => {
            remove_helper(docker, &id).await;
            Err(error)
        }
    }
}

async fn remove_helper(docker: &Docker, id: &str) {
    let _ = docker
        .remove_container(
            id,
            Some(query::RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
}

/// Remove helpers that never started or already stopped and are older
/// than [`ORPHAN_AGE_SECS`]: left over when Berth quit or crashed while
/// mounting. Younger ones may be another request's, between its create
/// and start; running ones belong to an open Files tab and end on their own.
async fn remove_orphans(docker: &Docker) {
    let Ok(orphans) = docker
        .list_containers(Some(query::ListContainersOptions {
            all: true,
            filters: Some(HashMap::from([
                ("label".to_owned(), vec![HELPER_LABEL.to_owned()]),
                (
                    "status".to_owned(),
                    ["created", "exited", "dead"].map(str::to_owned).to_vec(),
                ),
            ])),
            ..Default::default()
        }))
        .await
    else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    for orphan in orphans {
        let old = orphan
            .created
            .is_some_and(|created| now.saturating_sub(created) > ORPHAN_AGE_SECS);
        if let Some(id) = orphan.id.filter(|_| old) {
            remove_helper(docker, &id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect_returns(body: &str) -> EngineResult<i64> {
        let mut server = mockito::Server::new();
        let _mock = server
            .mock("GET", "/exec/x/json")
            .with_header("content-type", "application/json")
            .with_body(body)
            .create();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let docker = {
            let _entered = runtime.enter();
            client(&Endpoint::Tcp {
                url: server.url(),
                tls: None,
            })
            .unwrap()
        };
        runtime.block_on(exit_code(&docker, "x"))
    }

    #[test]
    fn a_command_that_never_answers_times_out() {
        // A daemon that accepts the connection and never replies.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("tcp://{}", listener.local_addr().unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let docker = {
            let _entered = runtime.enter();
            client(&Endpoint::Tcp { url, tls: None }).unwrap()
        };
        let started = std::time::Instant::now();
        let result = runtime.block_on(run(
            &docker,
            "x",
            vec!["find".into()],
            Duration::from_millis(200),
        ));
        assert!(matches!(result, Err(EngineError::Unsupported(m)) if m.contains("longer than")));
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(listener);
    }

    #[test]
    fn output_ending_while_the_exec_runs_is_not_success() {
        assert_eq!(inspect_returns(r#"{"Running":false,"ExitCode":3}"#), Ok(3));
        assert!(matches!(
            inspect_returns(r#"{"Running":true,"ExitCode":null}"#),
            Err(EngineError::Transport(_))
        ));
        assert!(matches!(
            inspect_returns(r#"{"Running":false}"#),
            Err(EngineError::Transport(_))
        ));
    }
}

//! Browsing files through a helper container (see [`crate::files`]):
//! starting it, keeping it alive, removing it, running its commands, and
//! unpacking what it copies out.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use bollard::{
    Docker,
    container::LogOutput,
    errors::Error as BollardError,
    exec::{StartExecOptions, StartExecResults},
    models::{ContainerCreateBody, ExecConfig, HostConfig},
    query_parameters as query,
};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::{FutureExt as _, Stream, StreamExt as _};
use tar::EntryType;

use super::{BollardEngine, client, mapping};
use crate::{
    CONTAINER_ROOT, Endpoint, EngineError, EngineResult, FileEntry, FileKind, FileMount,
    FilesTarget, HELPER_IMAGE, HELPER_LABEL, LIST_TIMEOUT_SECS, Staging, VOLUME_ROOT,
    apply_link_targets, archive_path, escape_name, parse_listing,
};

/// Output larger than this fails rather than being buffered.
const MAX_OUTPUT: usize = 32 << 20;

/// A copy's error output kept, at most.
const MAX_STDERR: usize = 64 << 10;

/// Chunks of a copy in flight to its unpacking, at most.
const COPY_CHUNKS: usize = 16;

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

    pub(super) fn live_copy_out(
        &self,
        mount: &FileMount,
        path: &str,
        dest: &Path,
        copied: Arc<AtomicU64>,
    ) -> BoxFuture<'static, EngineResult<()>> {
        let docker = self.docker.clone();
        let helper = mount.container.clone();
        let command = mount.copy_command(path).map_err(EngineError::Unsupported);
        let check = mount.check_command(path).map_err(EngineError::Unsupported);
        let (path, dest) = (path.to_owned(), dest.to_owned());
        self.spawn(async move {
            let command = command?;
            if let Some(check) = check? {
                match run(&docker, &helper, check, RUN_DEADLINE).await? {
                    (0, output, _) => {
                        parse_listing("/", &output).map_err(EngineError::Unsupported)?;
                    }
                    (124 | 143, _, _) => return Err(timed_out()),
                    (_, _, stderr) => return Err(EngineError::NotFound(command_error(&stderr))),
                }
            }
            let staging = Staging::new(&dest).map_err(|e| EngineError::writing(&e))?;
            // Unpacked as it arrives, on a thread of its own. The staging
            // goes with it, so a cancelled copy is removed only once that
            // thread is done writing it.
            let (sender, chunks) = tokio::sync::mpsc::channel(COPY_CHUNKS);
            let mut unpacking = tokio::task::spawn_blocking(move || {
                unpack(Chunks::new(chunks), &staging.path(), &path, &copied).map(|()| staging)
            });
            let (exec, mut output) = start(&docker, &helper, command).await?;
            let mut stderr = Vec::new();
            let mut unpacked = None;
            // A hung network mount stalls the copy; a long one is fine.
            while let Some(frame) = tokio::time::timeout(RUN_DEADLINE, output.next())
                .await
                .map_err(|_| stalled())?
            {
                match frame.map_err(mapping::error)? {
                    LogOutput::StdErr { message } => {
                        if stderr.len() < MAX_STDERR {
                            stderr.extend_from_slice(&message);
                        }
                    }
                    LogOutput::StdOut { message } | LogOutput::Console { message } => {
                        if unpacked.is_none() && sender.send(message.into()).await.is_err() {
                            // The unpacking stopped before the output did:
                            // at the archive's end, or failing the copy.
                            let staging = joined(&mut unpacking)
                                .await?
                                .map_err(|e| EngineError::writing(&e))?;
                            unpacked = Some(staging);
                        }
                    }
                    LogOutput::StdIn { .. } => {}
                }
            }
            drop((output, sender));
            let unpacked = match unpacked {
                Some(staging) => Ok(staging),
                None => joined(&mut unpacking).await?,
            };
            // Only an exit status says the archive is complete; a failed
            // `tar` explains a broken archive best.
            let failed = exit_code(&docker, &exec).await? != 0;
            // Moving into place and removing what was there can take a
            // while, which isn't the engine's workers' to spend.
            let mut finished = tokio::task::spawn_blocking(move || {
                if failed {
                    drop(unpacked);
                    return Ok(false);
                }
                unpacked.and_then(Staging::finish).map(|()| true)
            });
            match joined(&mut finished).await? {
                Ok(true) => Ok(()),
                Ok(false) => Err(EngineError::NotFound(tar_error(&stderr))),
                Err(error) => Err(EngineError::writing(&error)),
            }
        })
    }
}

async fn joined<T>(task: &mut tokio::task::JoinHandle<T>) -> EngineResult<T> {
    task.await
        .map_err(|error| EngineError::Transport(error.to_string()))
}

fn stalled() -> EngineError {
    EngineError::Unsupported(format!(
        "Nothing arrived for {LIST_TIMEOUT_SECS} seconds. The file may be on an unresponsive \
         network mount."
    ))
}

/// `tar: can't open './x': Permission denied` → `Can't open './x':
/// Permission denied`.
fn tar_error(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let line = stderr.lines().next().unwrap_or_default();
    let line = line.strip_prefix("tar: ").unwrap_or(line);
    let mut chars = line.chars();
    chars.next().map_or_else(
        || "Couldn't copy this".to_owned(),
        |first| first.to_uppercase().chain(chars).collect(),
    )
}

/// The archive as it arrives, read on the unpacking thread.
struct Chunks {
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    read: usize,
}

impl Chunks {
    const fn new(receiver: tokio::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            chunk: Vec::new(),
            read: 0,
        }
    }
}

impl Read for Chunks {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut rest = self.chunk.get(self.read..).unwrap_or_default();
            if !rest.is_empty() {
                let read = rest.read(buf)?;
                self.read = self.read.saturating_add(read);
                return Ok(read);
            }
            // The sender gone, the archive ends (cut short if cancelled).
            match self.receiver.blocking_recv() {
                Some(chunk) => (self.chunk, self.read) = (chunk, 0),
                None => return Ok(0),
            }
        }
    }
}

/// Unpack [`FileMount::copy_command`]'s archive of real path `path` into
/// `to`, adding the bytes of file data written to `copied`.
fn unpack(archive: impl Read, to: &Path, path: &str, copied: &AtomicU64) -> io::Result<()> {
    let mut archive = tar::Archive::new(Ended::from(archive));
    let mut made = HashSet::new();
    // Folder permissions, set once their entries are written.
    #[cfg(unix)]
    let mut folders = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path_bytes().into_owned();
        let rel = relative(path, &name)?;
        let kind = entry.header().entry_type();
        // The copied entry itself was a file or folder when resolved; a
        // link now would be followed where the copy is opened.
        if rel.as_os_str().is_empty()
            && !matches!(
                kind,
                EntryType::Regular | EntryType::Continuous | EntryType::Directory
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "This changed since it was listed. Reload to see it.",
            ));
        }
        let target = place(to, &rel, &mut made)?;
        let written = match kind {
            EntryType::Directory => {
                if !target.is_dir() {
                    fs::create_dir(&target)?;
                }
                #[cfg(unix)]
                if let Ok(mode) = entry.header().mode() {
                    folders.push((target.clone(), mode));
                }
                true
            }
            EntryType::Regular | EntryType::Continuous => {
                write_file(&mut entry, &target, copied)?;
                true
            }
            #[cfg(unix)]
            EntryType::Symlink => {
                let link = entry.link_name()?.ok_or_else(|| unexpected(&name))?;
                std::os::unix::fs::symlink(link, &target)?;
                true
            }
            EntryType::Link => {
                let source = entry.link_name_bytes().ok_or_else(|| unexpected(&name))?;
                let source = to.join(relative(path, &source)?);
                // Only to what was copied already, never followed: a file
                // is linked, a symlink made again.
                let copied = fs::symlink_metadata(&source)
                    .ok()
                    .filter(|_| made.contains(&source));
                match copied {
                    Some(meta) if meta.is_file() => {
                        fs::hard_link(&source, &target)
                            .or_else(|_| fs::copy(&source, &target).map(drop))?;
                        true
                    }
                    #[cfg(unix)]
                    Some(meta) if meta.is_symlink() => {
                        std::os::unix::fs::symlink(fs::read_link(&source)?, &target)?;
                        true
                    }
                    // To what was left out.
                    _ => false,
                }
            }
            // Devices, pipes and sockets (and links, on Windows).
            _ => false,
        };
        // Only what is written may be found again (or linked to): a name
        // left out may be another's, by case.
        if written {
            made.insert(target);
        }
    }
    // Input ending before the end-of-archive blocks is a copy cut short,
    // which the archive reader takes for its end.
    if archive.into_inner().ended {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the copy ended early",
        ));
    }
    // Deepest first, kept open to the user, as files are.
    #[cfg(unix)]
    for (folder, mode) in folders.into_iter().rev() {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(folder, fs::Permissions::from_mode((mode & 0o777) | 0o700))?;
    }
    Ok(())
}

/// A reader telling whether it was read to its end.
struct Ended<R> {
    inner: R,
    ended: bool,
}

impl<R> From<R> for Ended<R> {
    fn from(inner: R) -> Self {
        Self {
            inner,
            ended: false,
        }
    }
}

impl<R: Read> Read for Ended<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read == 0 && !buf.is_empty() {
            self.ended = true;
        }
        Ok(read)
    }
}

fn relative(path: &str, name: &[u8]) -> io::Result<PathBuf> {
    archive_path(path, name).ok_or_else(|| unexpected(name))
}

fn unexpected(name: &[u8]) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{} can't be copied here: its name leads out of the copy or isn't allowed",
            String::from_utf8_lossy(name)
        ),
    )
}

/// `to` joined with `rel`, the folders on the way created as needed.
///
/// Everything under `to` is made here, and recorded in `made` once
/// written, so finding anything else on the way (a link, which could lead
/// out of `to`) or at the end is a clash: two names the file system doesn't
/// tell apart, e.g. by case. What an earlier entry of the same name made is
/// replaced, unless a folder.
fn place(to: &Path, rel: &Path, made: &mut HashSet<PathBuf>) -> io::Result<PathBuf> {
    let mut parts = rel.iter();
    let last = parts.next_back();
    let mut at = to.to_owned();
    if last.is_some() {
        folder(to, &at, made)?;
    }
    for part in parts {
        at.push(part);
        folder(to, &at, made)?;
    }
    if let Some(last) = last {
        at.push(last);
    }
    match fs::symlink_metadata(&at) {
        Ok(_) if !made.contains(&at) => return Err(clash(to, &at)),
        Ok(meta) if !meta.is_dir() => fs::remove_file(&at)?,
        _ => {}
    }
    Ok(at)
}

fn folder(to: &Path, at: &Path, made: &mut HashSet<PathBuf>) -> io::Result<()> {
    match fs::symlink_metadata(at) {
        Ok(meta) if meta.is_dir() && made.contains(at) => Ok(()),
        Ok(_) => Err(clash(to, at)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(at)?;
            made.insert(at.to_owned());
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn clash(to: &Path, at: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "{} clashes with another name in the copy, e.g. one differing only in case",
            at.strip_prefix(to).unwrap_or(at).display()
        ),
    )
}

/// Write a file entry to `target`, with its modification time and (on
/// Unix) permissions, kept readable and writable by the user.
fn write_file<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    target: &Path,
    copied: &AtomicU64,
) -> io::Result<()> {
    let mut file = fs::File::create(target)?;
    io::copy(
        entry,
        &mut Counted {
            inner: &mut file,
            copied,
        },
    )?;
    let header = entry.header();
    if let Some(time) = header
        .mtime()
        .ok()
        .and_then(|secs| UNIX_EPOCH.checked_add(Duration::from_secs(secs)))
    {
        let _ = file.set_modified(time);
    }
    #[cfg(unix)]
    if let Ok(mode) = header.mode() {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode((mode & 0o777) | 0o600))?;
    }
    Ok(())
}

/// A writer counting what it writes into `copied`.
struct Counted<'a, W> {
    inner: W,
    copied: &'a AtomicU64,
}

impl<W: Write> Write for Counted<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.copied
            .fetch_add(u64::try_from(written).unwrap_or(0), Ordering::Relaxed);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
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

type Output = Pin<Box<dyn Stream<Item = Result<LogOutput, BollardError>> + Send>>;

/// Start `command` in helper `id`: the exec's ID and its output.
async fn start(docker: &Docker, id: &str, command: Vec<String>) -> EngineResult<(String, Output)> {
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
    let StartExecResults::Attached { output, .. } = docker
        .start_exec(&created.id, None::<StartExecOptions>)
        .await
        .map_err(mapping::error)?
    else {
        return Err(EngineError::Transport("exec unexpectedly detached".into()));
    };
    Ok((created.id, output))
}

async fn run_to_end(
    docker: &Docker,
    id: &str,
    command: Vec<String>,
) -> EngineResult<(i64, Vec<u8>, Vec<u8>)> {
    let (exec, mut output) = start(docker, id, command).await?;
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
    let code = exit_code(docker, &exec).await?;
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

    /// An archive of `entries` (name, type, data or link target), cut
    /// before its end-of-archive blocks unless `end`.
    fn archive(entries: &[(&str, EntryType, &str)], end: bool) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for &(name, kind, data) in entries {
            let mut header = tar::Header::new_ustar();
            header.set_entry_type(kind);
            header.set_mode(if kind == EntryType::Directory {
                0o700
            } else {
                0o644
            });
            let body = match kind {
                EntryType::Regular => data.as_bytes(),
                EntryType::Symlink | EntryType::Link => {
                    header.set_link_name(data).unwrap();
                    b""
                }
                _ => b"",
            };
            header.set_size(u64::try_from(body.len()).unwrap());
            builder.append_data(&mut header, name, body).unwrap();
        }
        let mut bytes = builder.into_inner().unwrap();
        if !end {
            bytes.truncate(bytes.len().saturating_sub(1024));
        }
        bytes
    }

    fn unpacked(
        entries: &[(&str, EntryType, &str)],
        end: bool,
    ) -> (tempfile::TempDir, io::Result<()>) {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("copy");
        let result = unpack(&archive(entries, end)[..], &to, "/d", &AtomicU64::new(0));
        (dir, result)
    }

    #[test]
    fn folders_unpack_inside_their_copy() {
        let (dir, result) = unpacked(
            &[
                ("./d/", EntryType::Directory, ""),
                ("./d/a", EntryType::Regular, "hi"),
                ("./d/b", EntryType::Link, "./d/a"),
                ("./d/a", EntryType::Regular, "again"),
                ("./d/s", EntryType::Symlink, "/etc/hostname"),
                ("./d/t", EntryType::Link, "./d/s"),
                ("./d/p", EntryType::Fifo, ""),
                ("./d/q", EntryType::Link, "./d/p"),
            ],
            true,
        );
        result.unwrap();
        let copy = dir.path().join("copy");
        assert_eq!(fs::read_to_string(copy.join("a")).unwrap(), "again");
        assert_eq!(fs::read_to_string(copy.join("b")).unwrap(), "hi");
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(copy.join("t")).unwrap(),
            Path::new("/etc/hostname")
        );
        // What was left out isn't linked to either.
        assert!(fs::symlink_metadata(copy.join("q")).is_err());
        // Folders keep their permissions, as files do.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&copy).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        // Cut short at an entry's end, it isn't complete.
        let (_dir, result) = unpacked(
            &[
                ("./d/", EntryType::Directory, ""),
                ("./d/a", EntryType::Regular, "hi"),
            ],
            false,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn unpacking_never_writes_through_a_link() {
        let (dir, result) = unpacked(
            &[
                ("./d/", EntryType::Directory, ""),
                ("./d/out", EntryType::Symlink, "/tmp"),
                ("./d/out/x", EntryType::Regular, "escaped"),
            ],
            true,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(fs::symlink_metadata(dir.path().join("copy/out")).is_ok());
        // Nor is the copied entry itself a link now.
        let (_dir, result) = unpacked(&[("./d", EntryType::Symlink, "/etc/passwd")], true);
        assert!(result.unwrap_err().to_string().contains("changed"));
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

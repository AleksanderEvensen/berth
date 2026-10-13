#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;

use super::*;
use berth_docker::{FileEntry, FileKind, FileMount, FilesTarget, Folder, Resolution, resolve_path};

/// Lists `path` of the browsed tree the way the app does: symlinks are
/// resolved against its root first, listing folders on the way as needed.
fn list(engine: &BollardEngine, mount: &FileMount, path: &str) -> Result<Vec<FileEntry>, String> {
    let mut known: HashMap<String, Folder> = HashMap::new();
    loop {
        let step = resolve_path(path, mount.confined, |dir| known.get(dir));
        let dir = match step {
            Resolution::Real(dir) => dir,
            Resolution::Needs(dir) => {
                let entries = fetch(engine, mount, &dir)?;
                known.insert(dir, Folder::new(entries));
                continue;
            }
            Resolution::NeedsLink(link) => {
                let target = block_on(engine.read_link(mount, &link)).map_err(|e| e.to_string())?;
                let (parent, name) = link.rsplit_once('/').unwrap();
                let parent = if parent.is_empty() { "/" } else { parent };
                let entry = known
                    .get_mut(parent)
                    .and_then(|folder| folder.get_mut(name))
                    .unwrap();
                entry.link_target = Some(target);
                continue;
            }
            Resolution::Failed(why) => return Err(why),
        };
        return fetch(engine, mount, &dir);
    }
}

fn fetch(engine: &BollardEngine, mount: &FileMount, dir: &str) -> Result<Vec<FileEntry>, String> {
    block_on(engine.list_files(mount, dir)).map_err(|e| e.to_string())
}

fn names(entries: &[FileEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
#[ignore = "requires Docker; creates throwaway alpine and pause containers and a volume, may pull busybox"]
fn live_browsing_containers_and_volumes() {
    std::sync::LazyLock::force(&STARTED);
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
    let id = create_test_container(&runtime, &docker, &name).unwrap();
    assert!(matches!(
        block_on(engine.open_files(&FilesTarget::Container(id.clone()))),
        Err(EngineError::Conflict(_))
    ));
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    a_shared_pid_namespace_is_refused(&engine, &runtime, &docker, &name, &id);
    browse_a_container(&engine, &runtime, &docker, &id);
    browse_a_container_without_a_shell(&engine, &runtime, &docker, &name);
    browse_a_volume(&endpoint, &runtime, &docker, &name);
}

fn browse_a_container(engine: &BollardEngine, runtime: &Runtime, docker: &Docker, id: &str) {
    exec_sh(
        runtime,
        docker,
        id,
        "mkdir -p /t/real/in \"/t/a$(printf '\\200')\" && cd /t && ln -s /t/real abs && \
         ln -s real rel && ln -s /etc/hosts file && touch \"nl\nname\" && \
         mkdir \"nldir\n\" && touch \"nldir\n/inner\"",
    );
    let mount = block_on(engine.open_files(&FilesTarget::Container(id.to_owned()))).unwrap();
    let root = list(engine, &mount, "/").unwrap();
    let etc = root.iter().find(|e| e.name == "etc").expect("/etc listed");
    assert_eq!(etc.kind, FileKind::Directory);
    assert!(etc.modified.is_some());
    let hosts = list(engine, &mount, "/etc").unwrap();
    let hosts = hosts
        .iter()
        .find(|e| e.name == "hosts")
        .expect("/etc/hosts");
    assert!(hosts.size.is_some_and(|s| s > 0));

    let t = list(engine, &mount, "/t").unwrap();
    assert_eq!(
        names(&t),
        [
            "a\\200", "abs", "file", "nldir\n", "real", "rel", "nl\nname"
        ]
    );
    // Link targets come with the listing, to tell links to files apart.
    let abs = t.iter().find(|e| e.name == "abs").unwrap();
    assert_eq!(abs.link_target.as_deref(), Some("/t/real"));
    let bin = list(engine, &mount, "/bin").unwrap();
    let arch = bin
        .iter()
        .find(|e| e.name == "arch")
        .expect("busybox applet");
    assert_eq!(arch.link_target.as_deref(), Some("/bin/busybox"));
    // Line breaks in a folder's own path are fine too.
    assert_eq!(
        names(&list(engine, &mount, "/t/nldir\n").unwrap()),
        ["inner"]
    );
    assert_eq!(
        block_on(engine.read_link(&mount, "/t/abs")).unwrap(),
        "/t/real"
    );
    // procfs links report no size; `/proc` lists all the same.
    assert!(
        list(engine, &mount, "/proc")
            .unwrap()
            .iter()
            .any(|e| e.name == "self")
    );
    // Absolute and relative links lead into the container, not the helper.
    assert_eq!(names(&list(engine, &mount, "/t/abs").unwrap()), ["in"]);
    assert_eq!(names(&list(engine, &mount, "/t/rel").unwrap()), ["in"]);
    assert!(
        list(engine, &mount, "/t/file")
            .unwrap_err()
            .contains("isn't a folder")
    );
    assert!(
        list(engine, &mount, "/t/a\\200")
            .unwrap_err()
            .contains("UTF-8")
    );
    // The helper's own listing errors are reported.
    assert!(matches!(
        block_on(engine.list_files(&mount, "/etc/hosts")),
        Err(EngineError::NotFound(_))
    ));
    // A folder replaced by a link since it was resolved isn't followed
    // inside the helper.
    assert_eq!(names(&list(engine, &mount, "/t/real").unwrap()), ["in"]);
    exec_sh(
        runtime,
        docker,
        id,
        "mv /t/real /t/moved && ln -s /etc /t/real",
    );
    let error = block_on(engine.list_files(&mount, "/t/real")).unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
    drop(mount);
    wait_for_no_helpers(runtime, docker);
}

/// A container in another's process namespace isn't its process 1.
fn a_shared_pid_namespace_is_refused(
    engine: &BollardEngine,
    runtime: &Runtime,
    docker: &Docker,
    name: &str,
    owner: &str,
) {
    let name = format!("{name}-shared");
    let _cleanup = Cleanup {
        runtime,
        docker,
        name: name.clone(),
    };
    let id = runtime
        .block_on(docker.create_container(
            Some(query::CreateContainerOptions {
                name: Some(name),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some("alpine:latest".into()),
                cmd: Some(vec!["sleep".into(), "300".into()]),
                host_config: Some(bollard::models::HostConfig {
                    pid_mode: Some(format!("container:{owner}")),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ))
        .unwrap()
        .id;
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    assert!(matches!(
        block_on(engine.open_files(&FilesTarget::Container(id))),
        Err(EngineError::Unsupported(_))
    ));
}

/// No shell, no tools, a non-root user: browsable all the same.
fn browse_a_container_without_a_shell(
    engine: &BollardEngine,
    runtime: &Runtime,
    docker: &Docker,
    name: &str,
) {
    const PAUSE: &str = "registry.k8s.io/pause:3.9";
    runtime
        .block_on(async {
            let mut pull = docker.create_image(
                Some(query::CreateImageOptions {
                    from_image: Some(PAUSE.into()),
                    ..Default::default()
                }),
                None,
                None,
            );
            while let Some(progress) = pull.next().await {
                progress?;
            }
            Ok::<_, bollard::errors::Error>(())
        })
        .unwrap();
    let name = format!("{name}-pause");
    let _cleanup = Cleanup {
        runtime,
        docker,
        name: name.clone(),
    };
    let id = runtime
        .block_on(docker.create_container(
            Some(query::CreateContainerOptions {
                name: Some(name),
                ..Default::default()
            }),
            ContainerCreateBody {
                image: Some(PAUSE.into()),
                user: Some("65532".into()),
                ..Default::default()
            },
        ))
        .unwrap()
        .id;
    block_on(engine.container_action(&id, ContainerAction::Start)).unwrap();
    let mount = block_on(engine.open_files(&FilesTarget::Container(id))).unwrap();
    let root = list(engine, &mount, "/").unwrap();
    assert!(
        root.iter()
            .any(|e| e.name == "pause" && e.kind == FileKind::File)
    );
    drop(mount);
    wait_for_no_helpers(runtime, docker);
}

fn browse_a_volume(endpoint: &Endpoint, runtime: &Runtime, docker: &Docker, name: &str) {
    let volume = name.replace("berth-live-test-", "berth-live-files-");
    let _volume_cleanup = VolumeCleanup {
        runtime,
        docker,
        name: volume.clone(),
    };
    runtime
        .block_on(docker.create_volume(VolumeCreateOptions {
            name: Some(volume.clone()),
            ..Default::default()
        }))
        .unwrap();
    let engine = BollardEngine::connect(endpoint).unwrap();
    // A missing volume is reported, not created by the helper's bind.
    let missing = format!("{volume}-missing");
    assert!(matches!(
        block_on(engine.open_files(&FilesTarget::Volume(missing.clone()))),
        Err(EngineError::NotFound(_))
    ));
    assert!(runtime.block_on(docker.inspect_volume(&missing)).is_err());
    let mount = block_on(engine.open_files(&FilesTarget::Volume(volume.clone()))).unwrap();
    assert!(mount.confined);
    let helper = block_on(engine.inspect_container(&mount.container)).unwrap();
    assert!(helper.is_helper());
    assert_eq!(list(&engine, &mount, "/").unwrap(), []);

    // Dropping the engine first must not strand the helper.
    let helper_id = mount.container.clone();
    drop(engine);
    drop(mount);
    wait_for_no_helpers(runtime, docker);

    // Nor may dropping a request before it completes.
    let engine = BollardEngine::connect(endpoint).unwrap();
    drop(engine.open_files(&FilesTarget::Volume(volume)));
    wait_for_no_helpers(runtime, docker);
    assert!(
        runtime
            .block_on(docker.inspect_container(&helper_id, None::<query::InspectContainerOptions>))
            .is_err()
    );
}

/// Run by [`helpers_outlive_no_crashed_berth`] in a child process: open a
/// helper, then exit at once, with no cleanup, as a crash would.
#[test]
#[ignore = "child process of helpers_outlive_no_crashed_berth"]
#[expect(clippy::exit, reason = "exiting without cleanup is what a crash does")]
fn crashing_helper_owner() {
    let Ok(volume) = std::env::var("BERTH_CRASH_VOLUME") else {
        return;
    };
    let engine = BollardEngine::connect(&endpoint().unwrap()).unwrap();
    let mount = block_on(engine.open_files(&FilesTarget::Volume(volume))).unwrap();
    println!("helper {}", mount.container);
    std::process::exit(0);
}

#[test]
#[ignore = "requires Docker; creates a throwaway volume, may pull busybox"]
fn helpers_outlive_no_crashed_berth() {
    std::sync::LazyLock::force(&STARTED);
    let endpoint = endpoint().unwrap();
    let (runtime, docker) = test_client(&endpoint).unwrap();
    let volume = test_container_name()
        .unwrap()
        .replace("berth-live-test-", "berth-live-crash-");
    let _volume_cleanup = VolumeCleanup {
        runtime: &runtime,
        docker: &docker,
        name: volume.clone(),
    };
    runtime
        .block_on(docker.create_volume(VolumeCreateOptions {
            name: Some(volume.clone()),
            ..Default::default()
        }))
        .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "files::crashing_helper_owner",
            "--nocapture",
        ])
        .env("BERTH_CRASH_VOLUME", &volume)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(stdout.contains("helper "), "{stdout}");
    // Its connection gone, the helper's stdin closes, so it exits and
    // removes itself.
    wait_for_no_helpers(&runtime, &docker);
}

/// Run `script` in container `id` and return its output.
fn exec_sh(runtime: &Runtime, docker: &Docker, id: &str, script: &str) -> String {
    runtime.block_on(async {
        let exec = docker
            .create_exec(
                id,
                bollard::exec::CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", script]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut out = String::new();
        if let bollard::exec::StartExecResults::Attached { mut output, .. } =
            docker.start_exec(&exec.id, None).await.unwrap()
        {
            while let Some(Ok(frame)) = output.next().await {
                out.push_str(&String::from_utf8_lossy(&frame.into_bytes()));
            }
        }
        out
    })
}

/// When these tests started (unix seconds): helpers older than that are a
/// running Berth's, not theirs.
static STARTED: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
});

/// Waits until no helper containers the tests started exist.
fn wait_for_no_helpers(runtime: &Runtime, docker: &Docker) {
    let deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(20))
        .unwrap();
    // Give a just-dropped request time to create its helper first.
    std::thread::sleep(Duration::from_millis(1500));
    loop {
        let helpers = runtime
            .block_on(docker.list_containers(Some(query::ListContainersOptions {
                all: true,
                filters: Some(std::collections::HashMap::from([(
                    "label".to_owned(),
                    vec![berth_docker::HELPER_LABEL.to_owned()],
                )])),
                ..Default::default()
            })))
            .unwrap();
        let helpers: Vec<_> = helpers
            .into_iter()
            .filter(|h| h.created.is_some_and(|c| c >= *STARTED))
            .collect();
        if helpers.is_empty() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "helpers left: {helpers:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

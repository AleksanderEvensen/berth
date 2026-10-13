//! A small, fixed filesystem for every running fixture container and
//! volume, seen through helpers that appear in the container list like the
//! live ones.

use futures::future::BoxFuture;

use super::{FixtureEngine, FixtureState};
use crate::{
    CONTAINER_ROOT, Container, ContainerState, EngineError, EngineResult, EventAction, FileEntry,
    FileKind, FileMount, FilesTarget, HELPER_IMAGE, HELPER_LABEL, Mount, MountKind, ObjectKind,
    VOLUME_ROOT, sort_entries,
};

const MODIFIED: i64 = 1_791_000_000;

fn dir(name: &str) -> FileEntry {
    FileEntry {
        name: name.into(),
        kind: FileKind::Directory,
        link_target: None,
        size: Some(4096),
        modified: Some(MODIFIED),
    }
}

fn file(name: &str, size: u64) -> FileEntry {
    FileEntry {
        name: name.into(),
        kind: FileKind::File,
        link_target: None,
        size: Some(size),
        modified: Some(MODIFIED.saturating_sub(86_400)),
    }
}

fn link(name: &str, target: &str) -> FileEntry {
    FileEntry {
        name: name.into(),
        kind: FileKind::Symlink,
        link_target: Some(target.into()),
        size: u64::try_from(target.len()).ok(),
        modified: Some(MODIFIED),
    }
}

/// A container's tree: `None` for directories that can't be read.
fn container_listing(path: &str) -> Option<Vec<FileEntry>> {
    Some(match path {
        "/" => vec![
            file(".dockerenv", 0),
            link("bin", "usr/bin"),
            dir("data"),
            dir("dev"),
            file("entrypoint.sh", 12_288),
            dir("etc"),
            dir("home"),
            dir("lib"),
            link("lib64", "/usr/lib"),
            dir("proc"),
            dir("root"),
            dir("tmp"),
            dir("usr"),
            dir("var"),
        ],
        "/etc" => vec![
            file("hostname", 13),
            file("hosts", 174),
            dir("nginx"),
            file("os-release", 393),
            file("passwd", 1_172),
            link("localtime", "/usr/share/zoneinfo/UTC"),
        ],
        "/etc/nginx" => vec![
            dir("conf.d"),
            file("mime.types", 5_349),
            file("nginx.conf", 2_656),
        ],
        "/etc/nginx/conf.d" => vec![file("default.conf", 1_093)],
        "/data" => volume_listing("/"),
        "/data/base" => volume_listing("/base"),
        "/usr" => vec![dir("bin"), dir("lib"), dir("local"), dir("share")],
        // Long enough to scroll, and links to a file next to them, as with
        // busybox.
        "/usr/bin" => (0..400)
            .map(|n| file(&format!("tool-{n:03}"), 40_960))
            .chain([file("busybox", 1_032_000), link("arch", "/usr/bin/busybox")])
            .collect(),
        "/root" => return None,
        _ => Vec::new(),
    })
}

/// Every volume's tree.
fn volume_listing(path: &str) -> Vec<FileEntry> {
    match path {
        "/" => vec![
            dir("base"),
            dir("global"),
            dir("pg_wal"),
            file("PG_VERSION", 3),
            file("postgresql.conf", 30_721),
            link("current", "base/1"),
            link("escape", "/etc"),
        ],
        "/base" => vec![dir("1"), dir("4"), dir("5")],
        _ => Vec::new(),
    }
}

fn helper(id: &str, target: &FilesTarget) -> Container {
    let mounts = match target {
        FilesTarget::Volume(volume) => vec![Mount {
            kind: MountKind::Volume,
            source: volume.clone(),
            destination: VOLUME_ROOT.into(),
            read_only: true,
        }],
        FilesTarget::Container(_) => Vec::new(),
    };
    Container {
        id: id.into(),
        name: id.into(),
        image: HELPER_IMAGE.into(),
        state: ContainerState::Running,
        status: "Up Less than a second".into(),
        ip: None,
        ports: Vec::new(),
        mounts,
        labels: vec![(HELPER_LABEL.into(), "file-browser".into())],
        compose: None,
        created: MODIFIED,
        command: None,
        env: None,
    }
}

/// Check `target` can be browsed; `Ok` with the helper's root.
fn check(state: &FixtureState, target: &FilesTarget) -> EngineResult<(&'static str, bool)> {
    match target {
        FilesTarget::Volume(volume) => state
            .volumes
            .iter()
            .any(|v| &v.name == volume)
            .then_some((VOLUME_ROOT, true))
            .ok_or_else(|| EngineError::NotFound(volume.clone())),
        FilesTarget::Container(id) => {
            let c = state
                .containers
                .iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| EngineError::NotFound(id.clone()))?;
            if c.state == ContainerState::Running {
                Ok((CONTAINER_ROOT, false))
            } else {
                Err(EngineError::Conflict("The container isn't running.".into()))
            }
        }
    }
}

impl FixtureEngine {
    pub(super) fn fixture_open_files(
        &self,
        target: &FilesTarget,
    ) -> BoxFuture<'static, EngineResult<FileMount>> {
        let target = target.clone();
        let state = self.state.clone();
        self.mutate(move |s| {
            let (root, confined) = check(s, &target)?;
            s.helpers = s.helpers.saturating_add(1);
            let id = format!("helper-{}", s.helpers);
            s.containers.push(helper(&id, &target));
            s.emit(ObjectKind::Container, &id, EventAction::Create);
            s.emit(ObjectKind::Container, &id, EventAction::Start);
            let helper = id.clone();
            Ok(FileMount::new(id, root.into(), confined, move || {
                let mut s = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                s.containers.retain(|c| c.id != helper);
                s.emit(ObjectKind::Container, &helper, EventAction::Destroy);
            }))
        })
    }

    pub(super) fn fixture_list_files(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<Vec<FileEntry>>> {
        let (helper, confined, path) = (mount.container.clone(), mount.confined, path.to_owned());
        self.mutate(move |s| {
            helper_exists(s, &helper)?;
            let mut entries = listing(confined, &path)
                .ok_or_else(|| EngineError::NotFound("Permission denied".into()))?;
            sort_entries(&mut entries);
            Ok(entries)
        })
    }

    pub(super) fn fixture_read_link(
        &self,
        mount: &FileMount,
        path: &str,
    ) -> BoxFuture<'static, EngineResult<String>> {
        let (helper, confined, path) = (mount.container.clone(), mount.confined, path.to_owned());
        self.mutate(move |s| {
            helper_exists(s, &helper)?;
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
            let parent = if parent.is_empty() { "/" } else { parent };
            listing(confined, parent)
                .unwrap_or_default()
                .into_iter()
                .find(|e| e.name == name)
                .and_then(|e| e.link_target)
                .ok_or_else(|| EngineError::NotFound("Invalid argument".into()))
        })
    }
}

/// The tree a helper sees: a volume's, or a container's.
fn listing(confined: bool, path: &str) -> Option<Vec<FileEntry>> {
    if confined {
        Some(volume_listing(path))
    } else {
        container_listing(path)
    }
}

fn helper_exists(state: &FixtureState, id: &str) -> EngineResult<()> {
    state
        .containers
        .iter()
        .any(|c| c.id == id && c.is_helper())
        .then_some(())
        .ok_or_else(|| EngineError::NotFound(id.to_owned()))
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::{Engine as _, Folder, Resolution, fixtures::FixtureEngine, resolve_path};

    fn running(engine: &FixtureEngine) -> Container {
        engine
            .with_state(|s| {
                s.containers
                    .iter()
                    .find(|c| c.state == ContainerState::Running)
                    .cloned()
            })
            .unwrap()
    }

    #[test]
    fn containers_are_browsed_through_a_helper() {
        let engine = FixtureEngine::default();
        let target = FilesTarget::Container(running(&engine).id);
        let mount = block_on(engine.open_files(&target)).unwrap();
        assert!(!mount.confined);
        let root = block_on(engine.list_files(&mount, "/")).unwrap();
        assert_eq!(root.first().map(|e| e.name.as_str()), Some("bin"));
        let root = Folder::new(root);
        let lookup = |dir: &str| (dir == "/").then_some(&root);
        assert_eq!(
            resolve_path("/bin", false, lookup),
            Resolution::Needs("/usr".into())
        );
        assert_eq!(
            block_on(engine.read_link(&mount, "/bin")).unwrap(),
            "usr/bin"
        );
        assert!(matches!(
            block_on(engine.list_files(&mount, "/root")),
            Err(EngineError::NotFound(_))
        ));
        let id = mount.container.clone();
        drop(mount);
        assert!(
            !block_on(engine.list_containers())
                .unwrap()
                .iter()
                .any(|c| c.id == id)
        );
    }

    #[test]
    fn stopped_containers_and_missing_volumes_cannot_be_browsed() {
        let engine = FixtureEngine::default();
        let stopped = engine
            .with_state(|s| s.containers.iter().find(|c| !c.state.is_active()).cloned())
            .unwrap();
        assert!(matches!(
            block_on(engine.open_files(&FilesTarget::Container(stopped.id))),
            Err(EngineError::Conflict(_))
        ));
        assert!(matches!(
            block_on(engine.open_files(&FilesTarget::Volume("missing".into()))),
            Err(EngineError::NotFound(_))
        ));
    }

    #[test]
    fn volume_helpers_come_and_go() {
        let engine = FixtureEngine::default();
        let volume = engine.with_state(|s| s.volumes[0].name.clone());
        let mount = block_on(engine.open_files(&FilesTarget::Volume(volume))).unwrap();
        assert!(mount.confined);
        let listed = block_on(engine.list_containers()).unwrap();
        assert!(
            listed
                .iter()
                .any(|c| c.id == mount.container && c.is_helper())
        );
        let files = block_on(engine.list_files(&mount, "/")).unwrap();
        assert!(files.iter().any(|e| e.name == "PG_VERSION"));
        let id = mount.container.clone();
        drop(mount);
        let listed = block_on(engine.list_containers()).unwrap();
        assert!(!listed.iter().any(|c| c.id == id));
    }
}

//! The Files tabs of containers (B9) and volumes (C5).
//!
//! A [`FilesTab`] starts a helper container that sees the target's files
//! (see [`berth_docker::files`]), once the user has allowed helpers, and
//! shows them in a [`FileBrowser`]. The browser owns the helper: dropping
//! it (leaving the tab, the selection or the screen) removes the helper.
//!
//! The browser keeps the listings and the expanded folders, loading each
//! folder when it is first opened, and hands the flattened tree to the
//! stateless [`FileTree`]. Paths are the browsed tree's own, from `/`, and
//! escaped ([`berth_docker::escape_name`]); symlinks are resolved against
//! that root before a folder is listed ([`resolve_path`]).
//!
//! A file is opened (double-click, Enter) by copying it to a temporary
//! folder and handing that copy to the program the system picks; files and
//! folders are downloaded to where the user saves them. Copies in progress
//! and their failures show below the tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use berth_docker::{
    Engine, FileEntry, FileKind, FileMount, FilesTarget, Folder, HELPER_IMAGE, Resolution,
    display_name, join_path, resolve_entry, resolve_path,
};
use berth_ui::{
    Button, Disclosure, FileTree, FileTreeEntry, FileTreeRow, IconButton, Spinner, Theme,
    empty_state, icon, icons,
};
use gpui::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Entity, FocusHandle, FontWeight, Global,
    IntoElement, Render, SharedString, Task, UniformListScrollHandle, WeakEntity, Window, div,
    prelude::*, px,
};

use crate::format;

/// The user allowed helper containers this session, so Files tabs start
/// them without asking again.
struct HelpersAllowed;

impl Global for HelpersAllowed {}

/// The Files tab of a container or volume: asks once per session before
/// starting a helper, then browses.
pub struct FilesTab {
    engine: Arc<dyn Engine>,
    target: FilesTarget,
    /// Notes for paths, e.g. the volume mounted there.
    tags: Vec<(String, SharedString)>,
    state: TabState,
}

enum TabState {
    /// Waiting for the user to allow a helper container.
    Asking,
    /// Owns the request; dropping it gives the request up.
    Opening(#[expect(dead_code, reason = "held to keep the request alive")] Task<()>),
    Browsing(Entity<FileBrowser>),
    Failed(String),
}

impl FilesTab {
    pub fn new(
        engine: Arc<dyn Engine>,
        target: FilesTarget,
        tags: Vec<(String, SharedString)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            engine,
            target,
            tags,
            state: TabState::Asking,
        };
        if cx.has_global::<HelpersAllowed>() {
            this.open(cx);
        }
        this
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let request = self.engine.open_files(&self.target);
        let engine = self.engine.clone();
        let tags = self.tags.clone();
        self.state = TabState::Opening(cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |this, cx| {
                this.state = match result {
                    Ok(mount) => {
                        TabState::Browsing(cx.new(|cx| FileBrowser::new(engine, mount, tags, cx)))
                    }
                    Err(error) => TabState::Failed(error.to_string()),
                };
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn allow(&mut self, cx: &mut Context<Self>) {
        cx.set_global(HelpersAllowed);
        self.open(cx);
    }
}

impl Render for FilesTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        // `docker.io/library/busybox:1.37` → `busybox`.
        let image = HELPER_IMAGE.rsplit('/').next().unwrap_or(HELPER_IMAGE);
        let image = image.split(':').next().unwrap_or(image);
        let (title, detail, action) = match &self.state {
            TabState::Browsing(browser) => return browser.clone().into_any_element(),
            TabState::Opening(_) => {
                return div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(Theme::SPACE_SM))
                    .text_color(theme.text_muted)
                    .child(Spinner::new("files-opening"))
                    .child("Starting a helper container…")
                    .into_any_element();
            }
            TabState::Asking => (
                "Browse files",
                match self.target {
                    FilesTarget::Container(_) => format!(
                        "Berth starts a temporary {image} container that can read this \
                         container's files, whatever its image, and removes it when you \
                         leave this tab. Nothing runs in this container. The image is \
                         pulled if it isn't present."
                    ),
                    FilesTarget::Volume(_) => format!(
                        "Berth mounts the volume read-only in a temporary {image} \
                         container to list its files, and removes it when you leave this \
                         tab. The image is pulled if it isn't present."
                    ),
                },
                "Browse files",
            ),
            TabState::Failed(error) => ("Couldn't open the files", error.clone(), "Try again"),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_LG * 3.0))
            .child(
                div()
                    .text_size(px(Theme::TEXT_LG))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .text_center()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_faint)
                    .child(detail),
            )
            .child(
                div().pt(px(Theme::SPACE_MD)).child(
                    Button::new("files-open", action)
                        .icon(icons::FOLDER)
                        .on_click(cx.listener(|this, _, _, cx| this.allow(cx))),
                ),
            )
            .into_any_element()
    }
}

#[derive(Debug)]
enum Listing {
    Loading,
    Loaded(Vec<FileEntry>),
    Failed(String),
}

/// What an entry is copied out for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// To a temporary file, opened in the program the system picks.
    Open,
    /// To where the user saves it.
    Download,
}

/// A copy out of the browsed tree, shown below it.
struct Transfer {
    id: usize,
    purpose: Purpose,
    name: SharedString,
    /// Bytes of file data written so far.
    copied: Arc<AtomicU64>,
    /// A file's size, once known; a folder's isn't.
    total: Option<u64>,
    state: TransferState,
}

enum TransferState {
    /// Owns the copy; dropping it (cancelling) stops the copy.
    Running(#[expect(dead_code, reason = "held to keep the copy alive")] Task<()>),
    /// Downloaded here.
    Saved(PathBuf),
    Failed(String),
}

/// How often running copies redraw their progress.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Opens a copied file in the program the system picks; replaced in tests.
type Opener = Rc<dyn Fn(&Path, &mut App)>;

pub struct FileBrowser {
    engine: Arc<dyn Engine>,
    /// The helper; dropped with the browser, which removes it.
    mount: FileMount,
    tags: Vec<(String, SharedString)>,
    /// Folder path → its listing, kept while the folder is collapsed.
    dirs: HashMap<String, Listing>,
    /// Real (link-free) path → its listing, for resolving links.
    real: HashMap<String, Folder>,
    expanded: HashSet<String>,
    selected: Option<SharedString>,
    rows: Rc<[FileTreeRow]>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// Listings in flight, by path.
    loads: HashMap<String, Task<()>>,
    transfers: Vec<Transfer>,
    next_transfer: usize,
    /// A redraw is scheduled while copies run.
    ticking: bool,
    open_with: Opener,
}

const ROOT: &str = "/";

impl FileBrowser {
    pub fn new(
        engine: Arc<dyn Engine>,
        mount: FileMount,
        tags: Vec<(String, SharedString)>,
        cx: &Context<Self>,
    ) -> Self {
        let mut this = Self {
            engine,
            mount,
            tags,
            dirs: HashMap::new(),
            real: HashMap::new(),
            expanded: HashSet::new(),
            selected: None,
            rows: Rc::from([]),
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            loads: HashMap::new(),
            transfers: Vec::new(),
            next_transfer: 0,
            ticking: false,
            open_with: Rc::new(|path, cx| cx.open_with_system(path)),
        };
        this.load(ROOT.to_owned(), cx);
        this
    }

    fn load(&mut self, path: String, cx: &Context<Self>) {
        self.dirs.insert(path.clone(), Listing::Loading);
        let task = cx.spawn({
            let path = path.clone();
            async move |this, cx| {
                let listing = match Self::fetch(&this, &path, cx).await {
                    Ok(entries) => Listing::Loaded(entries),
                    Err(error) => Listing::Failed(error),
                };
                this.update(cx, |this, cx| {
                    this.loads.remove(&path);
                    this.dirs.insert(path, listing);
                    this.rebuild();
                    cx.notify();
                })
                .ok();
            }
        });
        self.loads.insert(path, task);
        self.rebuild();
    }

    /// The entries of `path`: its links resolved, listing the folders on
    /// the way that aren't known yet.
    #[expect(
        clippy::future_not_send,
        reason = "GPUI AsyncApp stays on the foreground executor"
    )]
    async fn fetch(
        this: &WeakEntity<Self>,
        path: &str,
        cx: &mut AsyncApp,
    ) -> Result<Vec<FileEntry>, String> {
        let dir = Self::resolve(this, path, false, cx).await?;
        let known = this
            .read_with(cx, |b, _| b.real.get(&dir).map(|f| f.entries().to_vec()))
            .map_err(closed)?;
        match known {
            Some(entries) => Ok(entries),
            None => Self::list(this, dir, cx).await,
        }
    }

    /// The real path `path` leads to, listing the folders and reading the
    /// links on the way that aren't known yet. With `any_entry` it may lead
    /// to a file ([`resolve_entry`]), not only to a folder.
    #[expect(
        clippy::future_not_send,
        reason = "GPUI AsyncApp stays on the foreground executor"
    )]
    async fn resolve(
        this: &WeakEntity<Self>,
        path: &str,
        any_entry: bool,
        cx: &mut AsyncApp,
    ) -> Result<String, String> {
        loop {
            let step = this
                .read_with(cx, |b, _| {
                    let lookup = |dir: &str| b.real.get(dir);
                    if any_entry {
                        resolve_entry(path, b.mount.confined, lookup)
                    } else {
                        resolve_path(path, b.mount.confined, lookup)
                    }
                })
                .map_err(closed)?;
            match step {
                Resolution::Failed(why) => return Err(why),
                Resolution::NeedsLink(link) => {
                    let request = this
                        .read_with(cx, |b, _| b.engine.read_link(&b.mount, &link))
                        .map_err(closed)?;
                    let target = request.await.map_err(|e| e.to_string())?;
                    this.update(cx, |b, _| b.learn_link(&link, target))
                        .map_err(closed)?;
                }
                Resolution::Needs(dir) => {
                    Self::list(this, dir, cx).await?;
                }
                Resolution::Real(path) => return Ok(path),
            }
        }
    }

    /// List the folder at real path `dir`, and keep its listing.
    #[expect(
        clippy::future_not_send,
        reason = "GPUI AsyncApp stays on the foreground executor"
    )]
    async fn list(
        this: &WeakEntity<Self>,
        dir: String,
        cx: &mut AsyncApp,
    ) -> Result<Vec<FileEntry>, String> {
        let request = this
            .read_with(cx, |b, _| b.engine.list_files(&b.mount, &dir))
            .map_err(closed)?;
        let entries = request.await.map_err(|e| e.to_string())?;
        this.update(cx, |b, _| b.real.insert(dir, Folder::new(entries.clone())))
            .map_err(closed)?;
        Ok(entries)
    }

    /// The kind and size of the entry at real path `path`, if listed.
    fn real_entry(&self, path: &str) -> Option<(FileKind, Option<u64>)> {
        if path == ROOT {
            return Some((FileKind::Directory, None));
        }
        let (parent, name) = path.rsplit_once('/')?;
        let parent = if parent.is_empty() { ROOT } else { parent };
        let entry = self.real.get(parent)?.get(name)?;
        Some((entry.kind, entry.size))
    }

    /// Record the target of the symlink at real path `link`.
    fn learn_link(&mut self, link: &str, target: String) {
        let (parent, name) = link.rsplit_once('/').unwrap_or(("", link));
        let parent = if parent.is_empty() { ROOT } else { parent };
        if let Some(entry) = self
            .real
            .get_mut(parent)
            .and_then(|folder| folder.get_mut(name))
        {
            entry.link_target = Some(target);
        }
    }

    /// Expand or collapse the folder at `path`, listing it when opened for
    /// the first time (or again after a failure).
    fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.expanded.remove(path) {
            // A selection inside the folder moves up to it.
            if self
                .selected
                .as_ref()
                .is_some_and(|s| s.starts_with(&format!("{path}/")))
            {
                self.selected = Some(path.to_owned().into());
            }
        } else {
            self.expanded.insert(path.to_owned());
            if !matches!(
                self.dirs.get(path),
                Some(Listing::Loaded(_) | Listing::Loading)
            ) {
                self.load(path.to_owned(), cx);
            }
        }
        self.rebuild();
        cx.notify();
    }

    /// List the root and every open folder again.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.dirs.clear();
        self.real.clear();
        self.loads.clear();
        self.load(ROOT.to_owned(), cx);
        for path in self.expanded.clone() {
            self.load(path, cx);
        }
        cx.notify();
    }

    /// Copy the entry at tree path `path` out, for `purpose`.
    fn transfer(&mut self, path: &str, purpose: Purpose, cx: &mut Context<Self>) {
        let id = self.next_transfer;
        // IDs only tell apart the transfers shown at once.
        self.next_transfer = id.wrapping_add(1);
        let copied = Arc::new(AtomicU64::new(0));
        let task = cx.spawn({
            let (path, copied) = (path.to_owned(), copied.clone());
            async move |this, cx| {
                let result = Self::copy_out(&this, id, &path, purpose, copied, cx).await;
                this.update(cx, |this, cx| this.finish(id, result, cx)).ok();
            }
        });
        let name = path.rsplit('/').next().unwrap_or(path);
        self.transfers.push(Transfer {
            id,
            purpose,
            name: display_name(name).into(),
            copied,
            total: None,
            state: TransferState::Running(task),
        });
        self.tick(cx);
        cx.notify();
    }

    /// Where the entry at `path` was copied, or `None` if the user didn't
    /// choose where to save it.
    #[expect(
        clippy::future_not_send,
        reason = "GPUI AsyncApp stays on the foreground executor"
    )]
    async fn copy_out(
        this: &WeakEntity<Self>,
        id: usize,
        path: &str,
        purpose: Purpose,
        copied: Arc<AtomicU64>,
        cx: &mut AsyncApp,
    ) -> Result<Option<PathBuf>, String> {
        let real = Self::resolve(this, path, true, cx).await?;
        let found = this
            .read_with(cx, |b, _| b.real_entry(&real))
            .map_err(closed)?;
        let (kind, size) = found.ok_or_else(|| format!("{} doesn't exist", display_name(path)))?;
        let name = local_name(path.rsplit('/').next().unwrap_or(path));
        let dest = match (purpose, kind) {
            (_, FileKind::Symlink | FileKind::Other) => {
                return Err("Only files and folders can be copied out.".into());
            }
            (Purpose::Open, FileKind::Directory) => {
                return Err("A folder can't be opened. Download it instead.".into());
            }
            // A folder of its own, so the copy keeps its name.
            (Purpose::Open, FileKind::File) => berth_docker::private_dir("berth-")
                .tempdir()
                .map_err(|e| format!("Couldn't make a temporary folder: {e}"))?
                .keep()
                .join(name),
            (Purpose::Download, _) => {
                let folder = dirs::download_dir()
                    .or_else(dirs::home_dir)
                    .unwrap_or_default();
                let chosen = cx.update(|cx| cx.prompt_for_new_path(&folder, Some(&name)));
                match chosen.await {
                    Ok(Ok(Some(dest))) => dest,
                    Ok(Ok(None)) | Err(_) => return Ok(None),
                    Ok(Err(error)) => return Err(error.to_string()),
                }
            }
        };
        let request = this
            .update(cx, |b, _| {
                if let Some(transfer) = b.transfers.iter_mut().find(|t| t.id == id) {
                    transfer.total = size.filter(|_| kind == FileKind::File);
                }
                b.engine.copy_out(&b.mount, &real, &dest, copied)
            })
            .map_err(closed)?;
        if let Err(error) = request.await {
            if purpose == Purpose::Open
                && let Some(folder) = dest.parent()
            {
                let _ = std::fs::remove_dir(folder);
            }
            return Err(error.to_string());
        }
        Ok(Some(dest))
    }

    fn finish(
        &mut self,
        id: usize,
        result: Result<Option<PathBuf>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.transfers.iter().position(|t| t.id == id) else {
            return;
        };
        match result {
            Ok(None) => drop(self.transfers.remove(ix)),
            Ok(Some(dest)) => {
                let transfer = self.transfers.remove(ix);
                match transfer.purpose {
                    Purpose::Open => {
                        let open = self.open_with.clone();
                        open(&dest, cx);
                    }
                    Purpose::Download => self.transfers.insert(
                        ix,
                        Transfer {
                            state: TransferState::Saved(dest),
                            ..transfer
                        },
                    ),
                }
            }
            Err(error) => {
                if let Some(transfer) = self.transfers.get_mut(ix) {
                    transfer.state = TransferState::Failed(error);
                }
            }
        }
        cx.notify();
    }

    /// Remove a transfer, cancelling it if it still runs.
    fn dismiss(&mut self, id: usize, cx: &mut Context<Self>) {
        self.transfers.retain(|t| t.id != id);
        cx.notify();
    }

    /// Redraw every [`PROGRESS_INTERVAL`] while copies run.
    fn tick(&mut self, cx: &Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_INTERVAL).await;
                let running = this.update(cx, |b, cx| {
                    cx.notify();
                    b.ticking = b
                        .transfers
                        .iter()
                        .any(|t| matches!(t.state, TransferState::Running(_)));
                    b.ticking
                });
                if !running.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        self.flatten(ROOT, 0, &mut rows);
        self.rows = rows.into();
        // Once listings settle, a selection that went away (deleted, or in
        // a folder that failed to list) moves to its nearest shown parent.
        if self.loads.is_empty() {
            while let Some(selected) = self.selected.clone()
                && !self.rows.iter().any(|row| *row.key() == selected)
            {
                self.selected = selected
                    .rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .filter(|parent| !parent.is_empty())
                    .map(|parent| parent.to_owned().into());
            }
        }
    }

    fn flatten(&self, path: &str, depth: usize, out: &mut Vec<FileTreeRow>) {
        let note = |text: String, error: bool| FileTreeRow::Note {
            // Paths never end in '/', so this can't collide with an entry.
            key: format!("{}/", path.trim_end_matches('/')).into(),
            depth,
            text: text.into(),
            error,
        };
        let entries = match self.dirs.get(path) {
            Some(Listing::Loaded(entries)) => entries,
            Some(Listing::Failed(error)) => return out.push(note(error.clone(), true)),
            Some(Listing::Loading) | None => return,
        };
        if entries.is_empty() {
            out.push(note("Empty folder".into(), false));
        }
        for entry in entries {
            let child = join_path(path, &entry.name);
            let open = self.expanded.contains(&child);
            let expandable = self.expandable(entry, &child);
            let disclosure = expandable.then(|| match self.dirs.get(&child) {
                _ if !open => Disclosure::Collapsed,
                Some(Listing::Loading) | None => Disclosure::Loading,
                Some(_) => Disclosure::Expanded,
            });
            out.push(FileTreeRow::Entry(
                self.entry_row(entry, &child, depth, disclosure),
            ));
            if open && expandable {
                self.flatten(&child, depth.saturating_add(1), out);
            }
        }
    }

    /// Whether `entry`, at tree path `path`, may open as a folder. A link
    /// is told by following it through what is known: one to a file (or
    /// nowhere) doesn't; one that can't be followed yet may, and is
    /// resolved when opened.
    fn expandable(&self, entry: &FileEntry, path: &str) -> bool {
        match entry.kind {
            FileKind::Directory => true,
            FileKind::Symlink => !matches!(
                resolve_path(path, self.mount.confined, |dir| { self.real.get(dir) }),
                Resolution::Failed(_)
            ),
            FileKind::File | FileKind::Other => false,
        }
    }

    fn entry_row(
        &self,
        entry: &FileEntry,
        path: &str,
        depth: usize,
        disclosure: Option<Disclosure>,
    ) -> FileTreeEntry {
        let size = match (entry.kind, entry.size) {
            (FileKind::File, Some(size)) => format::bytes(size),
            _ => String::new(),
        };
        FileTreeEntry {
            key: path.to_owned().into(),
            name: display_name(&entry.name).into(),
            depth,
            disclosure,
            symlink: entry.kind == FileKind::Symlink,
            modified: entry
                .modified
                .map(format::date_time)
                .unwrap_or_default()
                .into(),
            size: size.into(),
            kind: entry.kind.label().into(),
            tag: self
                .tags
                .iter()
                .find(|(tagged, _)| tagged == path)
                .map(|(_, tag)| tag.clone()),
        }
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let path = self
            .selected
            .as_ref()
            .map_or_else(|| ROOT.into(), |p| display_name(p));
        let busy = !self.loads.is_empty();
        let selected = self.selected.as_ref().and_then(|key| {
            self.rows.iter().find_map(|row| match row {
                FileTreeRow::Entry(entry) if entry.key == *key => Some(entry),
                _ => None,
            })
        });
        let open = selected
            .filter(|entry| entry.disclosure.is_none())
            .map(|entry| entry.key.clone());
        let download = selected.map(|entry| entry.key.clone());
        div()
            .h(px(36.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_SM))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.font_mono.clone())
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_muted)
                    .child(path),
            )
            .when(busy, |d| d.child(Spinner::new("files-busy").size(12.0)))
            .children(open.map(|key| {
                Button::new("files-open-file", "Open")
                    .icon(icons::FILE)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.transfer(&key, Purpose::Open, cx);
                    }))
            }))
            .children(download.map(|key| {
                Button::new("files-download", "Download")
                    .icon(icons::DOWNLOAD)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.transfer(&key, Purpose::Download, cx);
                    }))
            }))
            .child(
                IconButton::new("files-reload", icons::RESTART)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
    }
}

/// A copy's progress, outcome or failure.
fn render_transfer(transfer: &Transfer, cx: &Context<FileBrowser>) -> AnyElement {
    let theme = Theme::of(cx);
    let id = transfer.id;
    let name = &transfer.name;
    let (lead, title, detail, error) = match (&transfer.state, transfer.purpose) {
        (TransferState::Running(_), purpose) => {
            let copied = format::bytes(transfer.copied.load(Ordering::Relaxed));
            let progress = match transfer.total {
                Some(total) => format!("{copied} of {}", format::bytes(total)),
                None => copied,
            };
            let verb = match purpose {
                Purpose::Open => "Opening",
                Purpose::Download => "Downloading",
            };
            (
                Spinner::new(("transfer-spinner", id))
                    .size(12.0)
                    .into_any_element(),
                format!("{verb} {name}…"),
                progress,
                false,
            )
        }
        (TransferState::Saved(dest), _) => (
            icon(icons::CHECK)
                .size(px(14.0))
                .text_color(theme.success)
                .into_any_element(),
            format!("Downloaded {name}"),
            dest.display().to_string(),
            false,
        ),
        (TransferState::Failed(why), purpose) => (
            icon(icons::DANGER)
                .size(px(14.0))
                .text_color(theme.danger)
                .into_any_element(),
            match purpose {
                Purpose::Open => format!("Couldn't open {name}"),
                Purpose::Download => format!("Couldn't download {name}"),
            },
            why.clone(),
            true,
        ),
    };
    let reveal = match &transfer.state {
        TransferState::Saved(dest) => Some(dest.clone()),
        _ => None,
    };
    div()
        .h(px(32.0))
        .flex()
        .items_center()
        .gap(px(Theme::SPACE_SM))
        .px(px(Theme::SPACE_SM))
        .rounded(px(Theme::CONTROL_RADIUS))
        .bg(theme.surface_raised)
        .text_size(px(Theme::TEXT_SM))
        .child(
            div()
                .w(px(14.0))
                .flex_none()
                .flex()
                .justify_center()
                .child(lead),
        )
        .child(
            div()
                .flex_none()
                .max_w(px(320.0))
                .truncate()
                .text_color(theme.text)
                .child(title),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(if error {
                    theme.danger
                } else {
                    theme.text_muted
                })
                .child(detail),
        )
        .children(reveal.map(|dest| {
            Button::new(("transfer-reveal", id), "Show")
                .on_click(move |_, _, cx| cx.reveal_path(&dest))
        }))
        .child(
            IconButton::new(("transfer-dismiss", id), icons::CLOSE)
                .size(22.0, 13.0)
                .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx))),
        )
        .into_any_element()
}

/// A name for the local copy of the entry named `name` (escaped): as it
/// shows, with what Windows doesn't allow in names replaced.
fn local_name(name: &str) -> String {
    let shown = display_name(name);
    if cfg!(windows) {
        shown
            .chars()
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    } else {
        shown
    }
}

/// The error of an update to a browser that has gone.
fn closed<E>(_: E) -> String {
    "Closed".to_owned()
}

impl Render for FileBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let body = match self.dirs.get(ROOT) {
            Some(Listing::Failed(error)) => div()
                .size_full()
                .flex()
                .flex_col()
                .child(empty_state("Couldn't list files", error.clone(), theme))
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .pb(px(Theme::SPACE_LG * 3.0))
                        .child(
                            Button::new("files-retry", "Try again")
                                .icon(icons::RESTART)
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        ),
                )
                .into_any_element(),
            Some(Listing::Loading) | None => {
                empty_state("Loading…", "Listing files", theme).into_any_element()
            }
            Some(Listing::Loaded(_)) => {
                let tree = cx.entity().downgrade();
                let toggle = tree.clone();
                let open = tree.clone();
                FileTree::new("file-tree", self.rows.clone(), &self.focus, &self.scroll)
                    .selected(self.selected.clone())
                    .on_select(move |key, _, cx| {
                        tree.update(cx, |this, cx| {
                            this.selected = Some(key.clone());
                            cx.notify();
                        })
                        .ok();
                    })
                    .on_toggle(move |key, _, cx| {
                        toggle.update(cx, |this, cx| this.toggle(key, cx)).ok();
                    })
                    .on_open(move |key, _, cx| {
                        open.update(cx, |this, cx| this.transfer(key, Purpose::Open, cx))
                            .ok();
                    })
                    .into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .px(px(Theme::SPACE_LG))
            .pb(px(Theme::SPACE_SM))
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
            .when(!self.transfers.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(Theme::SPACE_XS))
                        .pt(px(Theme::SPACE_SM))
                        .children(self.transfers.iter().map(|t| render_transfer(t, cx))),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use berth_docker::ContainerState;
    use berth_docker::fixtures::FixtureEngine;
    use futures::executor::block_on;
    use gpui::TestAppContext;

    use super::*;

    fn keys(rows: &[FileTreeRow]) -> Vec<String> {
        rows.iter().map(|r| r.key().to_string()).collect()
    }

    fn running_id() -> String {
        berth_docker::fixtures::containers()
            .into_iter()
            .find(|c| c.state == ContainerState::Running)
            .unwrap()
            .id
    }

    fn browser(
        cx: &mut TestAppContext,
        engine: &Arc<dyn Engine>,
        target: &FilesTarget,
        tags: Vec<(String, SharedString)>,
    ) -> Entity<FileBrowser> {
        let mount = block_on(engine.open_files(target)).unwrap();
        let engine = engine.clone();
        let browser = cx.new(|cx| FileBrowser::new(engine, mount, tags, cx));
        cx.run_until_parked();
        browser
    }

    fn entry<'a>(b: &'a FileBrowser, key: &str) -> &'a FileTreeEntry {
        match b.rows.iter().find(|r| r.key() == key) {
            Some(FileTreeRow::Entry(entry)) => entry,
            _ => panic!("{key} listed"),
        }
    }

    #[gpui::test]
    fn folders_load_when_opened_and_fold_away(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let target = FilesTarget::Container(running_id());
        let tags = vec![("/data".into(), "volume pgdata".into())];
        let browser = browser(cx, &engine, &target, tags);
        browser.read_with(cx, |b, _| {
            let rows = keys(&b.rows);
            assert_eq!(rows.first().map(String::as_str), Some("/bin"));
            assert!(rows.contains(&"/etc".to_string()));
            assert_eq!(entry(b, "/data").tag.as_deref(), Some("volume pgdata"));
        });

        browser.update(cx, |b, cx| b.toggle("/etc", cx));
        browser.read_with(cx, |b, _| {
            assert_eq!(entry(b, "/etc").disclosure, Some(Disclosure::Loading));
        });
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            let rows = keys(&b.rows);
            let etc = rows.iter().position(|k| k == "/etc").unwrap();
            // Links sort with folders.
            assert_eq!(
                rows.get(etc.saturating_add(1)).map(String::as_str),
                Some("/etc/localtime")
            );
            assert_eq!(
                rows.get(etc.saturating_add(2)).map(String::as_str),
                Some("/etc/nginx")
            );
        });

        // An unreadable folder shows its error in place.
        browser.update(cx, |b, cx| b.toggle("/root", cx));
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            assert!(b.rows.iter().any(|r| matches!(
                r,
                FileTreeRow::Note { key, error: true, .. } if key == "/root/"
            )));
        });

        // Collapsing a selected entry's folder selects the folder.
        browser.update(cx, |b, cx| {
            b.selected = Some("/etc/nginx".into());
            b.toggle("/etc", cx);
        });
        browser.read_with(cx, |b, _| {
            assert!(!keys(&b.rows).contains(&"/etc/nginx".to_string()));
            assert_eq!(b.selected.as_deref(), Some("/etc"));
        });

        // A selection that disappears on reload moves to its parent, or
        // clears at the root.
        browser.update(cx, |b, cx| {
            b.toggle("/etc", cx);
            b.selected = Some("/etc/gone".into());
            b.reload(cx);
        });
        cx.run_until_parked();
        browser.read_with(cx, |b, _| assert_eq!(b.selected.as_deref(), Some("/etc")));
        browser.update(cx, |b, cx| {
            b.selected = Some("/gone".into());
            b.reload(cx);
        });
        cx.run_until_parked();
        browser.read_with(cx, |b, _| assert_eq!(b.selected, None));
    }

    #[gpui::test]
    fn links_open_where_they_point_in_the_browsed_tree(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let target = FilesTarget::Container(running_id());
        let browser = browser(cx, &engine, &target, Vec::new());
        // `/bin → usr/bin` and `/lib64 → /usr/lib`: `/usr` is listed on the
        // way, unseen.
        browser.update(cx, |b, cx| {
            b.toggle("/bin", cx);
            b.toggle("/lib64", cx);
            b.toggle("/etc", cx);
        });
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            let rows = keys(&b.rows);
            assert!(rows.contains(&"/bin/tool-000".to_string()));
            assert!(!rows.contains(&"/usr/bin".to_string()));
            assert!(b.real.contains_key("/usr"));
            // A link to a file next to it is told apart straight away.
            assert_eq!(entry(b, "/bin/arch").disclosure, None);
            assert_eq!(entry(b, "/bin/busybox").disclosure, None);
            // One leading where nothing is known yet may be a folder…
            assert!(entry(b, "/etc/localtime").disclosure.is_some());
        });
        // …until opening it finds out it leads nowhere.
        browser.update(cx, |b, cx| b.toggle("/etc/localtime", cx));
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            assert_eq!(entry(b, "/etc/localtime").disclosure, None);
            assert!(!keys(&b.rows).contains(&"/etc/localtime/".to_string()));
        });
    }

    #[gpui::test]
    fn volume_links_stay_in_the_volume(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let volume = block_on(engine.list_volumes()).unwrap().remove(0).name;
        let browser = browser(cx, &engine, &FilesTarget::Volume(volume), Vec::new());
        // A link out of the volume isn't a folder here.
        browser.read_with(cx, |b, _| {
            assert_eq!(entry(b, "/escape").disclosure, None);
        });
        browser.update(cx, |b, cx| b.toggle("/current", cx));
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            assert!(keys(&b.rows).contains(&"/current/".to_string()));
        });
    }

    #[gpui::test]
    fn files_open_from_a_temporary_copy(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let target = FilesTarget::Container(running_id());
        let browser = browser(cx, &engine, &target, Vec::new());
        let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
        browser.update(cx, |b, cx| {
            let opened = opened.clone();
            b.open_with = Rc::new(move |path, _| opened.borrow_mut().push(path.to_owned()));
            b.transfer("/etc/hosts", Purpose::Open, cx);
            // Links are followed in the browsed tree: `/bin → usr/bin`,
            // `arch → /usr/bin/busybox`.
            b.transfer("/bin/arch", Purpose::Open, cx);
            // `localtime` leads nowhere.
            b.transfer("/etc/localtime", Purpose::Open, cx);
            assert_eq!(b.transfers.len(), 3);
        });
        cx.run_until_parked();
        let mut opened = opened.take();
        opened.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        let read = |path: &PathBuf| {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            (name, std::fs::read_to_string(path).unwrap())
        };
        assert_eq!(
            opened.iter().map(read).collect::<Vec<_>>(),
            [
                ("arch".to_owned(), "/usr/bin/busybox\n".to_owned()),
                ("hosts".to_owned(), "/etc/hosts\n".to_owned())
            ]
        );
        for path in &opened {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
        // Opened copies go; a failure stays until dismissed.
        browser.update(cx, |b, cx| {
            let failed = b.transfers.first().unwrap();
            assert!(
                matches!(&failed.state, TransferState::Failed(why) if why.contains("doesn't exist"))
            );
            b.dismiss(failed.id, cx);
            assert!(b.transfers.is_empty());
        });
    }

    #[gpui::test]
    fn files_and_folders_download_where_saved(cx: &mut TestAppContext) {
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let volume = block_on(engine.list_volumes()).unwrap().remove(0).name;
        let browser = browser(cx, &engine, &FilesTarget::Volume(volume), Vec::new());
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("saved");
        browser.update(cx, |b, cx| b.transfer("/base", Purpose::Download, cx));
        cx.run_until_parked();
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| Some(dest.clone()));
        cx.run_until_parked();
        assert!(dest.join("1").is_dir());
        browser.read_with(cx, |b, _| {
            assert!(matches!(
                &b.transfers.first().unwrap().state,
                TransferState::Saved(saved) if *saved == dest
            ));
        });
        // A file replaces what was saved there; not choosing a place drops
        // the download.
        browser.update(cx, |b, cx| {
            b.transfers.clear();
            b.transfer("/PG_VERSION", Purpose::Download, cx);
            b.transfer("/postgresql.conf", Purpose::Download, cx);
        });
        cx.run_until_parked();
        cx.simulate_new_path_selection(|_| Some(dest.clone()));
        cx.run_until_parked();
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "/PG_VERSION\n");
        browser.read_with(cx, |b, _| assert_eq!(b.transfers.len(), 1));
        // Links out of a volume can't be copied.
        browser.update(cx, |b, cx| {
            b.transfers.clear();
            b.transfer("/escape", Purpose::Download, cx);
        });
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            assert!(matches!(
                &b.transfers.first().unwrap().state,
                TransferState::Failed(why) if why.contains("outside")
            ));
        });
    }

    #[gpui::test]
    fn the_tab_asks_once_and_the_helper_lives_with_it(cx: &mut TestAppContext) {
        let fixture = Arc::new(FixtureEngine::default());
        let engine: Arc<dyn Engine> = fixture.clone();
        let helpers = || {
            block_on(fixture.list_containers())
                .unwrap()
                .iter()
                .filter(|c| c.is_helper())
                .count()
        };
        let target = FilesTarget::Container(running_id());
        let tab = cx.new(|cx| FilesTab::new(engine.clone(), target.clone(), Vec::new(), cx));
        tab.read_with(cx, |t, _| assert!(matches!(t.state, TabState::Asking)));
        assert_eq!(helpers(), 0);
        tab.update(cx, FilesTab::allow);
        cx.run_until_parked();
        tab.read_with(cx, |t, _| assert!(matches!(t.state, TabState::Browsing(_))));
        assert_eq!(helpers(), 1);
        assert!(cx.read(gpui::App::has_global::<HelpersAllowed>));
        // Released at the end of the next effect cycle, the tab and its
        // browser (and so its helper) go.
        drop(tab);
        cx.update(|_| {});
        cx.run_until_parked();
        assert_eq!(helpers(), 0);
        // Allowed once: the next tab opens straight away.
        let tab = cx.new(|cx| FilesTab::new(engine, target, Vec::new(), cx));
        cx.run_until_parked();
        tab.read_with(cx, |t, _| assert!(matches!(t.state, TabState::Browsing(_))));
    }

    #[gpui::test]
    fn the_tree_renders_in_a_window(cx: &mut TestAppContext) {
        cx.update(|cx| berth_ui::install_theme(berth_ui::Theme::dark(), cx));
        let engine: Arc<dyn Engine> = Arc::new(FixtureEngine::default());
        let mount = block_on(engine.open_files(&FilesTarget::Container(running_id()))).unwrap();
        let (browser, cx) =
            cx.add_window_view(|_, cx| FileBrowser::new(engine, mount, Vec::new(), cx));
        cx.run_until_parked();
        browser.update(cx, |b, cx| {
            b.toggle("/usr", cx);
            b.toggle("/usr/bin", cx);
        });
        cx.run_until_parked();
        // 14 root entries, /usr's 4 and 402 in /usr/bin: drawn virtualized.
        browser.read_with(cx, |b, _| assert_eq!(b.rows.len(), 420));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        // Columns line up whatever the name's length ("bin", then the
        // longer "lib64" link, "usr/bin" entries deeper in).
        let kind = |cx: &mut gpui::VisualTestContext, selector| {
            cx.debug_bounds(selector).expect("row drawn").origin.x
        };
        let first = kind(cx, "file-tree-kind-0");
        assert_eq!(kind(cx, "file-tree-kind-8"), first);
        assert_eq!(kind(cx, "file-tree-kind-12"), first);
        // Copies draw below the tree: one waiting to be saved, one failed.
        browser.update(cx, |b, cx| {
            b.selected = Some("/etc/hosts".into());
            b.transfer("/etc/hosts", Purpose::Download, cx);
            b.transfer("/etc/localtime", Purpose::Open, cx);
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        browser.read_with(cx, |b, _| {
            assert!(matches!(
                b.transfers.as_slice(),
                [
                    Transfer {
                        state: TransferState::Running(_),
                        ..
                    },
                    Transfer {
                        state: TransferState::Failed(_),
                        ..
                    }
                ]
            ));
        });
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
    }
}

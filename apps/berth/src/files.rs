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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use berth_docker::{
    Engine, FileEntry, FileKind, FileMount, FilesTarget, Folder, HELPER_IMAGE, Resolution,
    display_name, join_path, resolve_path,
};
use berth_ui::{
    Button, Disclosure, FileTree, FileTreeEntry, FileTreeRow, IconButton, Spinner, Theme,
    empty_state, icons,
};
use gpui::{
    AppContext as _, AsyncApp, Context, Entity, FocusHandle, FontWeight, Global, IntoElement,
    Render, SharedString, Task, UniformListScrollHandle, WeakEntity, Window, div, prelude::*, px,
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
        let closed = |_| "Closed".to_owned();
        loop {
            let step = this
                .read_with(cx, |b, _| {
                    resolve_path(path, b.mount.confined, |dir| b.real.get(dir))
                })
                .map_err(closed)?;
            let (dir, done) = match step {
                Resolution::Failed(why) => return Err(why),
                Resolution::NeedsLink(link) => {
                    let request = this
                        .read_with(cx, |b, _| b.engine.read_link(&b.mount, &link))
                        .map_err(closed)?;
                    let target = request.await.map_err(|e| e.to_string())?;
                    this.update(cx, |b, _| b.learn_link(&link, target))
                        .map_err(closed)?;
                    continue;
                }
                Resolution::Needs(dir) => (dir, false),
                Resolution::Real(dir) => (dir, true),
            };
            let known = this
                .read_with(cx, |b, _| b.real.get(&dir).map(|f| f.entries().to_vec()))
                .map_err(closed)?;
            let entries = if let Some(entries) = known {
                entries
            } else {
                let request = this
                    .read_with(cx, |b, _| b.engine.list_files(&b.mount, &dir))
                    .map_err(closed)?;
                let entries = request.await.map_err(|e| e.to_string())?;
                this.update(cx, |b, _| b.real.insert(dir, Folder::new(entries.clone())))
                    .map_err(closed)?;
                entries
            };
            if done {
                return Ok(entries);
            }
        }
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
            .child(
                IconButton::new("files-reload", icons::RESTART)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
    }
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
    }
}

//! Volumes screen: "In use" / "Unused" sections with sizes, multi-select,
//! single and bulk delete, and remove-all-unused. The Files tab browses a
//! volume through a helper container (C5).

use std::rc::Rc;
use std::sync::Arc;

use berth_docker::{Engine, FilesTarget, Volume};
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, SectionLabel, SegmentedTabs, Theme, empty_state, icons,
};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, Modifiers,
    Render, SharedString, Window, div, list, prelude::*, px,
};

use super::{
    Chrome, ListOp, ListScreen, ShowContainer, detail_pane, list_column, scroll_page, used_by,
};
use crate::dialogs::ConfirmRequest;
use crate::files::FilesTab;
use crate::format;
use crate::selection::MultiSelect;
use crate::store::{PruneTarget, Store};
use crate::virtual_rows::VirtualRows;

const TABS: [&str; 2] = ["Info", "Files"];
const FILES_TAB: usize = 1;

/// The Files tab of one volume, on one engine. Its helper container is
/// removed when this is dropped: on leaving the tab, the volume or the
/// screen.
struct VolumeFiles {
    volume: String,
    engine: Arc<dyn Engine>,
    tab: Entity<FilesTab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Section { in_use: bool, count: usize },
    Volume(String),
}

pub struct VolumesScreen {
    store: Entity<Store>,
    rows: VirtualRows<Row>,
    /// Selectable names in display order (for shift-ranges).
    order: Vec<String>,
    selection: MultiSelect<String>,
    chrome: Chrome,
    /// A volume to scroll into view once the rows are next synced.
    reveal: Option<String>,
    tab: usize,
    /// Whether the shell is showing this screen.
    visible: bool,
    files: Option<VolumeFiles>,
}

impl EventEmitter<ConfirmRequest> for VolumesScreen {}
impl EventEmitter<ShowContainer> for VolumesScreen {}

impl VolumesScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |this, store, cx| {
            let store = store.read(cx);
            this.selection
                .retain(|name| store.volumes().iter().any(|v| &v.name == name));
            cx.notify();
        })
        .detach();
        Self {
            store,
            rows: VirtualRows::new(),
            order: Vec::new(),
            selection: MultiSelect::default(),
            chrome: Chrome::default(),
            reveal: None,
            tab: 0,
            visible: true,
            files: None,
        }
    }

    /// The shell shows or hides this screen; a hidden screen keeps no
    /// helper container running.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            if !visible {
                self.files = None;
            }
            cx.notify();
        }
    }

    fn set_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        if tab < TABS.len() && self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    /// Make the Files tab match the selected volume: a change drops the old
    /// helper.
    fn sync_files(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let want = match (self.selection.single(), store.engine()) {
            // Only volumes of the current engine: a selection can outlive
            // an engine switch.
            (Some(name), Some(engine))
                if self.tab == FILES_TAB
                    && self.visible
                    && store.volumes().iter().any(|v| &v.name == name) =>
            {
                Some((name.clone(), engine))
            }
            _ => None,
        };
        let same = match (&want, &self.files) {
            (Some((name, engine)), Some(files)) => {
                *name == files.volume && Arc::ptr_eq(engine, &files.engine)
            }
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.files = want.map(|(volume, engine)| VolumeFiles {
                tab: cx.new(|cx| {
                    FilesTab::new(
                        engine.clone(),
                        FilesTarget::Volume(volume.clone()),
                        Vec::new(),
                        cx,
                    )
                }),
                volume,
                engine,
            });
        }
    }

    fn render_files(&self, cx: &Context<Self>) -> AnyElement {
        self.files.as_ref().map_or_else(
            || {
                empty_state("Not connected", "Waiting for the engine…", Theme::of(cx))
                    .into_any_element()
            },
            |files| files.tab.clone().into_any_element(),
        )
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
            cx.notify();
        }
    }

    /// Select only `name` and scroll it into view.
    pub fn reveal(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.to_string();
        self.selection
            .click(&name, Modifiers::default(), &self.order);
        self.reveal = Some(name);
        cx.notify();
    }

    #[cfg(test)]
    pub fn selected(&self) -> Option<&str> {
        self.selection.single().map(String::as_str)
    }

    /// Returns whether any rows changed.
    fn sync_rows(&mut self, store: &Store) -> bool {
        let (mut in_use, mut unused): (Vec<&Volume>, Vec<&Volume>) =
            store.volumes().iter().partition(|v| store.volume_in_use(v));
        in_use.sort_by(|a, b| a.name.cmp(&b.name));
        unused.sort_by(|a, b| a.name.cmp(&b.name));

        let mut rows = Vec::new();
        let mut order = Vec::new();
        for (flag, group) in [(true, &in_use), (false, &unused)] {
            if group.is_empty() {
                continue;
            }
            rows.push(Row::Section {
                in_use: flag,
                count: group.len(),
            });
            for v in group {
                rows.push(Row::Volume(v.name.clone()));
                order.push(v.name.clone());
            }
        }
        let changed = self.rows.sync(rows);
        self.order = order;
        changed
    }

    fn click(&mut self, name: &str, event: &ClickEvent, cx: &mut Context<Self>) {
        self.selection
            .click(&name.to_string(), event.modifiers(), &self.order);
        cx.notify();
    }

    fn confirm_remove(&self, names: Vec<String>, title: String, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.emit(ConfirmRequest {
            title: title.into(),
            message: "Their data is deleted permanently.".into(),
            items: names.iter().cloned().map(SharedString::from).collect(),
            confirm_label: "Delete".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| store.remove_volumes(names.clone(), cx));
            }),
        });
    }

    /// Delete the selected volumes that aren't in use (Delete key).
    fn delete_selected(&self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let names: Vec<String> = store
            .volumes()
            .iter()
            .filter(|v| self.selection.contains(&v.name) && !store.volume_in_use(v))
            .map(|v| v.name.clone())
            .collect();
        let title = match names.as_slice() {
            [] => return,
            [name] => format!("Delete volume {name}?"),
            _ => format!("Delete {} volumes?", names.len()),
        };
        self.confirm_remove(names, title, cx);
    }

    fn render_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let row = match self.rows.get(ix) {
            None => return div().into_any_element(),
            Some(Row::Section { in_use, count }) => {
                SectionLabel::new(if *in_use { "In use" } else { "Unused" })
                    .count(*count)
                    .into_any_element()
            }
            Some(Row::Volume(name)) => {
                let Some(volume) = store.volumes().iter().find(|v| &v.name == name) else {
                    return div().into_any_element();
                };
                self.volume_row(store, volume, cx).into_any_element()
            }
        };
        div()
            .px(px(Theme::SPACE_SM))
            .pb(px(2.0))
            .child(row)
            .into_any_element()
    }

    fn volume_row(&self, store: &Store, volume: &Volume, cx: &Context<Self>) -> ResourceRow {
        let name = volume.name.clone();
        let in_use = store.volume_in_use(volume);
        let subtitle = match (volume.size, store.usage_known()) {
            (Some(size), _) => format::bytes(size),
            (None, true) => "Size unknown".into(),
            (None, false) => "Measuring…".into(),
        };
        let delete = {
            let name = name.clone();
            cx.listener(move |this, _, _, cx| {
                this.confirm_remove(vec![name.clone()], format!("Delete volume {name}?"), cx);
            })
        };
        let click = {
            let name = name.clone();
            cx.listener(move |this, event: &ClickEvent, _, cx| this.click(&name, event, cx))
        };
        ResourceRow::new(
            SharedString::from(format!("vol-{name}")),
            middle_ellipsis(&name, 34),
        )
        .leading(Badge::icon(name.clone(), icons::HARD_DRIVE))
        .subtitle(subtitle)
        .busy(store.pending(&name).map(|_| "Deleting…"))
        .selected(self.selection.contains(&name))
        .keyboard_cursor(self.selection.is_cursor(&name))
        .action(
            IconButton::new(SharedString::from(format!("delete-{name}")), icons::TRASH)
                .tone(ButtonTone::Danger)
                .disabled(in_use)
                .tab_stop(false)
                .on_click(delete),
        )
        .on_click(click)
    }

    fn render_detail(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        if self.selection.is_empty() {
            return empty_state(
                "No selection",
                "Select a volume to inspect it. Shift- or Ctrl-click to select several.",
                theme,
            )
            .into_any_element();
        }
        if let Some(name) = self.selection.single()
            && let Some(volume) = store.volumes().iter().find(|v| &v.name == name)
        {
            if self.tab == FILES_TAB {
                return self.render_files(cx);
            }
            return scroll_page("volume-detail", volume_info(store, volume, theme, cx))
                .into_any_element();
        }
        self.render_bulk(store, cx)
    }

    fn render_bulk(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let selected: Vec<&Volume> = store
            .volumes()
            .iter()
            .filter(|v| self.selection.contains(&v.name))
            .collect();
        let size: u64 = selected.iter().filter_map(|v| v.size).sum();
        let deletable: Vec<String> = selected
            .iter()
            .filter(|v| !store.volume_in_use(v))
            .map(|v| v.name.clone())
            .collect();
        let skipped = selected.len().saturating_sub(deletable.len());
        let count = deletable.len();

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(Theme::SPACE_SM))
            .child(
                div()
                    .text_size(px(Theme::TEXT_LG + 2.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(format!("{} selected", selected.len())),
            )
            .child(
                div()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_faint)
                    .child(if skipped > 0 {
                        format!(
                            "{} total · {skipped} in use will be kept",
                            format::bytes(size)
                        )
                    } else {
                        format!("{} total", format::bytes(size))
                    }),
            )
            .when(count > 0, |d| {
                d.child(
                    div().pt(px(Theme::SPACE_MD)).child(
                        Button::new(
                            "delete-selected",
                            format!("Delete {count} volume{}", plural(count)),
                        )
                        .icon(icons::TRASH)
                        .solid()
                        .tone(ButtonTone::Danger)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_remove(
                                deletable.clone(),
                                format!("Delete {count} volume{}?", plural(count)),
                                cx,
                            );
                        })),
                    ),
                )
            })
            .into_any_element()
    }
}

impl ListScreen for VolumesScreen {
    fn list_op(&mut self, op: ListOp, cx: &mut Context<Self>) {
        if op == ListOp::Delete {
            self.delete_selected(cx);
        } else if let Some(key) = op.apply(&mut self.selection, &self.order)
            && let Some(ix) = self
                .rows
                .position(|row| matches!(row, Row::Volume(k) if *k == key))
        {
            self.rows.scroll_into_view(ix);
        }
        cx.notify();
    }
}

impl Render for VolumesScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_files(cx);
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let rows_changed = self.sync_rows(store);
        if let Some(name) = self.reveal.take()
            && let Some(ix) = self
                .rows
                .position(|row| matches!(row, Row::Volume(row_name) if *row_name == name))
        {
            self.rows.reveal(ix, rows_changed);
        }
        let theme = Theme::of(cx);

        let unused: Vec<String> = store
            .volumes()
            .iter()
            .filter(|v| !store.volume_in_use(v))
            .map(|v| v.name.clone())
            .collect();
        let total: Option<u64> = store
            .usage_known()
            .then(|| store.volumes().iter().filter_map(|v| v.size).sum());
        let subtitle = total.map_or_else(
            || format!("{} volumes", store.volumes().len()),
            |t| format!("{} total", format::bytes(t)),
        );
        let reclaim = store.reclaimable(PruneTarget::UnusedVolumes);
        let header = ListHeader::new("Volumes").subtitle(subtitle).action(
            IconButton::new("remove-unused-volumes", icons::BROOM)
                .disabled(unused.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let title = reclaim.map_or_else(
                        || format!("Delete {} unused volumes?", unused.len()),
                        |b| {
                            format!(
                                "Delete {} unused volumes ({})?",
                                unused.len(),
                                format::bytes(b)
                            )
                        },
                    );
                    this.confirm_remove(unused.clone(), title, cx);
                })),
        );

        let body = if self.rows.is_empty() {
            empty_state(
                "No volumes",
                "Named and anonymous volumes appear here.",
                theme,
            )
            .into_any_element()
        } else {
            list(
                self.rows.state.clone(),
                cx.processor(|this, ix: usize, _, cx| this.render_row(ix, cx)),
            )
            .flex_1()
            .min_h_0()
            .pb(px(Theme::SPACE_MD))
            .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .child(list_column(
                theme,
                self.chrome,
                "volumes-titlebar",
                header,
                body,
            ))
            .child(detail_pane(
                theme,
                self.chrome,
                "volume-detail-titlebar",
                self.selection.single().is_some().then(|| {
                    SegmentedTabs::new("volume-tabs", TABS)
                        .selected(self.tab)
                        .on_select(cx.listener(|this, ix: &usize, _, cx| this.set_tab(*ix, cx)))
                        .into_any_element()
                }),
                self.render_detail(store, cx),
            ))
    }
}

fn volume_info(
    store: &Store,
    volume: &Volume,
    theme: &Theme,
    cx: &Context<VolumesScreen>,
) -> impl IntoElement {
    let users = store.volume_users(&volume.name);
    let mut info = KeyValueCard::new()
        .row("Name", Cell::plain(volume.name.clone()))
        .row("Driver", Cell::plain(volume.driver.clone()))
        .row("Mountpoint", Cell::mono(volume.mountpoint.clone()))
        .row(
            "Size",
            volume
                .size
                .map_or_else(|| Cell::muted("Unknown"), |s| Cell::plain(format::bytes(s))),
        );
    if let Some(created) = &volume.created {
        info = info.row("Created", Cell::plain(created.clone()));
    }
    if let Some(project) = volume.compose_project() {
        info = info.row("Compose project", Cell::plain(project.to_string()));
    }

    let labels = DataTable::new([("Key", 1.0), ("Value", 1.2)])
        .empty("No labels")
        .rows(
            volume
                .labels
                .iter()
                .map(|(k, v)| vec![Cell::plain(k.clone()), Cell::plain(v.clone())]),
        );

    div()
        .flex()
        .flex_col()
        .child(info)
        .child(Heading::new("Used by"))
        .child(used_by(&users, theme, cx))
        .child(Heading::new("Labels"))
        .child(labels)
}

const fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Shorten long (often 64-hex anonymous) names in the middle so both ends
/// stay recognisable: `1d958085aa2b…a3081f5d65`.
fn middle_ellipsis(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return name.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep.div_ceil(2);
    let tail = keep.saturating_sub(head);
    let start: String = chars.iter().take(head).collect();
    let end: String = chars
        .iter()
        .skip(chars.len().saturating_sub(tail))
        .collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::middle_ellipsis;

    #[test]
    fn middle_ellipsis_keeps_both_ends() {
        assert_eq!(middle_ellipsis("short", 10), "short");
        let hex = "1d958085aa2bf8105311d0a3081f5d65aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let out = middle_ellipsis(hex, 21);
        assert_eq!(out.chars().count(), 21);
        assert!(out.starts_with("1d958085aa"));
        assert!(out.ends_with("aaaaaaaaaa"));
    }
}

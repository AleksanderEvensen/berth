//! Volumes screen: "In use" / "Unused" sections with sizes, multi-select,
//! single and bulk delete, and remove-all-unused.

use std::rc::Rc;

use berth_docker::Volume;
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, SectionLabel, Status, Theme, empty_state, icons, status_dot,
};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, Render,
    SharedString, Window, div, list, prelude::*, px,
};

use super::{Chrome, detail_pane, list_column, scroll_page};
use crate::dialogs::ConfirmRequest;
use crate::format;
use crate::selection::MultiSelect;
use crate::store::{PruneTarget, Store};
use crate::virtual_rows::VirtualRows;

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
}

impl EventEmitter<ConfirmRequest> for VolumesScreen {}

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
        }
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
            cx.notify();
        }
    }

    fn sync_rows(&mut self, store: &Store) {
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
        self.rows.sync(rows);
        self.order = order;
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
        .action(
            IconButton::new(SharedString::from(format!("delete-{name}")), icons::TRASH)
                .tone(ButtonTone::Danger)
                .disabled(in_use)
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
            return scroll_page("volume-detail", volume_info(store, volume, theme))
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

impl Render for VolumesScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        self.sync_rows(store);
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
                None,
                self.render_detail(store, cx),
            ))
    }
}

fn volume_info(store: &Store, volume: &Volume, theme: &Theme) -> impl IntoElement {
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

    let used_by = div()
        .flex()
        .flex_col()
        .rounded(px(Theme::PANEL_RADIUS))
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .map(|d| {
            if users.is_empty() {
                d.child(
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .text_color(theme.text_faint)
                        .child("Not used by any container"),
                )
            } else {
                d.children(users.iter().enumerate().map(|(ix, c)| {
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_SM + 2.0))
                        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                        .child(status_dot(
                            if c.state.is_active() {
                                Status::Running
                            } else {
                                Status::Stopped
                            },
                            7.0,
                            None,
                            theme,
                        ))
                        .child(div().text_color(theme.text).child(c.name.clone()))
                }))
            }
        });

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
        .child(used_by)
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

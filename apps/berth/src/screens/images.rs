//! Images screen: "In use" / "Unused" / "Dangling" sections with size and
//! age, multi-select, delete, and remove-dangling.

use std::rc::Rc;

use berth_docker::Image;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    InUse,
    Unused,
    Dangling,
}

impl Group {
    const fn label(self) -> &'static str {
        match self {
            Self::InUse => "In use",
            Self::Unused => "Unused",
            Self::Dangling => "Dangling",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Section { group: Group, count: usize },
    Image(String),
}

pub struct ImagesScreen {
    store: Entity<Store>,
    rows: VirtualRows<Row>,
    order: Vec<String>,
    selection: MultiSelect<String>,
    chrome: Chrome,
}

impl EventEmitter<ConfirmRequest> for ImagesScreen {}

impl ImagesScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |this, store, cx| {
            let store = store.read(cx);
            this.selection
                .retain(|id| store.images().iter().any(|i| &i.id == id));
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

    fn group_of(store: &Store, image: &Image) -> Group {
        if store.image_in_use(image) {
            Group::InUse
        } else if image.is_dangling() {
            Group::Dangling
        } else {
            Group::Unused
        }
    }

    fn sync_rows(&mut self, store: &Store) {
        let mut groups: [(Group, Vec<&Image>); 3] = [
            (Group::InUse, Vec::new()),
            (Group::Unused, Vec::new()),
            (Group::Dangling, Vec::new()),
        ];
        for image in store.images() {
            let group = Self::group_of(store, image);
            if let Some((_, list)) = groups.iter_mut().find(|(g, _)| *g == group) {
                list.push(image);
            }
        }
        let mut rows = Vec::new();
        let mut order = Vec::new();
        for (group, mut images) in groups {
            if images.is_empty() {
                continue;
            }
            images.sort_by_key(|i| i.display_name());
            rows.push(Row::Section {
                group,
                count: images.len(),
            });
            for image in images {
                rows.push(Row::Image(image.id.clone()));
                order.push(image.id.clone());
            }
        }
        self.rows.sync(rows);
        self.order = order;
    }

    fn click(&mut self, id: &str, event: &ClickEvent, cx: &mut Context<Self>) {
        self.selection
            .click(&id.to_string(), event.modifiers(), &self.order);
        cx.notify();
    }

    fn confirm_remove(&self, images: Vec<(String, String)>, title: String, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let (ids, names): (Vec<String>, Vec<String>) = images.into_iter().unzip();
        cx.emit(ConfirmRequest {
            title: title.into(),
            message: "They can be pulled or rebuilt again later.".into(),
            items: names.into_iter().map(SharedString::from).collect(),
            confirm_label: "Delete".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| store.remove_images(ids.clone(), false, cx));
            }),
        });
    }

    fn render_row(&self, ix: usize, now: i64, cx: &Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let row = match self.rows.get(ix) {
            None => return div().into_any_element(),
            Some(Row::Section { group, count }) => SectionLabel::new(group.label())
                .count(*count)
                .into_any_element(),
            Some(Row::Image(id)) => {
                let Some(image) = store.images().iter().find(|i| &i.id == id) else {
                    return div().into_any_element();
                };
                self.image_row(store, image, now, cx).into_any_element()
            }
        };
        div()
            .px(px(Theme::SPACE_SM))
            .pb(px(2.0))
            .child(row)
            .into_any_element()
    }

    fn image_row(&self, store: &Store, image: &Image, now: i64, cx: &Context<Self>) -> ResourceRow {
        let id = image.id.clone();
        let name = image.display_name();
        let in_use = store.image_in_use(image);
        let delete = {
            let (id, name) = (id.clone(), name.clone());
            cx.listener(move |this, _, _, cx| {
                this.confirm_remove(
                    vec![(id.clone(), name.clone())],
                    format!("Delete {name}?"),
                    cx,
                );
            })
        };
        let click = {
            let id = id.clone();
            cx.listener(move |this, event: &ClickEvent, _, cx| this.click(&id, event, cx))
        };
        let badge = if image.is_dangling() {
            Badge::icon(id.clone(), icons::DISC).dimmed(true)
        } else {
            Badge::monogram(repo_name(&name))
        };
        ResourceRow::new(SharedString::from(format!("img-{id}")), name)
            .leading(badge)
            .subtitle(format!(
                "{}, {}",
                format::bytes(image.size),
                format::age(image.created, now)
            ))
            .busy(store.pending(&id).map(|_| "Deleting…"))
            .selected(self.selection.contains(&id))
            .action(
                IconButton::new(SharedString::from(format!("delete-{id}")), icons::TRASH)
                    .tone(ButtonTone::Danger)
                    .disabled(in_use)
                    .on_click(delete),
            )
            .on_click(click)
    }

    fn render_detail(&self, store: &Store, now: i64, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        if self.selection.is_empty() {
            return empty_state(
                "No selection",
                "Select an image to inspect it. Shift- or Ctrl-click to select several.",
                theme,
            )
            .into_any_element();
        }
        if let Some(id) = self.selection.single()
            && let Some(image) = store.images().iter().find(|i| &i.id == id)
        {
            return scroll_page("image-detail", image_info(store, image, now, theme))
                .into_any_element();
        }
        self.render_bulk(store, cx)
    }

    fn render_bulk(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let selected: Vec<&Image> = store
            .images()
            .iter()
            .filter(|i| self.selection.contains(&i.id))
            .collect();
        let size: u64 = selected.iter().map(|i| i.size).sum();
        let deletable: Vec<(String, String)> = selected
            .iter()
            .filter(|i| !store.image_in_use(i))
            .map(|i| (i.id.clone(), i.display_name()))
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
                            format!("Delete {count} image{}", plural(count)),
                        )
                        .icon(icons::TRASH)
                        .solid()
                        .tone(ButtonTone::Danger)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_remove(
                                deletable.clone(),
                                format!("Delete {count} image{}?", plural(count)),
                                cx,
                            );
                        })),
                    ),
                )
            })
            .into_any_element()
    }
}

impl Render for ImagesScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        self.sync_rows(store);
        let theme = Theme::of(cx);
        let now = format::now_unix();

        let total: u64 = store.images().iter().map(|i| i.size).sum();
        let dangling: Vec<(String, String)> = store
            .images()
            .iter()
            .filter(|i| i.is_dangling() && !store.image_in_use(i))
            .map(|i| (i.id.clone(), i.display_name()))
            .collect();
        let reclaim = store
            .reclaimable(PruneTarget::DanglingImages)
            .unwrap_or_default();
        let header = ListHeader::new("Images")
            .subtitle(format!("{} total", format::bytes(total)))
            .action(
                IconButton::new("remove-dangling", icons::BROOM)
                    .disabled(dangling.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let title = format!(
                            "Delete {} dangling image{} ({})?",
                            dangling.len(),
                            plural(dangling.len()),
                            format::bytes(reclaim)
                        );
                        this.confirm_remove(dangling.clone(), title, cx);
                    })),
            );

        let body = if self.rows.is_empty() {
            empty_state("No images", "Pulled and built images appear here.", theme)
                .into_any_element()
        } else {
            list(
                self.rows.state.clone(),
                cx.processor(move |this, ix: usize, _, cx| this.render_row(ix, now, cx)),
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
                "images-titlebar",
                header,
                body,
            ))
            .child(detail_pane(
                theme,
                self.chrome,
                "image-detail-titlebar",
                None,
                self.render_detail(store, now, cx),
            ))
    }
}

fn image_info(store: &Store, image: &Image, now: i64, theme: &Theme) -> impl IntoElement {
    let users = store.image_users(image);
    let tags = if image.repo_tags.is_empty() {
        Cell::muted("None (dangling)")
    } else {
        Cell::plain(image.repo_tags.join(", "))
    };
    let info = KeyValueCard::new()
        .row("ID", Cell::mono(image.short_id().to_string()))
        .row("Tags", tags)
        .row("Created", Cell::plain(format::age(image.created, now)))
        .row("Size", Cell::plain(format::bytes(image.size)));

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
            image
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

/// `ghcr.io/berth/api:dev` → `api` — the part worth a monogram.
fn repo_name(display: &str) -> String {
    let without_tag = display
        .rsplit_once(':')
        .filter(|(_, tag)| !tag.contains('/'))
        .map_or(display, |(name, _)| name);
    without_tag
        .rsplit('/')
        .next()
        .unwrap_or(without_tag)
        .to_string()
}

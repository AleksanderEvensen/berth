//! Images screen: "In use" / "Unused" / "Dangling" sections with size and
//! age, multi-select, delete, and remove-dangling.

use std::rc::Rc;

use berth_docker::{Image, ImageDetails};
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, SectionLabel, Theme, empty_state, icons,
};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, Render,
    SharedString, Task, Window, div, list, prelude::*, px,
};

use super::{Chrome, ShowContainer, detail_pane, list_column, scroll_page, used_by};
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

/// Inspect data for the single selected image.
#[derive(Debug)]
enum Details {
    Loading,
    Failed(String),
    Loaded(ImageDetails),
}

/// [`Details`] for one image, and the engine session that produced them.
#[derive(Debug)]
struct Fetched {
    id: String,
    generation: u64,
    details: Details,
}

/// Whether `id` needs (re)fetching. Image ids are content digests, so loaded
/// details never go stale; anything else is retried on a new engine session.
fn needs_fetch(cached: Option<&Fetched>, id: &str, generation: u64) -> bool {
    match cached {
        Some(f) if f.id == id => {
            !matches!(f.details, Details::Loaded(_)) && f.generation != generation
        }
        _ => true,
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
    /// Inspect details for the single selected image.
    details: Option<Fetched>,
    fetch: Option<Task<()>>,
}

impl EventEmitter<ConfirmRequest> for ImagesScreen {}
impl EventEmitter<ShowContainer> for ImagesScreen {}

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
            details: None,
            fetch: None,
        }
    }

    /// Fetch inspect details when the single selected image changes, or
    /// retry after a failure once a new engine session starts.
    fn sync_details(&mut self, cx: &Context<Self>) {
        let Some(id) = self.selection.single().cloned() else {
            self.details = None;
            self.fetch = None;
            return;
        };
        let store = self.store.read(cx);
        let generation = store.generation();
        if !needs_fetch(self.details.as_ref(), &id, generation) {
            return;
        }
        let Some(engine) = store.engine() else {
            self.details = Some(Fetched {
                id,
                generation,
                details: Details::Failed("Not connected".into()),
            });
            self.fetch = None;
            return;
        };
        self.details = Some(Fetched {
            id: id.clone(),
            generation,
            details: Details::Loading,
        });
        self.fetch = Some(cx.spawn(async move |this, cx| {
            let result = engine.inspect_image(&id).await;
            this.update(cx, |this, cx| {
                if let Some(fetched) = &mut this.details
                    && fetched.id == id
                    && fetched.generation == generation
                {
                    fetched.details =
                        result.map_or_else(|e| Details::Failed(e.to_string()), Details::Loaded);
                    cx.notify();
                }
            })
            .ok();
        }));
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
        // Clicking an image whose details failed to load retries the fetch.
        if self
            .details
            .as_ref()
            .is_some_and(|f| f.id == id && matches!(f.details, Details::Failed(_)))
        {
            self.details = None;
        }
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
            let details = self
                .details
                .as_ref()
                .filter(|fetched| &fetched.id == id)
                .map(|fetched| &fetched.details);
            return scroll_page(
                "image-detail",
                image_info(store, image, details, now, theme, cx),
            )
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
        self.sync_details(cx);
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

fn image_info(
    store: &Store,
    image: &Image,
    details: Option<&Details>,
    now: i64,
    theme: &Theme,
    cx: &Context<ImagesScreen>,
) -> impl IntoElement {
    let users = store.image_users(image);
    let tags = if image.repo_tags.is_empty() {
        Cell::muted("None (dangling)")
    } else {
        Cell::plain(image.repo_tags.join(", "))
    };
    let loaded = match details {
        Some(Details::Loaded(details)) => Some(details),
        _ => None,
    };
    let platform = match details {
        None | Some(Details::Loading) => Cell::muted("Loading…"),
        Some(Details::Failed(_)) => Cell::muted("Unknown"),
        Some(Details::Loaded(d)) => d
            .platform
            .clone()
            .map_or_else(|| Cell::muted("Unknown"), Cell::plain),
    };
    let info = KeyValueCard::new()
        .row("ID", Cell::mono(image.short_id().to_string()))
        .row("Tags", tags)
        .row("Created", Cell::plain(format::age(image.created, now)))
        .row("Size", Cell::plain(format::bytes(image.size)))
        .row("Platform", platform);

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
        .when_some(loaded, |d, details| d.child(image_config(details)))
        .when_some(
            match details {
                Some(Details::Failed(error)) => Some(error.clone()),
                _ => None,
            },
            |d, error| {
                d.child(
                    div()
                        .pt(px(Theme::SPACE_MD))
                        .text_size(px(Theme::TEXT_SM))
                        .text_color(theme.text_faint)
                        .child(format!("Couldn't load the image config: {error}")),
                )
            },
        )
        .child(Heading::new("Used by"))
        .child(used_by(&users, theme, cx))
        .child(Heading::new("Labels"))
        .child(labels)
}

/// Config, environment, volumes and exposed ports from inspect (D4).
fn image_config(details: &ImageDetails) -> impl IntoElement {
    let argv = |argv: &[String]| {
        if argv.is_empty() {
            Cell::muted("None")
        } else {
            Cell::mono(format::command(argv))
        }
    };
    let mut config = KeyValueCard::new()
        .row("Command", argv(&details.command))
        .row("Entrypoint", argv(&details.entrypoint))
        .row(
            "Working dir",
            details
                .working_dir
                .clone()
                .map_or_else(|| Cell::muted("/"), Cell::mono),
        );
    if let Some(user) = &details.user {
        config = config.row("User", Cell::plain(user.clone()));
    }

    let env = DataTable::new([("Key", 1.0), ("Value", 1.6)])
        .empty("No environment variables")
        .rows(details.env.iter().map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            vec![Cell::plain(key.to_string()), Cell::mono(value.to_string())]
        }));

    let volumes = DataTable::new([("Path", 1.0)])
        .empty("No volumes")
        .rows(details.volumes.iter().map(|v| vec![Cell::mono(v.clone())]));

    let ports = DataTable::new([("Port", 1.0), ("Protocol", 1.0)])
        .empty("No exposed ports")
        .rows(details.exposed_ports.iter().map(|p| {
            vec![
                Cell::plain(p.port.to_string()),
                Cell::muted(p.protocol.label()),
            ]
        }));

    div()
        .flex()
        .flex_col()
        .child(Heading::new("Config"))
        .child(config)
        .child(Heading::new("Environment"))
        .child(env)
        .child(Heading::new("Volumes"))
        .child(volumes)
        .child(Heading::new("Exposed ports"))
        .child(ports)
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

#[cfg(test)]
mod tests {
    use super::{Details, Fetched, ImageDetails, needs_fetch};

    fn fetched(details: Details, generation: u64) -> Fetched {
        Fetched {
            id: "a".into(),
            generation,
            details,
        }
    }

    #[test]
    fn failures_and_stale_loads_retry_on_a_new_session() {
        assert!(needs_fetch(None, "a", 1));
        let loaded = fetched(Details::Loaded(ImageDetails::default()), 1);
        assert!(!needs_fetch(Some(&loaded), "a", 1));
        assert!(!needs_fetch(Some(&loaded), "a", 2));
        assert!(needs_fetch(Some(&loaded), "b", 1));
        for details in [Details::Failed("Not connected".into()), Details::Loading] {
            let cached = fetched(details, 1);
            assert!(!needs_fetch(Some(&cached), "a", 1));
            assert!(needs_fetch(Some(&cached), "a", 2));
        }
    }
}

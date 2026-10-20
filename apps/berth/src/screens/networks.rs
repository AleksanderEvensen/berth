//! Networks screen (story E1): "In use" / "Unused" sections with driver and
//! subnet, multi-select, single and bulk delete, and remove-all-unused.

use std::rc::Rc;

use berth_docker::{Network, NetworkRole};
use berth_ui::{
    Badge, Button, ButtonTone, Cell, DataTable, Heading, IconButton, KeyValueCard, ListHeader,
    ResourceRow, SectionLabel, Status, Theme, empty_state, icons, status_dot,
};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement, Render,
    SharedString, Window, div, list, prelude::*, px,
};

use super::{Chrome, ListOp, ListScreen, detail_pane, list_column, scroll_page};
use crate::dialogs::ConfirmRequest;
use crate::selection::MultiSelect;
use crate::store::{PruneTarget, Store};
use crate::virtual_rows::VirtualRows;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Section {
        in_use: bool,
        count: usize,
    },
    /// Network ID.
    Network(String),
}

pub struct NetworksScreen {
    store: Entity<Store>,
    rows: VirtualRows<Row>,
    /// Selectable IDs in display order (for shift-ranges).
    order: Vec<String>,
    selection: MultiSelect<String>,
    chrome: Chrome,
}

impl EventEmitter<ConfirmRequest> for NetworksScreen {}

impl NetworksScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |this, store, cx| {
            let store = store.read(cx);
            this.selection
                .retain(|id| store.networks().iter().any(|n| &n.id == id));
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
        let (mut in_use, mut unused): (Vec<&Network>, Vec<&Network>) = store
            .networks()
            .iter()
            .partition(|n| Store::network_in_use(n));
        // Built-ins first, then by name.
        let key = |n: &&Network| (!store.network_builtin(n), n.name.clone());
        in_use.sort_by_key(key);
        unused.sort_by_key(key);

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
            for n in group {
                rows.push(Row::Network(n.id.clone()));
                order.push(n.id.clone());
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

    /// `networks` are `(id, name)` pairs.
    fn confirm_remove(
        &self,
        networks: Vec<(String, String)>,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        let (ids, names): (Vec<String>, Vec<String>) = networks.into_iter().unzip();
        cx.emit(ConfirmRequest {
            title: title.into(),
            message: "Compose recreates project networks on the next `up`.".into(),
            items: names.into_iter().map(SharedString::from).collect(),
            confirm_label: "Delete".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| store.remove_networks(ids.clone(), cx));
            }),
        });
    }

    /// Remove unused networks with the engine's prune, which knows which
    /// ones must survive; `names` is the preview.
    fn confirm_prune(&self, names: Vec<SharedString>, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let count = names.len();
        cx.emit(ConfirmRequest {
            title: format!("Delete {count} unused network{}?", plural(count)).into(),
            message: "Compose recreates project networks on the next `up`.".into(),
            items: names,
            confirm_label: "Delete".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| {
                    store.prune(vec![PruneTarget::UnusedNetworks], cx);
                });
            }),
        });
    }

    /// Delete the selected networks that can go (Delete key): not built in
    /// and not in use.
    fn delete_selected(&self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let networks: Vec<(String, String)> = store
            .networks()
            .iter()
            .filter(|n| {
                self.selection.contains(&n.id)
                    && !store.network_builtin(n)
                    && !Store::network_in_use(n)
            })
            .map(|n| (n.id.clone(), n.name.clone()))
            .collect();
        let title = match networks.as_slice() {
            [] => return,
            [(_, name)] => format!("Delete network {name}?"),
            _ => format!("Delete {} networks?", networks.len()),
        };
        self.confirm_remove(networks, title, cx);
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
            Some(Row::Network(id)) => {
                let Some(network) = store.networks().iter().find(|n| &n.id == id) else {
                    return div().into_any_element();
                };
                self.network_row(store, network, cx).into_any_element()
            }
        };
        div()
            .px(px(Theme::SPACE_SM))
            .pb(px(2.0))
            .child(row)
            .into_any_element()
    }

    fn network_row(&self, store: &Store, network: &Network, cx: &Context<Self>) -> ResourceRow {
        let id = network.id.clone();
        let removable = !store.network_builtin(network) && !Store::network_in_use(network);
        let delete = {
            let target = (id.clone(), network.name.clone());
            cx.listener(move |this, _, _, cx| {
                let title = format!("Delete network {}?", target.1);
                this.confirm_remove(vec![target.clone()], title, cx);
            })
        };
        let click = {
            let id = id.clone();
            cx.listener(move |this, event: &ClickEvent, _, cx| this.click(&id, event, cx))
        };
        let mut row = ResourceRow::new(
            SharedString::from(format!("net-{id}")),
            network.name.clone(),
        )
        .leading(Badge::icon(network.name.clone(), icons::NETWORK))
        .subtitle(summary(network))
        .busy(store.pending(&id).map(|_| "Deleting…"))
        .selected(self.selection.contains(&id))
        .keyboard_cursor(self.selection.is_cursor(&id))
        .action(
            IconButton::new(
                SharedString::from(format!("delete-{id}")),
                icons::TRASH,
                "Delete network",
            )
            .tone(ButtonTone::Danger)
            .disabled(!removable)
            .tab_stop(false)
            .on_click(delete),
        )
        .on_click(click);
        if !network.endpoints.is_empty() {
            row = row.meta(network.endpoints.len().to_string());
        }
        row
    }

    fn render_detail(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        if self.selection.is_empty() {
            return empty_state(
                "No selection",
                "Select a network to inspect it. Shift- or Ctrl-click to select several.",
                theme,
            )
            .into_any_element();
        }
        if let Some(id) = self.selection.single()
            && let Some(network) = store.networks().iter().find(|n| &n.id == id)
        {
            return scroll_page("network-detail", network_info(store, network, theme))
                .into_any_element();
        }
        self.render_bulk(store, cx)
    }

    fn render_bulk(&self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let selected: Vec<&Network> = store
            .networks()
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .collect();
        let deletable: Vec<(String, String)> = selected
            .iter()
            .filter(|n| !store.network_builtin(n) && !Store::network_in_use(n))
            .map(|n| (n.id.clone(), n.name.clone()))
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
            .when(skipped > 0, |d| {
                d.child(
                    div()
                        .text_size(px(Theme::TEXT_SM))
                        .text_color(theme.text_faint)
                        .child(format!("{skipped} in use or built-in will be kept")),
                )
            })
            .when(count > 0, |d| {
                d.child(
                    div().pt(px(Theme::SPACE_MD)).child(
                        Button::new(
                            "delete-selected",
                            format!("Delete {count} network{}", plural(count)),
                        )
                        .icon(icons::TRASH)
                        .solid()
                        .tone(ButtonTone::Danger)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_remove(
                                deletable.clone(),
                                format!("Delete {count} network{}?", plural(count)),
                                cx,
                            );
                        })),
                    ),
                )
            })
            .into_any_element()
    }
}

impl ListScreen for NetworksScreen {
    fn list_op(&mut self, op: ListOp, cx: &mut Context<Self>) {
        if op == ListOp::Delete {
            self.delete_selected(cx);
        } else if let Some(key) = op.apply(&mut self.selection, &self.order)
            && let Some(ix) = self
                .rows
                .position(|row| matches!(row, Row::Network(k) if *k == key))
        {
            self.rows.scroll_into_view(ix);
        }
        cx.notify();
    }
}

impl Render for NetworksScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        self.sync_rows(store);
        let theme = Theme::of(cx);

        let unused: Vec<SharedString> = store
            .unused_networks()
            .map(|n| n.name.clone().into())
            .collect();
        let total = store.networks().len();
        let header = ListHeader::new("Networks")
            .subtitle(format!("{total} network{}", plural(total)))
            .action(
                IconButton::new(
                    "remove-unused-networks",
                    icons::BROOM,
                    "Delete unused networks",
                )
                .disabled(unused.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.confirm_prune(unused.clone(), cx);
                })),
            );

        let body = if self.rows.is_empty() {
            let (title, detail) = if store.networks_loaded() {
                (
                    "No networks",
                    "Bridge, Compose and custom networks appear here.",
                )
            } else {
                ("Loading networks…", "Asking the engine for its networks.")
            };
            empty_state(title, detail, theme).into_any_element()
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
                "networks-titlebar",
                header,
                body,
            ))
            .child(detail_pane(
                theme,
                self.chrome,
                "network-detail-titlebar",
                None,
                self.render_detail(store, cx),
            ))
    }
}

/// Row subtitle: `bridge · 172.19.0.0/16`.
fn summary(network: &Network) -> String {
    network.subnets.first().map_or_else(
        || network.driver.clone(),
        |s| format!("{} · {}", network.driver, s.subnet),
    )
}

fn yes_no(value: bool) -> Cell {
    if value {
        Cell::plain("Yes")
    } else {
        Cell::muted("No")
    }
}

fn network_info(store: &Store, network: &Network, theme: &Theme) -> impl IntoElement {
    let mut info = KeyValueCard::new()
        .row("Name", Cell::plain(network.name.clone()))
        .row("ID", Cell::mono(network.short_id().to_string()))
        .row("Driver", Cell::plain(network.driver.clone()))
        .row("Scope", Cell::plain(network.scope.clone()));
    if let Some(project) = network.compose_project() {
        info = info.row("Compose project", Cell::plain(project.to_string()));
    }
    if let Some(created) = &network.created {
        info = info.row("Created", Cell::plain(created.clone()));
    }

    let mut addressing = KeyValueCard::new();
    if network.subnets.is_empty() {
        addressing = addressing.row("Subnet", Cell::muted("None"));
    }
    for subnet in &network.subnets {
        addressing = addressing
            .row("Subnet", Cell::mono(subnet.subnet.clone()))
            .row(
                "Gateway",
                subnet
                    .gateway
                    .clone()
                    .map_or_else(|| Cell::muted("None"), Cell::mono),
            );
    }
    addressing = addressing
        .row("IPv6", yes_no(network.ipv6))
        .row("Internal", yes_no(network.internal))
        .row("Attachable", yes_no(network.attachable));
    match network.role {
        NetworkRole::Regular => {}
        NetworkRole::Ingress => addressing = addressing.row("Role", Cell::plain("Swarm ingress")),
        NetworkRole::ConfigOnly => addressing = addressing.row("Role", Cell::plain("Config only")),
    }

    let containers = connected_containers(store, network, theme);

    let labels = DataTable::new([("Key", 1.0), ("Value", 1.2)])
        .empty("No labels")
        .rows(
            network
                .labels
                .iter()
                .map(|(k, v)| vec![Cell::plain(k.clone()), Cell::plain(v.clone())]),
        );

    div()
        .flex()
        .flex_col()
        .child(info)
        .child(Heading::new("Addressing"))
        .child(addressing)
        .child(Heading::new("Containers"))
        .child(containers)
        .child(Heading::new("Labels"))
        .child(labels)
}

/// The containers with an endpoint on `network`, with their addresses.
fn connected_containers(store: &Store, network: &Network, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .rounded(px(Theme::PANEL_RADIUS))
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .map(|d| {
            if network.endpoints.is_empty() {
                d.child(
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .text_color(theme.text_faint)
                        .child(if network.is_swarm() {
                            "No local containers. Swarm services on other nodes may use it."
                        } else {
                            "No running containers are connected"
                        }),
                )
            } else {
                d.children(network.endpoints.iter().enumerate().map(|(ix, e)| {
                    // Endpoints exist only for running containers; the store
                    // may briefly lag behind, so fall back to "running".
                    let status = store
                        .containers()
                        .iter()
                        .find(|c| c.id == e.container_id)
                        .map_or(Status::Running, |c| {
                            if c.state.is_active() {
                                Status::Running
                            } else {
                                Status::Stopped
                            }
                        });
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_SM + 2.0))
                        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                        .child(status_dot(status, 7.0, None, theme))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.text)
                                .child(e.name.clone()),
                        )
                        .children(e.ipv4.clone().or_else(|| e.ipv6.clone()).map(|ip| {
                            div()
                                .flex_none()
                                .font_family(theme.font_mono.clone())
                                .text_size(px(Theme::TEXT_SM))
                                .text_color(theme.text_faint)
                                .child(ip)
                        }))
                }))
            }
        })
}

const fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn summary_shows_driver_and_first_subnet() {
        let networks = berth_docker::fixtures::networks();
        let find = |name: &str| networks.iter().find(|n| n.name == name).unwrap();
        assert_eq!(summary(find("bridge")), "bridge · 172.17.0.0/16");
        assert_eq!(summary(find("none")), "null");
    }
}

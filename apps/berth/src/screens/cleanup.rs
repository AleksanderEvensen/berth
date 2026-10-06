//! Clean-up page (story G1): pick categories, see what's reclaimable, prune.

use std::collections::HashSet;
use std::rc::Rc;

use berth_ui::{Button, ButtonTone, Checkbox, ListHeader, Spinner, Theme, icons};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, IntoElement, Render, SharedString,
    Window, div, prelude::*, px,
};

use super::Chrome;
use crate::chrome;
use crate::dialogs::ConfirmRequest;
use crate::format;
use crate::store::{PruneTarget, Store};

pub struct CleanupScreen {
    store: Entity<Store>,
    selected: HashSet<PruneTarget>,
    chrome: Chrome,
}

impl EventEmitter<ConfirmRequest> for CleanupScreen {}

const fn describe(target: PruneTarget) -> &'static str {
    match target {
        PruneTarget::StoppedContainers => {
            "Containers that aren't running. Their writable layers are deleted."
        }
        PruneTarget::DanglingImages => "Untagged layers left behind by rebuilds and pulls.",
        PruneTarget::UnusedImages => {
            "Every image no container uses, including tagged ones. They can be pulled again."
        }
        PruneTarget::UnusedVolumes => {
            "Volumes no container references. Their data is deleted permanently."
        }
        PruneTarget::UnusedNetworks => "Custom networks with no connected containers.",
        PruneTarget::BuildCache => "BuildKit cache that no build is using.",
    }
}

impl CleanupScreen {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            // Safe defaults: nothing a user could miss.
            selected: [PruneTarget::DanglingImages, PruneTarget::BuildCache]
                .into_iter()
                .collect(),
            chrome: Chrome::default(),
        }
    }

    pub fn set_chrome(&mut self, chrome: Chrome, cx: &mut Context<Self>) {
        if self.chrome != chrome {
            self.chrome = chrome;
            cx.notify();
        }
    }

    fn toggle(&mut self, target: PruneTarget, cx: &mut Context<Self>) {
        if !self.selected.remove(&target) {
            self.selected.insert(target);
        }
        cx.notify();
    }

    /// Selected targets in display order.
    fn targets(&self) -> Vec<PruneTarget> {
        PruneTarget::ALL
            .into_iter()
            .filter(|t| self.selected.contains(t))
            .collect()
    }

    fn confirm(&self, total: Option<u64>, cx: &mut Context<Self>) {
        let targets = self.targets();
        let store = self.store.clone();
        let title = total.map_or_else(
            || "Clean up?".to_string(),
            |b| format!("Clean up {}?", format::bytes(b)),
        );
        let destroys_data = targets.contains(&PruneTarget::UnusedVolumes);
        cx.emit(ConfirmRequest {
            title: title.into(),
            message: if destroys_data {
                "Unused volumes are included — their data is deleted permanently.".into()
            } else {
                "This removes the following:".into()
            },
            items: targets
                .iter()
                .map(|t| SharedString::from(t.label()))
                .collect(),
            confirm_label: "Clean up".into(),
            on_confirm: Rc::new(move |_, cx| {
                store.update(cx, |store, cx| store.prune(targets.clone(), cx));
            }),
        });
    }

    fn render_action(
        &self,
        pruning: bool,
        targets: &[PruneTarget],
        total: Option<u64>,
        known_total: u64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx);
        if pruning {
            div()
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_SM))
                .text_color(theme.text_muted)
                .child(Spinner::new("pruning"))
                .child("Cleaning up…")
                .into_any_element()
        } else if targets.is_empty() {
            div()
                .text_color(theme.text_faint)
                .child("Select something to clean up")
                .into_any_element()
        } else {
            let label = total
                .or_else(|| (known_total > 0).then_some(known_total))
                .map_or_else(
                    || "Clean up".to_string(),
                    |b| format!("Clean up {}", format::bytes(b)),
                );
            Button::new("run-cleanup", label)
                .icon(icons::BROOM)
                .solid()
                .tone(if self.selected.contains(&PruneTarget::UnusedVolumes) {
                    ButtonTone::Danger
                } else {
                    ButtonTone::Default
                })
                .on_click(cx.listener(move |this, _, _, cx| this.confirm(total, cx)))
                .into_any_element()
        }
    }

    fn category(&self, store: &Store, target: PruneTarget, cx: &Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let checked = self.selected.contains(&target);
        let count = store.prune_count(target);
        let size = store.reclaimable(target);
        let metric = match (count, size) {
            (Some(0), _) => "Nothing to remove".to_string(),
            (Some(n), Some(b)) => format!("{n} · {}", format::bytes(b)),
            (Some(n), None) => format!("{n} items"),
            (None, Some(b)) => format::bytes(b),
            (None, None) => "—".to_string(),
        };
        let row_hover = theme.row_hover;
        div()
            .id(SharedString::from(format!("cleanup-{target:?}")))
            .min_h(px(60.0))
            .px(px(Theme::SPACE_MD + 2.0))
            .py(px(Theme::SPACE_MD))
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_MD + 2.0))
            .cursor_pointer()
            .hover(move |s| s.bg(row_hover))
            .child(
                Checkbox::new(SharedString::from(format!("check-{target:?}")), checked)
                    .on_toggle(cx.listener(move |this, _, _, cx| this.toggle(target, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_MD + 0.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if target == PruneTarget::UnusedVolumes {
                                theme.danger
                            } else {
                                theme.text
                            })
                            .child(target.label()),
                    )
                    .child(
                        div()
                            .text_size(px(Theme::TEXT_SM))
                            .text_color(theme.text_faint)
                            .child(describe(target)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(Theme::TEXT_SM))
                    .text_color(theme.text_muted)
                    .child(metric),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(target, cx)))
            .into_any_element()
    }
}

impl Render for CleanupScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let theme = Theme::of(cx);
        let pruning = store.pruning();

        let targets = self.targets();
        // Unused images already include the dangling ones — don't double count.
        let total: Option<u64> = targets
            .iter()
            .filter(|t| {
                !(**t == PruneTarget::DanglingImages
                    && self.selected.contains(&PruneTarget::UnusedImages))
            })
            .map(|t| store.reclaimable(*t))
            .try_fold(0u64, |acc, b| b.map(|b| acc.saturating_add(b)));
        let known_total: u64 = targets.iter().filter_map(|t| store.reclaimable(*t)).sum();
        let subtitle = if store.usage_known() {
            "Reclaim disk space from things you no longer use"
        } else {
            "Measuring disk usage…"
        };

        let categories = div()
            .flex()
            .flex_col()
            .rounded(px(Theme::PANEL_RADIUS))
            .bg(theme.surface_card)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .children(
                PruneTarget::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(ix, target)| {
                        div()
                            .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                            .child(self.category(store, target, cx))
                    }),
            );

        let action = self.render_action(pruning, &targets, total, known_total, cx);

        let page = div()
            .id("cleanup-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(720.0))
                    .mx_auto()
                    .px(px(Theme::SPACE_LG + 8.0))
                    .pt(px(Theme::SPACE_SM))
                    .pb(px(Theme::SPACE_LG * 2.0))
                    .flex()
                    .flex_col()
                    .gap(px(Theme::SPACE_LG))
                    .child(categories)
                    .child(div().flex().justify_end().child(action)),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .rounded_l(px(self.chrome.left_radius))
            .rounded_r(px(self.chrome.right_radius))
            .child(chrome::drag_region(
                "cleanup-titlebar",
                div().child(
                    ListHeader::new("Clean up")
                        .subtitle(subtitle)
                        .leading_inset(self.chrome.header_inset),
                ),
            ))
            .child(page)
    }
}

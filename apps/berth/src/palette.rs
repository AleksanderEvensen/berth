//! The command palette (A3): run any command in the keyboard map, or jump to
//! a container, image or volume, picked by a fuzzy match on its name.

use std::ops::Range;

use berth_ui::{Badge, PANEL_SPRING, ResourceRow, SearchField, Status, Theme, empty_state, icons};
use gpui::{
    AnimationExt as _, AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    IntoElement, Render, ScrollStrategy, SharedString, SpringAnimation, Subscription,
    UniformListScrollHandle, Window, deferred, div, prelude::*, px, uniform_list,
};
use gpui_base::input::{InputEvent, InputState};

use crate::keymap::{
    MenuCancel, MenuConfirm, MenuNext, MenuPrev, PALETTE_CONTEXT, SHORTCUTS, Shortcut,
};
use crate::screens::{
    containers::{image_name, menu_step},
    settings::keys,
};
use crate::store::Store;

/// Results shown before the list scrolls.
const VISIBLE_RESULTS: usize = 8;
/// A result row and the gap below it.
const RESULT_HEIGHT: f32 = Theme::RESOURCE_ROW_HEIGHT + 2.0;

/// What a result runs or opens.
#[derive(Clone)]
pub enum Target {
    Command(&'static Shortcut),
    Container(String),
    Image(String),
    Volume(String),
}

/// What the palette asks of the shell, which owns it.
pub enum PaletteEvent {
    Run(Target),
    Dismiss,
}

struct Entry {
    target: Target,
    name: SharedString,
    detail: SharedString,
}

pub struct Palette {
    store: Entity<Store>,
    input: Entity<InputState>,
    entries: Vec<Entry>,
    /// Indices into `entries` matching the query, best first.
    matches: Vec<usize>,
    /// Index into `matches` that Enter runs.
    highlight: usize,
    scroll: UniformListScrollHandle,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Palette {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Run a command or go to a container, image or volume")
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscriptions = [
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.highlight = 0;
                    this.refilter(cx);
                }
            }),
            // Containers come and go while the palette is open.
            cx.observe(&store, |this, _, cx| {
                this.entries = entries(this.store.read(cx));
                this.refilter(cx);
            }),
        ];
        let entries = entries(store.read(cx));
        let matches = (0..entries.len()).collect();
        Self {
            store,
            input,
            entries,
            matches,
            highlight: 0,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The text field, which holds focus while the palette is open.
    pub fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value();
        self.matches = rank(&query, self.entries.iter().map(|e| e.name.as_ref()));
        self.highlight = self.highlight.min(self.matches.len().saturating_sub(1));
        self.scroll
            .scroll_to_item(self.highlight, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Move the highlight, wrapping around at either end.
    fn step(&mut self, back: bool, cx: &mut Context<Self>) {
        if let Some(ix) = menu_step(Some(self.highlight), self.matches.len(), back) {
            self.highlight = ix;
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn run(&self, ix: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.matches.get(ix).and_then(|&e| self.entries.get(e)) {
            cx.emit(PaletteEvent::Run(entry.target.clone()));
        }
    }

    fn render_rows(&self, range: Range<usize>, cx: &Context<Self>) -> Vec<AnyElement> {
        let theme = Theme::of(cx);
        let store = self.store.read(cx);
        range
            .filter_map(|ix| {
                let entry = self.entries.get(*self.matches.get(ix)?)?;
                let row = ResourceRow::new(("palette-result", ix), entry.name.clone())
                    .subtitle(entry.detail.clone())
                    .selected(ix == self.highlight)
                    .on_click(cx.listener(move |this, _, _, cx| this.run(ix, cx)));
                let row = match &entry.target {
                    Target::Command(shortcut) => row
                        .leading(Badge::icon(shortcut.group.label(), icons::KEYBOARD))
                        .trailing(keys(shortcut.keys, theme)),
                    Target::Container(id) => {
                        let active = store
                            .containers()
                            .iter()
                            .find(|c| &c.id == id)
                            .is_some_and(|c| c.state.is_active());
                        // The detail line is the container's image.
                        row.leading(
                            Badge::monogram(image_name(&entry.detail)).status(if active {
                                Status::Running
                            } else {
                                Status::Stopped
                            }),
                        )
                        .meta("Container")
                    }
                    Target::Image(id) => row
                        .leading(Badge::icon(id.clone(), icons::DISC))
                        .meta("Image"),
                    Target::Volume(name) => row
                        .leading(Badge::icon(name.clone(), icons::HARD_DRIVE))
                        .meta("Volume"),
                };
                Some(div().pb(px(2.0)).child(row).into_any_element())
            })
            .collect()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let shown = self.matches.len().min(VISIBLE_RESULTS);
        let results = if self.matches.is_empty() {
            div()
                .h(px(RESULT_HEIGHT * 3.0))
                .child(empty_state(
                    "No matches",
                    "No command, container, image or volume has that name.",
                    theme,
                ))
                .into_any_element()
        } else {
            uniform_list(
                "palette-results",
                self.matches.len(),
                cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
            )
            .track_scroll(&self.scroll)
            .h(px(
                RESULT_HEIGHT * f32::from(u8::try_from(shown).unwrap_or(u8::MAX))
            ))
            .into_any_element()
        };
        let card = div()
            .id("palette")
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(|this, _: &MenuPrev, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &MenuNext, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &MenuConfirm, _, cx| this.run(this.highlight, cx)))
            .on_action(cx.listener(|_, _: &MenuCancel, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .w(px(580.0))
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_SM))
            .p(px(Theme::SPACE_SM))
            .rounded(px(14.0))
            .bg(theme.surface_dialog)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_2xl()
            .child(SearchField::new(&self.input))
            .child(results);
        let card = if theme.reduce_motion {
            card.into_any_element()
        } else {
            // Drops in from just above its place.
            card.with_spring(
                "palette-in",
                SpringAnimation::new(PANEL_SPRING).to(1.0).from(0.0),
                |card, t: f32| card.opacity(t).mt(px((t - 1.0) * 12.0)),
            )
            .into_any_element()
        };
        deferred(
            div()
                .id("palette-overlay")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(Theme::HEADER_HEIGHT + 56.0))
                .bg(theme.scrim)
                .occlude()
                .child(card),
        )
        .with_priority(1)
    }
}

/// Every command in the keyboard map, then containers, images and volumes
/// by name.
fn entries(store: &Store) -> Vec<Entry> {
    let commands = SHORTCUTS
        .iter()
        .filter(|s| s.command().is_some())
        .map(|s| Entry {
            target: Target::Command(s),
            name: s.label.into(),
            detail: s.group.label().into(),
        });
    let mut containers: Vec<Entry> = store
        .containers()
        .iter()
        .map(|c| Entry {
            target: Target::Container(c.id.clone()),
            name: c.name.clone().into(),
            detail: c.image.clone().into(),
        })
        .collect();
    let mut images: Vec<Entry> = store
        .images()
        .iter()
        .map(|i| Entry {
            target: Target::Image(i.id.clone()),
            name: i.display_name().into(),
            detail: i.short_id().to_string().into(),
        })
        .collect();
    let mut volumes: Vec<Entry> = store
        .volumes()
        .iter()
        .map(|v| Entry {
            target: Target::Volume(v.name.clone()),
            name: v.name.clone().into(),
            detail: v.driver.clone().into(),
        })
        .collect();
    for list in [&mut containers, &mut images, &mut volumes] {
        list.sort_by(|a, b| a.name.cmp(&b.name));
    }
    commands
        .chain(containers)
        .chain(images)
        .chain(volumes)
        .collect()
}

/// Points per matched character.
const MATCH: i32 = 16;
/// Extra for a character starting a word, or following the one before.
const WORD_START: i32 = 10;
const CONSECUTIVE: i32 = 8;
/// Cost of skipping characters between two matched ones: to open the gap,
/// and per character.
const GAP_START: i32 = 3;
const GAP: i32 = 1;
/// Most that skipped characters before the first match can cost.
const LEADING_FLOOR: i32 = -3;

/// How well `query` matches `text`: all of its characters, in order,
/// ignoring case and spaces in the query. `None` without a match. Higher is
/// better: characters that start words or follow each other score more,
/// gaps between them less. Takes the best of every way to match.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let mut previous = ' ';
    let text: Vec<(char, bool)> = text
        .chars()
        .map(|c| {
            let after_separator = !previous.is_alphanumeric() && c.is_alphanumeric();
            let camel_hump = previous.is_lowercase() && c.is_uppercase();
            let starts_word = after_separator || camel_hump;
            previous = c;
            (c.to_lowercase().next().unwrap_or(c), starts_word)
        })
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    // best[j]: the best score with the query so far matched, its last
    // character at text position j.
    let mut best: Vec<Option<i32>> = Vec::new();
    for (i, &q) in query.iter().enumerate() {
        let first = i == 0;
        // The best score of an earlier match, less the gap since.
        let mut gapped = first.then_some(0);
        // The previous character's score at the position just before.
        let mut diagonal: Option<i32> = None;
        let mut row = Vec::with_capacity(text.len());
        for (j, &(c, starts_word)) in text.iter().enumerate() {
            let above = if first {
                None
            } else {
                best.get(j).copied().flatten()
            };
            row.push(
                (c == q)
                    .then(|| {
                        let bonus = if starts_word { WORD_START } else { 0 };
                        let consecutive = diagonal.map(|d| d.saturating_add(CONSECUTIVE));
                        gapped
                            .max(consecutive)
                            .map(|s| s.saturating_add(MATCH).saturating_add(bonus))
                    })
                    .flatten(),
            );
            gapped = if first {
                gapped.map(|g| g.saturating_sub(GAP).max(LEADING_FLOOR))
            } else {
                gapped
                    .map(|g| g.saturating_sub(GAP))
                    .max(above.map(|a| a.saturating_sub(GAP_START)))
            };
            diagonal = above;
        }
        best = row;
    }
    best.into_iter().flatten().max()
}

/// Indices of the `names` matching `query`, best first; equal scores keep
/// their order. An empty query keeps every name.
pub fn rank<'a>(query: &str, names: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut scored: Vec<(usize, i32)> = names
        .into_iter()
        .enumerate()
        .filter_map(|(ix, name)| Some((ix, score(query, name)?)))
        .collect();
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
    scored.into_iter().map(|(ix, _)| ix).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked<'a>(query: &str, names: &[&'a str]) -> Vec<&'a str> {
        rank(query, names.iter().copied())
            .into_iter()
            .filter_map(|ix| names.get(ix).copied())
            .collect()
    }

    #[test]
    fn matches_characters_in_order_ignoring_case() {
        assert!(score("pg", "postgres").is_some());
        assert!(score("PoStG", "postgres").is_some());
        assert!(score("pgx", "postgres").is_none());
        assert!(score("gp", "postgres").is_none());
        assert!(score("go im", "Go to Images").is_some());
        assert!(score("a", "").is_none());
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("  ", "anything"), Some(0));
    }

    #[test]
    fn word_starts_and_runs_beat_scattered_matches() {
        // A run beats the same characters spread out.
        assert!(score("red", "redis") > score("red", "rabbit-edge-d"));
        // Word starts, camelCase humps included, beat the middle of a word.
        assert!(score("b", "a b") > score("b", "aab"));
        assert!(score("s", "mailSink") > score("s", "mailsink"));
        // Fewer skipped characters is better, before and between matches.
        assert!(score("api", "api") > score("api", "my-api"));
        assert!(score("ab", "a-b") > score("ab", "a---b"));
    }

    #[test]
    fn takes_the_best_alignment_not_the_first() {
        // Greedy matching would pair the first `s` with the `t` of "sat".
        assert!(score("st", "sat Stop") > score("st", "sat"));
    }

    #[test]
    fn ranks_best_first() {
        let names = [
            "Go to Images",
            "imgproxy",
            "minio",
            "Start or stop",
            "Settings",
        ];
        assert_eq!(ranked("img", &names), ["imgproxy", "Go to Images"]);
        assert_eq!(ranked("st", &names), ["Start or stop", "Settings"]);
        assert_eq!(ranked("min", &names), ["minio"]);
        assert_eq!(ranked("zzz", &names), [""; 0]);
    }

    #[test]
    fn empty_query_and_ties_keep_order() {
        let names = ["b", "a", "c"];
        assert_eq!(ranked("", &names), names);
        assert_eq!(
            ranked("db", &["db-1", "db-2", "db-3"]),
            ["db-1", "db-2", "db-3"]
        );
    }
}

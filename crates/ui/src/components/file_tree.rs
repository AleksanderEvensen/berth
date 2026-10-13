//! A stateless, virtualized file tree table (Name · Date Modified · Size ·
//! Kind). The owner flattens its tree into [`FileTreeRow`]s and reacts to
//! the select / toggle / open callbacks; nothing is kept here.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, FontWeight, IntoElement, KeyDownEvent,
    RenderOnce, ScrollStrategy, SharedString, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};

use super::{Spinner, icon};
use crate::{Theme, icons};

/// Key context of a focused tree; the app's list keys stand aside for it.
pub const FILE_TREE_CONTEXT: &str = "FileTree";

const ROW_HEIGHT: f32 = 28.0;
const INDENT: f32 = 16.0;
const DATE_WIDTH: f32 = 168.0;
const SIZE_WIDTH: f32 = 84.0;
const KIND_WIDTH: f32 = 72.0;
const TAG_MAX_WIDTH: f32 = 240.0;

type PathHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App) + 'static>;

/// A folder's disclosure state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    Collapsed,
    Expanded,
    /// Expanded, its entries still being listed.
    Loading,
}

/// A file or folder line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeEntry {
    /// Unique among rows (the full path); handed back in callbacks.
    pub key: SharedString,
    pub name: SharedString,
    pub depth: usize,
    /// `None` for anything that can't be expanded.
    pub disclosure: Option<Disclosure>,
    pub symlink: bool,
    pub modified: SharedString,
    pub size: SharedString,
    pub kind: SharedString,
    /// A short note after the name, e.g. the volume mounted there.
    pub tag: Option<SharedString>,
}

/// One line of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTreeRow {
    Entry(FileTreeEntry),
    /// A message in place of a folder's entries ("Empty folder", an error).
    Note {
        key: SharedString,
        depth: usize,
        text: SharedString,
        error: bool,
    },
}

impl FileTreeRow {
    #[must_use]
    pub const fn key(&self) -> &SharedString {
        match self {
            Self::Entry(entry) => &entry.key,
            Self::Note { key, .. } => key,
        }
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        match self {
            Self::Entry(entry) => entry.depth,
            Self::Note { depth, .. } => *depth,
        }
    }

    const fn entry(&self) -> Option<&FileTreeEntry> {
        match self {
            Self::Entry(entry) => Some(entry),
            Self::Note { .. } => None,
        }
    }
}

/// What a navigation key asks of the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyIntent {
    Select(usize),
    Toggle(usize),
    Open(usize),
}

#[derive(IntoElement)]
#[must_use]
pub struct FileTree {
    id: ElementId,
    rows: Rc<[FileTreeRow]>,
    selected: Option<SharedString>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    on_select: Option<PathHandler>,
    on_toggle: Option<PathHandler>,
    on_open: Option<PathHandler>,
}

impl FileTree {
    /// `focus` and `scroll` belong to the owner so they survive re-renders.
    pub fn new(
        id: impl Into<ElementId>,
        rows: Rc<[FileTreeRow]>,
        focus: &FocusHandle,
        scroll: &UniformListScrollHandle,
    ) -> Self {
        Self {
            id: id.into(),
            rows,
            selected: None,
            focus: focus.clone(),
            scroll: scroll.clone(),
            on_select: None,
            on_toggle: None,
            on_open: None,
        }
    }

    pub fn selected(mut self, key: Option<SharedString>) -> Self {
        self.selected = key;
        self
    }

    /// A row was clicked or reached with the arrow keys.
    pub fn on_select(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// A folder's chevron was clicked, it was double-clicked, or ←/→/Enter.
    pub fn on_toggle(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }

    /// A file was double-clicked or Enter was pressed on it.
    pub fn on_open(mut self, f: impl Fn(&SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for FileTree {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let ring = theme.accent;
        let rows = self.rows.clone();
        let selected = self.selected.clone();
        let handlers = Handlers {
            select: self.on_select.clone(),
            toggle: self.on_toggle.clone(),
            open: self.on_open.clone(),
            focus: self.focus.clone(),
        };
        let keys = {
            let rows = self.rows.clone();
            let selected = self.selected.clone();
            let handlers = handlers.clone();
            let scroll = self.scroll.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                let current = selected
                    .as_ref()
                    .and_then(|key| rows.iter().position(|row| row.key() == key));
                let Some(intent) = key_intent(&rows, current, &event.keystroke.key) else {
                    return;
                };
                cx.stop_propagation();
                let ix = match intent {
                    KeyIntent::Select(ix) | KeyIntent::Toggle(ix) | KeyIntent::Open(ix) => ix,
                };
                let Some(key) = rows.get(ix).map(|row| row.key().clone()) else {
                    return;
                };
                scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
                let handler = match intent {
                    KeyIntent::Select(_) => &handlers.select,
                    KeyIntent::Toggle(_) => &handlers.toggle,
                    KeyIntent::Open(_) => &handlers.open,
                };
                if let Some(handler) = handler {
                    handler(&key, window, cx);
                }
            }
        };
        let count = rows.len();
        div()
            .id(self.id.clone())
            .key_context(FILE_TREE_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(keys)
            .size_full()
            .flex()
            .flex_col()
            .border_1()
            .border_color(gpui::transparent_black())
            .rounded(px(Theme::CONTROL_RADIUS))
            .focus_visible(move |s| s.border_color(ring))
            .text_size(px(Theme::TEXT_MD))
            .child(header(theme))
            .child(
                uniform_list(
                    SharedString::from(format!("{}-rows", self.id)),
                    count,
                    move |range, _, cx| {
                        let theme = Theme::of(cx);
                        range
                            .filter_map(|ix| {
                                let row = rows.get(ix)?;
                                let is_selected = selected.as_ref() == Some(row.key());
                                Some(render_row(row, ix, is_selected, &handlers, theme))
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            )
    }
}

#[derive(Clone)]
struct Handlers {
    select: Option<PathHandler>,
    toggle: Option<PathHandler>,
    open: Option<PathHandler>,
    focus: FocusHandle,
}

fn header(theme: &Theme) -> impl IntoElement {
    let column = |label: &'static str, width: Option<f32>| {
        div()
            .px(px(Theme::SPACE_SM))
            .map(|d| match width {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1().min_w_0(),
            })
            .child(label)
    };
    div()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .text_size(px(Theme::TEXT_SM))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text_muted)
        .child(column("Name", None).pl(px(Theme::SPACE_SM + 20.0)))
        .child(column("Date Modified", Some(DATE_WIDTH)))
        .child(column("Size", Some(SIZE_WIDTH)).flex().justify_end())
        .child(column("Kind", Some(KIND_WIDTH)))
}

fn render_row(
    row: &FileTreeRow,
    ix: usize,
    selected: bool,
    handlers: &Handlers,
    theme: &Theme,
) -> AnyElement {
    let indent = f32::from(u16::try_from(row.depth()).unwrap_or(u16::MAX)) * INDENT;
    // Full width: list items are otherwise sized to their content, and the
    // columns wouldn't line up under the header.
    let base = div()
        .id(ix)
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .rounded(px(Theme::CONTROL_RADIUS));
    let entry = match row {
        FileTreeRow::Note { text, error, .. } => {
            return base
                .pl(px(indent + Theme::SPACE_SM + 20.0))
                .text_color(if *error {
                    theme.danger
                } else {
                    theme.text_faint
                })
                .child(div().min_w_0().truncate().child(text.clone()))
                .into_any_element();
        }
        FileTreeRow::Entry(entry) => entry,
    };
    let row_hover = theme.row_hover;
    let chevron = chevron(entry, handlers, theme);
    let glyph = icon(if entry.disclosure.is_some() {
        icons::FOLDER
    } else {
        icons::FILE
    })
    .size(px(15.0))
    .text_color(if entry.disclosure.is_some() {
        theme.accent
    } else {
        theme.text_muted
    });
    let click = click_handler(entry, handlers);
    let cell = |text: &SharedString, width: f32| {
        div()
            .w(px(width))
            .flex_none()
            .px(px(Theme::SPACE_SM))
            .truncate()
            .text_color(theme.text_muted)
            .child(text.clone())
    };
    base.cursor_pointer()
        .map(|d| {
            if selected {
                d.bg(theme.row_selected)
            } else {
                d.hover(move |s| s.bg(row_hover))
            }
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_XS + 2.0))
                .pl(px(indent + Theme::SPACE_XS))
                .child(div().w(px(16.0)).flex_none().children(chevron))
                .child(glyph)
                .child(
                    // The space left after the icons: the name keeps its
                    // width up to all of it, so a tag truncates first.
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_XS + 2.0))
                        .child(
                            div()
                                .flex_shrink_0()
                                .max_w_full()
                                .truncate()
                                .text_color(theme.text)
                                .when(entry.symlink, gpui::Styled::italic)
                                .child(entry.name.clone()),
                        )
                        .children(entry.tag.clone().map(|tag| {
                            div()
                                .min_w_0()
                                .max_w(px(TAG_MAX_WIDTH))
                                .truncate()
                                .px(px(6.0))
                                .rounded(px(4.0))
                                .bg(theme.accent_wash)
                                .text_size(px(Theme::TEXT_XS))
                                .text_color(theme.accent)
                                .child(tag)
                        })),
                ),
        )
        .child(cell(&entry.modified, DATE_WIDTH))
        .child(cell(&entry.size, SIZE_WIDTH).flex().justify_end())
        .child(cell(&entry.kind, KIND_WIDTH).debug_selector(|| format!("file-tree-kind-{ix}")))
        .on_click(click)
        .into_any_element()
}

/// Click selects (and focuses the tree); a double click toggles a folder
/// or opens a file.
fn click_handler(
    entry: &FileTreeEntry,
    handlers: &Handlers,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let handlers = handlers.clone();
    let key = entry.key.clone();
    let folder = entry.disclosure.is_some();
    move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
        window.focus(&handlers.focus, cx);
        if let Some(select) = &handlers.select {
            select(&key, window, cx);
        }
        if event.click_count() == 2 {
            let handler = if folder {
                &handlers.toggle
            } else {
                &handlers.open
            };
            if let Some(handler) = handler {
                handler(&key, window, cx);
            }
        }
    }
}

/// The disclosure chevron (a spinner while loading) of a folder row.
fn chevron(entry: &FileTreeEntry, handlers: &Handlers, theme: &Theme) -> Option<impl IntoElement> {
    entry.disclosure.map(|state| {
        let toggle = handlers.toggle.clone();
        let focus = handlers.focus.clone();
        let key = entry.key.clone();
        div()
            .id(SharedString::from(format!("toggle-{}", entry.key)))
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .map(|d| match state {
                Disclosure::Loading => d.child(
                    Spinner::new(SharedString::from(format!("loading-{}", entry.key))).size(11.0),
                ),
                Disclosure::Expanded | Disclosure::Collapsed => d.child(
                    icon(if state == Disclosure::Collapsed {
                        icons::CHEVRON_RIGHT
                    } else {
                        icons::CHEVRON_DOWN
                    })
                    .size(px(12.0))
                    .text_color(theme.text_faint),
                ),
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                // Like a row click, so the arrows then move through the tree.
                window.focus(&focus, cx);
                if let Some(toggle) = &toggle {
                    toggle(&key, window, cx);
                }
            })
    })
}

/// The parent row of `ix`: the nearest row above it one level up.
fn parent_of(rows: &[FileTreeRow], ix: usize) -> Option<usize> {
    let depth = rows.get(ix)?.depth();
    rows.get(..ix)?
        .iter()
        .rposition(|row| row.depth() < depth && row.entry().is_some())
}

/// What `key` does with the row at `current` selected: arrows move and
/// fold like a file manager's tree, Enter toggles a folder or opens a file.
fn key_intent(rows: &[FileTreeRow], current: Option<usize>, key: &str) -> Option<KeyIntent> {
    let entries: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.entry().is_some())
        .map(|(ix, _)| ix)
        .collect();
    let first = entries.first().copied();
    let last = entries.last().copied();
    let Some(current) = current else {
        return match key {
            "down" | "up" | "home" | "right" => first.map(KeyIntent::Select),
            "end" => last.map(KeyIntent::Select),
            _ => None,
        };
    };
    let depth = rows.get(current).map_or(0, FileTreeRow::depth);
    let disclosure = rows
        .get(current)
        .and_then(FileTreeRow::entry)
        .and_then(|e| e.disclosure);
    match key {
        "down" => entries
            .iter()
            .find(|&&ix| ix > current)
            .copied()
            .map(KeyIntent::Select),
        "up" => entries
            .iter()
            .rev()
            .find(|&&ix| ix < current)
            .copied()
            .map(KeyIntent::Select),
        "home" => first.map(KeyIntent::Select),
        "end" => last.map(KeyIntent::Select),
        "right" => match disclosure {
            Some(Disclosure::Collapsed) => Some(KeyIntent::Toggle(current)),
            Some(_) => entries
                .iter()
                .find(|&&ix| ix > current)
                .copied()
                .filter(|&ix| rows.get(ix).is_some_and(|r| r.depth() > depth))
                .map(KeyIntent::Select),
            None => None,
        },
        "left" => match disclosure {
            Some(Disclosure::Expanded | Disclosure::Loading) => Some(KeyIntent::Toggle(current)),
            _ => parent_of(rows, current).map(KeyIntent::Select),
        },
        "enter" => Some(if disclosure.is_some() {
            KeyIntent::Toggle(current)
        } else {
            KeyIntent::Open(current)
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, depth: usize, disclosure: Option<Disclosure>) -> FileTreeRow {
        FileTreeRow::Entry(FileTreeEntry {
            key: key.to_string().into(),
            name: key.rsplit('/').next().unwrap_or(key).to_string().into(),
            depth,
            disclosure,
            symlink: false,
            modified: SharedString::default(),
            size: SharedString::default(),
            kind: SharedString::default(),
            tag: None,
        })
    }

    fn rows() -> Vec<FileTreeRow> {
        vec![
            entry("/bin", 0, Some(Disclosure::Collapsed)),
            entry("/etc", 0, Some(Disclosure::Expanded)),
            entry("/etc/hosts", 1, None),
            entry("/root", 0, Some(Disclosure::Expanded)),
            FileTreeRow::Note {
                key: "/root/".into(),
                depth: 1,
                text: "Can't open".into(),
                error: true,
            },
            entry("/x", 0, None),
        ]
    }

    #[test]
    fn arrows_move_over_entries_and_skip_notes() {
        let rows = rows();
        assert_eq!(key_intent(&rows, None, "down"), Some(KeyIntent::Select(0)));
        assert_eq!(
            key_intent(&rows, Some(0), "down"),
            Some(KeyIntent::Select(1))
        );
        assert_eq!(
            key_intent(&rows, Some(3), "down"),
            Some(KeyIntent::Select(5))
        );
        assert_eq!(key_intent(&rows, Some(5), "up"), Some(KeyIntent::Select(3)));
        assert_eq!(key_intent(&rows, Some(5), "down"), None);
        assert_eq!(
            key_intent(&rows, Some(2), "end"),
            Some(KeyIntent::Select(5))
        );
    }

    #[test]
    fn left_and_right_fold_and_walk_the_tree() {
        let rows = rows();
        assert_eq!(
            key_intent(&rows, Some(0), "right"),
            Some(KeyIntent::Toggle(0))
        );
        assert_eq!(
            key_intent(&rows, Some(1), "right"),
            Some(KeyIntent::Select(2))
        );
        // An expanded folder whose only child is a note has nowhere to go.
        assert_eq!(key_intent(&rows, Some(3), "right"), None);
        assert_eq!(
            key_intent(&rows, Some(1), "left"),
            Some(KeyIntent::Toggle(1))
        );
        assert_eq!(
            key_intent(&rows, Some(2), "left"),
            Some(KeyIntent::Select(1))
        );
        assert_eq!(key_intent(&rows, Some(0), "left"), None);
        assert_eq!(
            key_intent(&rows, Some(2), "enter"),
            Some(KeyIntent::Open(2))
        );
        assert_eq!(
            key_intent(&rows, Some(0), "enter"),
            Some(KeyIntent::Toggle(0))
        );
    }
}

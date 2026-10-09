use std::fmt;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, IntoElement, RenderOnce, SharedString,
    Window, div, prelude::*, px,
};

use crate::{IconButton, Theme, icons};

/// Section heading above a card ("Ports", "Mounts", "Labels").
#[derive(IntoElement)]
#[must_use]
pub struct Heading {
    label: SharedString,
}

impl Heading {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl RenderOnce for Heading {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .pt(px(Theme::SPACE_LG + 4.0))
            .pb(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_XS))
            .text_size(px(Theme::TEXT_MD))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text)
            .child(self.label)
    }
}

/// How a value cell is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellStyle {
    #[default]
    Plain,
    Mono,
    Link,
    Muted,
}

type CellClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A table or key-value cell. Also an element on its own, for composing
/// custom values.
#[derive(Clone, IntoElement)]
#[must_use]
pub struct Cell {
    pub text: SharedString,
    pub style: CellStyle,
    on_click: Option<CellClick>,
    /// What the hover copy button puts on the clipboard, if it has one.
    copy: Option<SharedString>,
}

impl fmt::Debug for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cell")
            .field("text", &self.text)
            .field("style", &self.style)
            .field("clickable", &self.on_click.is_some())
            .field("copy", &self.copy)
            .finish()
    }
}

impl Cell {
    pub fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Plain,
            on_click: None,
            copy: None,
        }
    }
    pub fn mono(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Mono,
            on_click: None,
            copy: None,
        }
    }
    pub fn link(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Link,
            on_click: None,
            copy: None,
        }
    }
    pub fn muted(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Muted,
            on_click: None,
            copy: None,
        }
    }

    /// Make the cell clickable (pair with [`Cell::link`]).
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Show a copy button on hover that copies the full text, which may be
    /// truncated on screen.
    pub fn copyable(self) -> Self {
        let text = self.text.clone();
        self.copy_value(text)
    }

    /// Show a copy button on hover that copies `value` (e.g. the full ID
    /// behind a shortened one).
    pub fn copy_value(mut self, value: impl Into<SharedString>) -> Self {
        self.copy = Some(value.into());
        self
    }

    /// `id` keys the click handlers; it must be unique among siblings.
    fn render_keyed(self, id: impl Into<ElementId>, theme: &Theme) -> AnyElement {
        let Some(copy) = self.copy.clone() else {
            return self.render_value(id, theme);
        };
        let group: SharedString = "cell".into();
        div()
            .id(id)
            .group(group.clone())
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_XS))
            .child(Self { copy: None, ..self }.render_value("value", theme))
            .child(
                div()
                    .flex_none()
                    .opacity(0.0)
                    .group_hover(group, |s| s.opacity(1.0))
                    .child(
                        IconButton::new("copy", icons::COPY)
                            .size(20.0, 12.0)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy.to_string(),
                                ));
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_value(self, id: impl Into<ElementId>, theme: &Theme) -> AnyElement {
        let d = div().min_w_0().truncate().child(self.text);
        let d = match self.style {
            CellStyle::Plain => d.text_color(theme.text),
            CellStyle::Mono => d
                .text_color(theme.text)
                .font_family(theme.font_mono.clone()),
            CellStyle::Link => d.text_color(theme.accent).cursor_pointer(),
            CellStyle::Muted => d.text_color(theme.text_faint),
        };
        match self.on_click {
            Some(f) => d
                .id(id)
                .hover(Styled::underline)
                .on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx);
                })
                .into_any_element(),
            None => d.into_any_element(),
        }
    }
}

fn card(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(Theme::PANEL_RADIUS))
        .bg(theme.surface_card)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
}

/// A rounded card of label → value rows separated by hairlines.
#[derive(IntoElement, Default)]
#[must_use]
pub struct KeyValueCard {
    rows: Vec<(SharedString, AnyElement)>,
}

impl KeyValueCard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn row(mut self, label: impl Into<SharedString>, value: Cell) -> Self {
        let label = label.into();
        let value = CellElement(label.clone(), value).into_any_element();
        self.rows.push((label, value));
        self
    }

    /// A row with a custom value element (e.g. status dot + text).
    pub fn row_element(mut self, label: impl Into<SharedString>, value: impl IntoElement) -> Self {
        self.rows.push((label.into(), value.into_any_element()));
        self
    }
}

impl RenderOnce for Cell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("cell-{}", self.text));
        self.render_keyed(id, Theme::of(cx))
    }
}

/// A key-value cell, keyed by its row label.
#[derive(IntoElement)]
struct CellElement(SharedString, Cell);

impl RenderOnce for CellElement {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.1.render_keyed(self.0, Theme::of(cx))
    }
}

impl RenderOnce for KeyValueCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        card(theme).children(
            self.rows
                .into_iter()
                .enumerate()
                .map(|(ix, (label, value))| {
                    div()
                        .h(px(38.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_LG))
                        .text_size(px(Theme::TEXT_MD))
                        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                        .child(div().flex_none().text_color(theme.text_muted).child(label))
                        .child(div().flex_1().min_w_0().flex().justify_end().child(value))
                }),
        )
    }
}

/// A simple table inside a card: header row + body rows, flex-weighted columns.
#[derive(IntoElement)]
#[must_use]
pub struct DataTable {
    id: Option<ElementId>,
    columns: Vec<(SharedString, f32)>,
    rows: Vec<Vec<Cell>>,
    empty: SharedString,
}

impl DataTable {
    pub fn new(columns: impl IntoIterator<Item = (&'static str, f32)>) -> Self {
        Self {
            id: None,
            columns: columns
                .into_iter()
                .map(|(name, weight)| (name.into(), weight))
                .collect(),
            rows: Vec::new(),
            empty: "None".into(),
        }
    }

    /// Scopes the cells' element ids. Needed when cells are clickable or
    /// copyable and another table shares the parent.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn row(mut self, cells: Vec<Cell>) -> Self {
        self.rows.push(cells);
        self
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = Vec<Cell>>) -> Self {
        self.rows.extend(rows);
        self
    }

    pub fn empty(mut self, text: impl Into<SharedString>) -> Self {
        self.empty = text.into();
        self
    }
}

impl RenderOnce for DataTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let weights: Vec<f32> = self.columns.iter().map(|(_, w)| *w).collect();
        let column = |weight: f32| div().flex_basis(px(0.0)).flex_grow(weight).min_w_0();

        let header = div()
            .h(px(30.0))
            .px(px(Theme::SPACE_MD))
            .flex()
            .items_center()
            .gap(px(Theme::SPACE_MD))
            .bg(theme.row_hover)
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(Theme::TEXT_XS))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.text_faint)
            .children(
                self.columns
                    .iter()
                    .map(|(name, w)| column(*w).child(name.clone())),
            );

        let body = if self.rows.is_empty() {
            vec![
                div()
                    .h(px(36.0))
                    .px(px(Theme::SPACE_MD))
                    .flex()
                    .items_center()
                    .text_size(px(Theme::TEXT_MD))
                    .text_color(theme.text_faint)
                    .child(self.empty),
            ]
        } else {
            self.rows
                .into_iter()
                .enumerate()
                .map(|(row, cells)| {
                    div()
                        .h(px(34.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_MD))
                        .text_size(px(Theme::TEXT_MD))
                        .when(row > 0, |d| d.border_t_1().border_color(theme.border))
                        .children(cells.into_iter().zip(weights.iter()).enumerate().map(
                            |(col, (cell, w))| {
                                let id = SharedString::from(format!("cell-{row}-{col}"));
                                column(*w).child(cell.render_keyed(id, theme))
                            },
                        ))
                })
                .collect()
        };

        let table = card(theme).child(header).children(body);
        match self.id {
            Some(id) => table.id(id).into_any_element(),
            None => table.into_any_element(),
        }
    }
}

/// Centered empty-state message for a pane.
pub fn empty_state(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(Theme::SPACE_XS))
        .child(
            div()
                .text_size(px(Theme::TEXT_LG))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_muted)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text_faint)
                .child(detail.into()),
        )
}

use gpui::{
    AnyElement, App, FontWeight, IntoElement, RenderOnce, SharedString, Window, div, prelude::*,
    px,
};

use crate::Theme;

/// Section heading above a card ("Ports", "Mounts", "Labels").
#[derive(IntoElement)]
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

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: SharedString,
    pub style: CellStyle,
}

impl Cell {
    pub fn plain(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Plain,
        }
    }
    pub fn mono(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Mono,
        }
    }
    pub fn link(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Link,
        }
    }
    pub fn muted(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            style: CellStyle::Muted,
        }
    }

    fn render(self, theme: &Theme) -> gpui::Div {
        let d = div().min_w_0().truncate().child(self.text);
        match self.style {
            CellStyle::Plain => d.text_color(theme.text),
            CellStyle::Mono => d.text_color(theme.text).font_family(theme.font_mono.clone()),
            CellStyle::Link => d.text_color(theme.accent).cursor_pointer(),
            CellStyle::Muted => d.text_color(theme.text_faint),
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
pub struct KeyValueCard {
    rows: Vec<(SharedString, AnyElement)>,
}

impl KeyValueCard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn row(mut self, label: impl Into<SharedString>, value: Cell) -> Self {
        self.rows.push((label.into(), CellElement(value).into_any_element()));
        self
    }

    /// A row with a custom value element (e.g. status dot + text).
    pub fn row_element(mut self, label: impl Into<SharedString>, value: impl IntoElement) -> Self {
        self.rows.push((label.into(), value.into_any_element()));
        self
    }
}

#[derive(IntoElement)]
struct CellElement(Cell);

impl RenderOnce for CellElement {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.0.render(Theme::of(cx))
    }
}

impl RenderOnce for KeyValueCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        card(theme).children(self.rows.into_iter().enumerate().map(|(ix, (label, value))| {
            div()
                .h(px(38.0))
                .px(px(Theme::SPACE_MD))
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_LG))
                .text_size(px(Theme::TEXT_MD))
                .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .justify_end()
                        .child(value),
                )
        }))
    }
}

/// A simple table inside a card: header row + body rows, flex-weighted columns.
#[derive(IntoElement)]
pub struct DataTable {
    columns: Vec<(SharedString, f32)>,
    rows: Vec<Vec<Cell>>,
    empty: SharedString,
}

impl DataTable {
    pub fn new(columns: impl IntoIterator<Item = (&'static str, f32)>) -> Self {
        Self {
            columns: columns
                .into_iter()
                .map(|(name, weight)| (name.into(), weight))
                .collect(),
            rows: Vec::new(),
            empty: "None".into(),
        }
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
        let column = |weight: f32| {
            div().flex_basis(px(0.0)).flex_grow(weight).min_w_0()
        };

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
                .map(|(ix, cells)| {
                    div()
                        .h(px(34.0))
                        .px(px(Theme::SPACE_MD))
                        .flex()
                        .items_center()
                        .gap(px(Theme::SPACE_MD))
                        .text_size(px(Theme::TEXT_MD))
                        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
                        .children(cells.into_iter().zip(weights.iter()).map(|(cell, w)| {
                            column(*w).child(cell.render(theme))
                        }))
                })
                .collect()
        };

        card(theme).child(header).children(body)
    }
}

/// Centered empty-state message for a pane.
pub fn empty_state(title: impl Into<SharedString>, detail: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
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

//! The terminal view: a focusable entity owning the [`Emulator`], and the
//! element that paints its grid.
//!
//! The element measures cell metrics from the real mono font each frame,
//! resizes the emulator to fit and reports the new size as
//! [`TerminalEvent::Resize`]. Keystrokes, pastes and query responses leave
//! as [`TerminalEvent::Input`]. Painting: one quad per run of non-default
//! background, a selection wash, column-pinned shaped text, the cursor.
//!
//! Adapted from zeron's `crates/ui/src/terminal/view.rs` (MIT).

use std::sync::Arc;

use berth_ui::Theme;
use berth_ui::ansi::ansi_color;
use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, PaintQuad, Pixels, Point, Render, ScrollWheelEvent, ShapedLine, SharedString,
    Style, TextAlign, TextRun, UnderlineStyle, Window, div, fill, font, outline, point, prelude::*,
    px, relative, size,
};
use num_traits::ToPrimitive as _;

use crate::emulator::{CellColor, CellSnapshot, Emulator, GridSize, Side, Snapshot};
use crate::input::{keystroke_bytes, paste_bytes};

const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 18.0;
const PADDING: f32 = 12.0;
const MAX_CELLS: u16 = 1000;

pub enum TerminalEvent {
    /// Bytes for the process: keystrokes, pastes, terminal query replies.
    Input(Vec<u8>),
    /// The grid was resized to fit the view.
    Resize(GridSize),
}

/// Where the grid was last painted, for pointer → cell mapping.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    origin: Point<Pixels>,
    cell_w: f32,
}

pub struct TerminalView {
    emulator: Emulator,
    focus: FocusHandle,
    geometry: Option<Geometry>,
    selecting: bool,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            emulator: Emulator::new(GridSize::new(80, 24)),
            focus: cx.focus_handle(),
            geometry: None,
            selecting: false,
        }
    }

    #[must_use]
    pub const fn size(&self) -> GridSize {
        self.emulator.size()
    }

    /// Feed process output.
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let replies = self.emulator.feed(bytes);
        if !replies.is_empty() {
            cx.emit(TerminalEvent::Input(replies));
        }
        cx.notify();
    }

    /// Start over with an empty screen, keeping the size.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.emulator = Emulator::new(self.emulator.size());
        cx.notify();
    }

    fn input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.emulator.clear_selection();
        self.emulator.scroll_to_bottom();
        cx.emit(TerminalEvent::Input(bytes));
        cx.notify();
    }

    fn fit(&mut self, size: GridSize, cx: &mut Context<Self>) {
        if size != self.emulator.size() {
            self.emulator.resize(size);
            cx.emit(TerminalEvent::Resize(self.emulator.size()));
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = &keystroke.modifiers;
        if mods.control && mods.shift {
            match keystroke.key.as_str() {
                "c" => {
                    if let Some(text) = self.emulator.selection_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        let bytes = paste_bytes(&text, self.emulator.bracketed_paste_mode());
                        self.input(bytes, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if let Some(bytes) = keystroke_bytes(
            &keystroke.key,
            keystroke.key_char.as_deref(),
            *mods,
            self.emulator.app_cursor_mode(),
        ) {
            cx.stop_propagation();
            self.input(bytes, cx);
        }
    }

    fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, Side)> {
        let geometry = self.geometry?;
        let grid = self.emulator.size();
        let x = f32::from(position.x) - f32::from(geometry.origin.x);
        let y = f32::from(position.y) - f32::from(geometry.origin.y);
        let col = usize::from(whole(x / geometry.cell_w, grid.cols.saturating_sub(1)));
        let row = usize::from(whole(y / LINE_HEIGHT, grid.rows.saturating_sub(1)));
        let within = x.max(0.0) % geometry.cell_w;
        let side = if within > geometry.cell_w / 2.0 {
            Side::Right
        } else {
            Side::Left
        };
        Some((row, col, side))
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.emulator.clear_selection();
        if let Some((row, col, side)) = self.cell_at(event.position) {
            self.emulator
                .start_selection(event.click_count >= 2, row, col, side);
            self.selecting = true;
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some((row, col, side)) = self.cell_at(event.position) {
            self.emulator.update_selection(row, col, side);
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let dy = f32::from(event.delta.pixel_delta(px(LINE_HEIGHT)).y) / LINE_HEIGHT;
        let lines = i32::from(whole(dy.abs().round(), MAX_CELLS));
        if lines > 0 {
            let lines = if dy > 0.0 {
                lines
            } else {
                lines.saturating_neg()
            };
            self.emulator.scroll(lines);
            cx.notify();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal")
            .size_full()
            .track_focus(&self.focus)
            .key_context("Terminal")
            .cursor_text()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(TerminalElement {
                view: cx.entity(),
                focused: false,
            })
    }
}

/// The largest whole number `n ≤ value`, clamped to `0..=max`; non-finite
/// metrics (a failed font probe) count as zero.
fn whole(value: f32, max: u16) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    value
        .clamp(0.0, f32::from(max))
        .to_u16()
        .unwrap_or_default()
}

/// `n` cells as `f32` (exact: grids are capped at [`MAX_CELLS`]).
fn cells(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(MAX_CELLS))
}

struct TerminalElement {
    view: Entity<TerminalView>,
    focused: bool,
}

struct Prepaint {
    backgrounds: Vec<PaintQuad>,
    selection: Vec<PaintQuad>,
    /// Per row, shaped segments and the column each starts at.
    lines: Vec<Vec<(usize, ShapedLine)>>,
    cursor: Option<PaintQuad>,
    origin: Point<Pixels>,
    cell_w: f32,
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme = Theme::of(cx).clone();
        let mono = mono_font(&theme);
        let font_id = window.text_system().resolve_font(&mono);
        let cell_w = window
            .text_system()
            .em_advance(font_id, px(FONT_SIZE))
            .map_or(FONT_SIZE * 0.6, f32::from);
        let inner_w = PADDING.mul_add(-2.0, f32::from(bounds.size.width));
        let inner_h = PADDING.mul_add(-2.0, f32::from(bounds.size.height));
        let grid = GridSize::new(
            whole(inner_w / cell_w, MAX_CELLS),
            whole(inner_h / LINE_HEIGHT, MAX_CELLS),
        );
        let origin = point(
            px(f32::from(bounds.origin.x) + PADDING),
            px(f32::from(bounds.origin.y) + PADDING),
        );
        self.focused = self.view.read(cx).focus.is_focused(window);
        let snapshot = self.view.update(cx, |view, cx| {
            view.fit(grid, cx);
            view.geometry = Some(Geometry { origin, cell_w });
            view.emulator.snapshot()
        });
        layout_grid(
            &snapshot,
            &theme,
            &mono,
            origin,
            cell_w,
            self.focused,
            window,
        )
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (): &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.backgrounds.drain(..) {
                window.paint_quad(quad);
            }
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            let origin = prepaint.origin;
            for (row, segments) in prepaint.lines.iter().enumerate() {
                let y = LINE_HEIGHT.mul_add(cells(row), f32::from(origin.y));
                for (col, line) in segments {
                    let x = prepaint.cell_w.mul_add(cells(*col), f32::from(origin.x));
                    let _ = line.paint(
                        point(px(x), px(y)),
                        px(LINE_HEIGHT),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        });
    }
}

/// Ligatures off: a terminal is a fixed grid and `->` must stay two cells.
fn mono_font(theme: &Theme) -> Font {
    let mut mono = font(theme.font_mono.clone());
    mono.features = FontFeatures(Arc::new(vec![
        ("liga".into(), 0),
        ("calt".into(), 0),
        ("dlig".into(), 0),
    ]));
    mono
}

fn resolve(color: CellColor, theme: &Theme) -> Hsla {
    match color {
        CellColor::Foreground => theme.text,
        CellColor::Background => theme.bg,
        CellColor::Ansi(color) => ansi_color(color, theme),
    }
}

fn layout_grid(
    snapshot: &Snapshot,
    theme: &Theme,
    mono: &Font,
    origin: Point<Pixels>,
    cell_w: f32,
    focused: bool,
    window: &Window,
) -> Prepaint {
    let span = |row: usize, start: usize, end: usize| {
        Bounds::new(
            point(
                px(cell_w.mul_add(cells(start), f32::from(origin.x))),
                px(LINE_HEIGHT.mul_add(cells(row), f32::from(origin.y))),
            ),
            size(
                px(cell_w * cells(end.saturating_sub(start))),
                px(LINE_HEIGHT),
            ),
        )
    };
    let mut backgrounds = Vec::new();
    let mut selection = Vec::new();
    for (row_ix, row) in snapshot.rows.iter().enumerate() {
        for (start, end, bg) in runs(row, |c| (c.bg != CellColor::Background).then_some(c.bg)) {
            backgrounds.push(fill(span(row_ix, start, end), resolve(bg, theme)));
        }
        for (start, end, ()) in runs(row, |c| c.selected.then_some(())) {
            selection.push(fill(span(row_ix, start, end), theme.accent.opacity(0.32)));
        }
    }
    let lines = snapshot
        .rows
        .iter()
        .map(|row| shape_row(row, theme, mono, window))
        .collect();
    let cursor = snapshot.cursor.map(|c| {
        let bounds = span(c.row, c.col, c.col.saturating_add(1));
        if focused {
            fill(bounds, theme.text.opacity(0.7))
        } else {
            outline(bounds, theme.text_muted, gpui::BorderStyle::Solid)
        }
    });
    Prepaint {
        backgrounds,
        selection,
        lines,
        cursor,
        origin,
        cell_w,
    }
}

/// Maximal runs of equal `Some` keys as `(start, end, key)` column ranges.
fn runs<K: PartialEq + Copy>(
    row: &[CellSnapshot],
    key: impl Fn(&CellSnapshot) -> Option<K>,
) -> Vec<(usize, usize, K)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, K)> = None;
    for (col, next) in row
        .iter()
        .map(&key)
        .chain(std::iter::once(None))
        .enumerate()
    {
        match open {
            Some((_, current)) if next == Some(current) => {}
            Some((start, current)) => {
                out.push((start, col, current));
                open = next.map(|k| (col, k));
            }
            None => open = next.map(|k| (col, k)),
        }
    }
    out
}

/// Shape one row into column-pinned segments. ASCII runs shape together
/// (cell-width in a mono font); anything that may fall back to another font
/// (box drawing, CJK, emoji) is its own segment pinned to its column, so a
/// wider fallback glyph can't push the rest of the row off the grid.
fn shape_row(
    row: &[CellSnapshot],
    theme: &Theme,
    mono: &Font,
    window: &Window,
) -> Vec<(usize, ShapedLine)> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut text_runs: Vec<TextRun> = Vec::new();
    let mut start = 0usize;
    let mut flush = |text: &mut String, text_runs: &mut Vec<TextRun>, start: usize| {
        if text.is_empty() {
            return;
        }
        let shaped = window.text_system().shape_line(
            SharedString::from(std::mem::take(text)),
            px(FONT_SIZE),
            text_runs,
            None,
        );
        text_runs.clear();
        segments.push((start, shaped));
    };
    for (col, cell) in row.iter().enumerate() {
        if cell.wide_spacer {
            continue;
        }
        let pinned = !cell.ch.is_ascii() || cell.wide;
        if pinned {
            flush(&mut text, &mut text_runs, start);
        }
        if text.is_empty() {
            start = col;
        }
        let run = cell_run(cell, theme, mono);
        text.push(cell.ch);
        match text_runs.last_mut() {
            Some(last)
                if last.color == run.color
                    && last.font == run.font
                    && last.underline == run.underline =>
            {
                last.len = last.len.saturating_add(run.len);
            }
            _ => text_runs.push(run),
        }
        if pinned {
            flush(&mut text, &mut text_runs, start);
        }
    }
    flush(&mut text, &mut text_runs, start);
    segments
}

fn cell_run(cell: &CellSnapshot, theme: &Theme, mono: &Font) -> TextRun {
    let mut color = resolve(cell.fg, theme);
    if cell.dim {
        color = color.opacity(0.6);
    }
    let mut cell_font = mono.clone();
    cell_font.weight = if cell.bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    cell_font.style = if cell.italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    TextRun {
        len: cell.ch.len_utf8(),
        font: cell_font,
        color,
        background_color: None,
        underline: cell.underline.then_some(UnderlineStyle {
            color: Some(color),
            thickness: px(1.0),
            wavy: false,
        }),
        strikethrough: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{runs, whole};
    use crate::emulator::CellSnapshot;

    #[test]
    fn whole_floors_and_clamps() {
        assert_eq!(whole(0.0, 10), 0);
        assert_eq!(whole(3.99, 10), 3);
        assert_eq!(whole(4.0, 10), 4);
        assert_eq!(whole(99.0, 10), 10);
        assert_eq!(whole(f32::NAN, 10), 0);
        assert_eq!(whole(-5.0, 10), 0);
        assert_eq!(whole(f32::INFINITY, 10), 0);
        assert_eq!(whole(0.99, 10), 0);
    }

    #[test]
    fn runs_merge_equal_neighbours() {
        let mut row = vec![CellSnapshot::default(); 6];
        for cell in row.iter_mut().skip(1).take(3) {
            cell.selected = true;
        }
        assert_eq!(runs(&row, |c| c.selected.then_some(())), vec![(1, 4, ())]);
    }
}

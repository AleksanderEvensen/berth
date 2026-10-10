//! The terminal core: `alacritty_terminal`'s `Term` + vte's ANSI `Processor`
//! as a pure state machine. Bytes in ([`Emulator::feed`]), grid snapshots out
//! ([`Emulator::snapshot`]). No I/O, timers or gpui, so the escape-sequence
//! surface is unit-testable with scripted byte strings.
//!
//! Adapted from zeron's `crates/ui/src/terminal/emulator.rs` (MIT).

use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
use berth_ui::ansi::AnsiColor;

pub use alacritty_terminal::index::Side;
pub use alacritty_terminal::vte::ansi::Rgb;

/// Scrollback kept in the emulator, in lines.
pub const SCROLLBACK_LINES: usize = 10_000;

/// Viewport dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

impl GridSize {
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(2),
            rows: rows.max(1),
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }
    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// A cell's paint colour, resolved against the theme by the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    Foreground,
    Background,
    Ansi(AnsiColor),
}

fn map_color(color: Color) -> CellColor {
    match color {
        Color::Named(NamedColor::Background) => CellColor::Background,
        color => sgr_color(color).map_or(CellColor::Foreground, CellColor::Ansi),
    }
}

/// A parsed SGR colour as an [`AnsiColor`]; `None` for the default
/// foreground/background. Shared with the log view so both resolve colours
/// the same way.
#[must_use]
pub fn sgr_color(color: Color) -> Option<AnsiColor> {
    match color {
        Color::Spec(Rgb { r, g, b }) => Some(AnsiColor::Rgb(r, g, b)),
        Color::Indexed(ix) => Some(AnsiColor::Indexed(ix)),
        Color::Named(named) => named_index(named).map(AnsiColor::Indexed),
    }
}

/// ANSI 0–15 for a named colour; dim variants fold onto their base (the DIM
/// flag still travels on the cell). `None` for foreground/background/cursor.
const fn named_index(named: NamedColor) -> Option<u8> {
    Some(match named {
        NamedColor::Black | NamedColor::DimBlack => 0,
        NamedColor::Red | NamedColor::DimRed => 1,
        NamedColor::Green | NamedColor::DimGreen => 2,
        NamedColor::Yellow | NamedColor::DimYellow => 3,
        NamedColor::Blue | NamedColor::DimBlue => 4,
        NamedColor::Magenta | NamedColor::DimMagenta => 5,
        NamedColor::Cyan | NamedColor::DimCyan => 6,
        NamedColor::White | NamedColor::DimWhite => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        _ => return None,
    })
}

/// One rendered cell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "mirrors cell flags")]
pub struct CellSnapshot {
    pub ch: char,
    /// Combining marks drawn on top of `ch` (accents, ZWJ sequences).
    pub combining: Option<Box<[char]>>,
    pub fg: CellColor,
    pub bg: CellColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    /// A double-width char (this cell plus the next spacer).
    pub wide: bool,
    /// The spacer half of a wide char: background only, never shaped.
    pub wide_spacer: bool,
    pub selected: bool,
}

impl Default for CellSnapshot {
    fn default() -> Self {
        Self {
            ch: ' ',
            combining: None,
            fg: CellColor::Foreground,
            bg: CellColor::Background,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            wide: false,
            wide_spacer: false,
            selected: false,
        }
    }
}

/// What the emulator asks its embedder when a program queries the terminal.
pub struct Host<'a> {
    /// RGB for a palette index (0–255), 256 = foreground, 257 = background,
    /// 258 = cursor.
    pub color: &'a dyn Fn(usize) -> Option<Rgb>,
    /// One cell in pixels.
    pub cell_px: (u16, u16),
}

impl Host<'_> {
    /// Answers nothing beyond plain text replies; for tests and headless use.
    pub const NONE: Host<'static> = Host {
        color: &|_| None,
        cell_px: (0, 0),
    };
}

/// Cursor in viewport coordinates (row 0 = top of the visible grid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub row: usize,
    pub col: usize,
}

/// The visible grid, top to bottom.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub rows: Vec<Vec<CellSnapshot>>,
    pub cursor: Option<CursorSnapshot>,
}

/// `Term` callbacks; interior-mutable because `send_event` takes `&self`.
#[derive(Default, Clone)]
struct EventCapture {
    events: Rc<RefCell<Vec<Event>>>,
}

impl EventListener for EventCapture {
    fn send_event(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }
}

/// A pure fold of TTY bytes into a renderable grid.
pub struct Emulator {
    term: Term<EventCapture>,
    parser: Processor,
    capture: EventCapture,
    size: GridSize,
}

impl Emulator {
    #[must_use]
    pub fn new(size: GridSize) -> Self {
        let capture = EventCapture::default();
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Config::default()
        };
        let size = GridSize::new(size.cols, size.rows);
        Self {
            term: Term::new(config, &size, capture.clone()),
            parser: Processor::new(),
            capture,
            size,
        }
    }

    /// Advance over TTY output. Returns bytes the terminal wants written
    /// back (DSR/DA query responses).
    pub fn feed(&mut self, bytes: &[u8], host: &Host<'_>) -> Vec<u8> {
        self.parser.advance(&mut self.term, bytes);
        let mut responses = Vec::new();
        for event in self.capture.events.borrow_mut().drain(..) {
            let reply = match event {
                Event::PtyWrite(text) => text,
                // OSC 4/10/11/12 colour queries: 0–255 palette, then the
                // default foreground, background and cursor.
                Event::ColorRequest(index, format) => match (host.color)(index) {
                    Some(color) => format(color),
                    None => continue,
                },
                // CSI 14 t / 18 t: text area size in pixels and cells.
                Event::TextAreaSizeRequest(format) => format(WindowSize {
                    num_lines: self.size.rows,
                    num_cols: self.size.cols,
                    cell_width: host.cell_px.0,
                    cell_height: host.cell_px.1,
                }),
                _ => continue,
            };
            responses.extend_from_slice(reply.as_bytes());
        }
        responses
    }

    pub fn resize(&mut self, size: GridSize) {
        let size = GridSize::new(size.cols, size.rows);
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
    }

    #[must_use]
    pub const fn size(&self) -> GridSize {
        self.size
    }

    /// Arrow keys send SS3 instead of CSI (DECCKM).
    #[must_use]
    pub fn app_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    #[must_use]
    pub fn bracketed_paste_mode(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Scroll the view: positive = up into history.
    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    fn display_offset(&self) -> i32 {
        i32::try_from(self.term.grid().display_offset()).unwrap_or(i32::MAX)
    }

    /// The grid point under a viewport cell; selections anchor in grid
    /// space so they stay on their text while output scrolls.
    fn grid_point(&self, row: usize, col: usize) -> Point {
        let row = i32::try_from(row).unwrap_or(i32::MAX);
        Point::new(
            Line(row.saturating_sub(self.display_offset())),
            Column(col.min(usize::from(self.size.cols).saturating_sub(1))),
        )
    }

    pub fn start_selection(&mut self, semantic: bool, row: usize, col: usize, side: Side) {
        let ty = if semantic {
            SelectionType::Semantic
        } else {
            SelectionType::Simple
        };
        let point = self.grid_point(row, col);
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&mut self, row: usize, col: usize, side: Side) {
        let point = self.grid_point(row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    /// Select everything: the scrollback and the screen (the keyboard's way
    /// to copy output).
    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let start = Point::new(grid.topmost_line(), Column(0));
        let end = Point::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        self.term.selection = Some(selection);
    }

    pub const fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// The visible grid with selection marks and the cursor.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let content = self.term.renderable_content();
        let offset = self.display_offset();
        let cols = usize::from(self.size.cols);
        let mut rows = vec![vec![CellSnapshot::default(); cols]; usize::from(self.size.rows)];
        let viewport_row = |line: Line| {
            line.0
                .checked_add(offset)
                .and_then(|r| usize::try_from(r).ok())
        };
        for indexed in content.display_iter {
            let Some(slot) = viewport_row(indexed.point.line)
                .and_then(|r| rows.get_mut(r))
                .and_then(|row| row.get_mut(indexed.point.column.0))
            else {
                continue;
            };
            let cell = indexed.cell;
            let flags = cell.flags;
            let inverse = flags.contains(Flags::INVERSE);
            let (fg, bg) = (map_color(cell.fg), map_color(cell.bg));
            let (fg, bg) = if inverse { (bg, fg) } else { (fg, bg) };
            let hidden = flags.contains(Flags::HIDDEN);
            *slot = CellSnapshot {
                ch: if hidden { ' ' } else { cell.c },
                combining: cell
                    .zerowidth()
                    .filter(|_| !hidden)
                    .map(Box::<[char]>::from),
                fg,
                bg,
                bold: flags.contains(Flags::BOLD),
                dim: flags.contains(Flags::DIM),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                wide: flags.contains(Flags::WIDE_CHAR),
                wide_spacer: flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: content
                    .selection
                    .is_some_and(|range| range.contains(indexed.point)),
            };
        }
        let cursor = (content.cursor.shape != CursorShape::Hidden)
            .then(|| {
                let row = viewport_row(content.cursor.point.line)?;
                (row < rows.len()).then_some(CursorSnapshot {
                    row,
                    col: content.cursor.point.column.0,
                })
            })
            .flatten();
        Snapshot { rows, cursor }
    }
}

impl std::fmt::Debug for Emulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emulator")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu(cols: u16, rows: u16) -> Emulator {
        Emulator::new(GridSize::new(cols, rows))
    }

    fn row_text(snapshot: &Snapshot, row: usize) -> String {
        let text: String = snapshot.rows[row]
            .iter()
            .filter(|c| !c.wide_spacer)
            .map(|c| c.ch)
            .collect();
        text.trim_end().to_string()
    }

    #[test]
    fn crlf_and_wrap() {
        let mut e = emu(5, 3);
        e.feed(b"ab\r\ncdefgh", &Host::NONE);
        let s = e.snapshot();
        assert_eq!(row_text(&s, 0), "ab");
        assert_eq!(row_text(&s, 1), "cdefg");
        assert_eq!(row_text(&s, 2), "h");
        assert_eq!(s.cursor, Some(CursorSnapshot { row: 2, col: 1 }));
    }

    #[test]
    fn sgr_colours_and_attributes() {
        let mut e = emu(10, 1);
        e.feed(
            b"\x1b[1;31mA\x1b[0m\x1b[38;5;196mB\x1b[38;2;1;2;3mC\x1b[7mD",
            &Host::NONE,
        );
        let row = &e.snapshot().rows[0];
        assert_eq!(row[0].fg, CellColor::Ansi(AnsiColor::Indexed(1)));
        assert!(row[0].bold);
        assert_eq!(row[1].fg, CellColor::Ansi(AnsiColor::Indexed(196)));
        assert_eq!(row[2].fg, CellColor::Ansi(AnsiColor::Rgb(1, 2, 3)));
        assert_eq!(row[3].bg, CellColor::Ansi(AnsiColor::Rgb(1, 2, 3)));
    }

    #[test]
    fn colour_and_size_queries_are_answered() {
        let mut e = emu(10, 3);
        let host = Host {
            color: &|ix| (ix == 257).then_some(Rgb { r: 1, g: 2, b: 3 }),
            cell_px: (8, 18),
        };
        let reply = String::from_utf8(e.feed(b"\x1b]11;?\x07", &host)).unwrap();
        assert!(reply.starts_with("\x1b]11;rgb:0101/0202/0303"), "{reply:?}");
        let reply = String::from_utf8(e.feed(b"\x1b[14t", &host)).unwrap();
        assert_eq!(reply, "\x1b[4;54;80t");
        assert_eq!(e.feed(b"\x1b]10;?\x07", &host), b"");
    }

    #[test]
    fn combining_marks_are_kept() {
        let mut e = emu(5, 1);
        e.feed("e\u{301}x".as_bytes(), &Host::NONE);
        let row = &e.snapshot().rows[0];
        assert_eq!(row[0].ch, 'e');
        assert_eq!(row[0].combining.as_deref(), Some(&['\u{301}'][..]));
        assert_eq!(row[1].ch, 'x');
    }

    #[test]
    fn dsr_produces_a_response() {
        let mut e = emu(10, 3);
        assert_eq!(e.feed(b"\x1b[6n", &Host::NONE), b"\x1b[1;1R".to_vec());
    }

    #[test]
    fn select_all_copies_scrollback_and_screen() {
        let mut e = emu(10, 2);
        e.feed(b"1\r\n2\r\n3", &Host::NONE);
        e.select_all();
        let text = e.selection_text().unwrap_or_default();
        assert!(text.contains('1') && text.contains('3'), "{text:?}");
    }

    #[test]
    fn scrollback_and_resize() {
        let mut e = emu(10, 2);
        e.feed(b"1\r\n2\r\n3", &Host::NONE);
        assert_eq!(row_text(&e.snapshot(), 0), "2");
        e.scroll(1);
        assert_eq!(row_text(&e.snapshot(), 0), "1");
        assert_eq!(e.snapshot().cursor, None, "cursor scrolled out");
        e.scroll_to_bottom();
        e.resize(GridSize::new(20, 4));
        assert_eq!(e.snapshot().rows.len(), 4);
        assert_eq!(e.snapshot().rows[0].len(), 20);
    }

    #[test]
    fn selection_yields_text() {
        let mut e = emu(10, 2);
        e.feed(b"hello world", &Host::NONE);
        e.start_selection(false, 0, 0, Side::Left);
        e.update_selection(0, 4, Side::Right);
        assert_eq!(e.selection_text().as_deref(), Some("hello"));
        assert!(e.snapshot().rows[0][2].selected);
        e.clear_selection();
        assert_eq!(e.selection_text(), None);
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let mut e = emu(6, 1);
        e.feed("界x".as_bytes(), &Host::NONE);
        let row = &e.snapshot().rows[0];
        assert!(row[0].wide);
        assert!(row[1].wide_spacer);
        assert_eq!(row[2].ch, 'x');
    }
}

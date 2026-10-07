//! ANSI SGR → styled spans for log lines.
//!
//! Lines are parsed once, on arrival, by `vte` (the parser behind the
//! terminal): the plain text (what search and copy see) plus style spans
//! over it. Colour/attribute SGR codes become spans; every other escape
//! sequence and control character is dropped. Tabs expand to the next
//! multiple of eight columns.

use std::ops::Range;

use berth_terminal::sgr_color;
use berth_terminal::vte::ansi::{Attr, Handler, Processor};
use berth_ui::ansi::AnsiColor;

/// Text attributes from SGR codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(clippy::struct_excessive_bools, reason = "independent SGR attributes")]
pub struct SgrStyle {
    pub fg: Option<AnsiColor>,
    pub bg: Option<AnsiColor>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl SgrStyle {
    fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// A styled byte range of the plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub style: SgrStyle,
}

impl Span {
    pub fn range(&self) -> Range<usize> {
        let to = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
        to(self.start)..to(self.end)
    }
}

const TAB: usize = 8;

/// Split `line` into plain text and style spans, starting in `style` (where
/// the stream's previous line left off). Returns the style the line ends in.
pub fn parse(line: &str, style: SgrStyle) -> (String, Vec<Span>, SgrStyle) {
    let mut collector = Collector {
        text: String::with_capacity(line.len()),
        style,
        ..Collector::default()
    };
    let mut parser: Processor = Processor::new();
    parser.advance(&mut collector, line.as_bytes());
    (collector.text, collector.spans, collector.style)
}

/// Receives `vte`'s decoded output for one line.
#[derive(Default)]
struct Collector {
    text: String,
    spans: Vec<Span>,
    style: SgrStyle,
    column: usize,
}

impl Collector {
    fn push(&mut self, s: &str) {
        let start = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        self.text.push_str(s);
        let end = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        if self.style.is_plain() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.end == start && last.style == self.style => last.end = end,
            _ => self.spans.push(Span {
                start,
                end,
                style: self.style,
            }),
        }
    }
}

impl Handler for Collector {
    fn input(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.push(c.encode_utf8(&mut buf));
        self.column = self.column.saturating_add(1);
    }

    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            let pad = TAB.saturating_sub(self.column % TAB);
            self.push(&" ".repeat(pad));
            self.column = self.column.saturating_add(pad);
        }
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        let style = &mut self.style;
        match attr {
            Attr::Reset => *style = SgrStyle::default(),
            Attr::Bold => style.bold = true,
            Attr::Dim => style.dim = true,
            Attr::Italic => style.italic = true,
            Attr::Underline
            | Attr::DoubleUnderline
            | Attr::Undercurl
            | Attr::DottedUnderline
            | Attr::DashedUnderline => style.underline = true,
            Attr::CancelBold => style.bold = false,
            Attr::CancelBoldDim => (style.bold, style.dim) = (false, false),
            Attr::CancelItalic => style.italic = false,
            Attr::CancelUnderline => style.underline = false,
            Attr::Foreground(color) => style.fg = sgr_color(color),
            Attr::Background(color) => style.bg = sgr_color(color),
            _ => {}
        }
    }
}

/// Merge style spans with search-match ranges into the sorted,
/// non-overlapping segments gpui text highlighting needs. Each segment
/// carries its style (if any) and whether it is inside a match.
pub fn segments(
    spans: &[Span],
    matches: &[Range<usize>],
) -> Vec<(Range<usize>, Option<SgrStyle>, bool)> {
    let mut cuts: Vec<usize> = spans
        .iter()
        .flat_map(|s| {
            let r = s.range();
            [r.start, r.end]
        })
        .chain(matches.iter().flat_map(|m| [m.start, m.end]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let (&start, &end) = (pair.first()?, pair.get(1)?);
            let style = spans
                .iter()
                .find(|s| s.range().contains(&start))
                .map(|s| s.style);
            let hit = matches.iter().any(|m| m.contains(&start));
            (style.is_some() || hit).then_some((start..end, style, hit))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &str) -> (String, Vec<Span>) {
        let (text, spans, _) = parse(line, SgrStyle::default());
        (text, spans)
    }

    #[test]
    fn styles_carry_across_lines() {
        let (_, first, open) = parse("\x1b[31mred", SgrStyle::default());
        assert_eq!(first.len(), 1);
        let (_, second, closed) = parse("still red\x1b[0m", open);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].style.fg, Some(AnsiColor::Indexed(1)));
        assert_eq!(closed, SgrStyle::default());
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(plain("hello"), ("hello".into(), vec![]));
    }

    #[test]
    fn colours_and_reset() {
        let (text, spans) = plain("\x1b[32mINFO\x1b[0m ready \x1b[1;31mERR\x1b[m");
        assert_eq!(text, "INFO ready ERR");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].range(), 0..4);
        assert_eq!(spans[0].style.fg, Some(AnsiColor::Indexed(2)));
        assert_eq!(spans[1].range(), 11..14);
        assert!(spans[1].style.bold);
        assert_eq!(spans[1].style.fg, Some(AnsiColor::Indexed(1)));
    }

    #[test]
    fn bright_256_and_truecolour() {
        let (_, spans) = plain("\x1b[91ma\x1b[38;5;208mb\x1b[48;2;1;2;3mc\x1b[38:2:9:8:7md");
        assert_eq!(spans[0].style.fg, Some(AnsiColor::Indexed(9)));
        assert_eq!(spans[1].style.fg, Some(AnsiColor::Indexed(208)));
        assert_eq!(spans[2].style.bg, Some(AnsiColor::Rgb(1, 2, 3)));
        assert_eq!(spans[3].style.fg, Some(AnsiColor::Rgb(9, 8, 7)));
    }

    #[test]
    fn other_sequences_and_controls_are_dropped() {
        let (text, spans) = plain("\x1b]0;title\x07a\x1b[2Kb\r\x1b]8;;x\x1b\\c\x01");
        assert_eq!(text, "abc");
        assert_eq!(spans.len(), 0);
    }

    #[test]
    fn default_colour_and_underline_variants() {
        let (text, spans) = plain("\x1b[31ma\x1b[39mb\x1b[4:3mc\x1b[24md");
        assert_eq!(text, "abcd");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].range(), 0..1);
        assert_eq!(spans[1].range(), 2..3);
        assert!(spans[1].style.underline);
    }

    #[test]
    fn tabs_expand_to_eight_columns() {
        assert_eq!(plain("ab\tc").0, "ab      c");
        assert_eq!(plain("\x1b[1mab\x1b[0m\tc").0, "ab      c");
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let (text, spans) = plain("é\x1b[4mü\x1b[0m");
        assert_eq!(text, "éü");
        assert_eq!(spans[0].range(), 2..4);
    }

    #[test]
    fn segments_split_on_every_boundary() {
        let (_, spans) = plain("\x1b[31mabcdef\x1b[0mgh");
        let matches = [4..8, 8..8];
        let segs = segments(&spans, &matches);
        let shape: Vec<_> = segs
            .iter()
            .map(|(r, s, m)| (r.clone(), s.is_some(), *m))
            .collect();
        assert_eq!(
            shape,
            vec![(0..4, true, false), (4..6, true, true), (6..8, false, true)]
        );
        assert_eq!(segments(&[], &[]).len(), 0);
    }
}

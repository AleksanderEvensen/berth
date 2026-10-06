//! ANSI SGR → styled spans for log lines.
//!
//! Lines are parsed once, on arrival: the plain text (what search and copy
//! see) plus style spans over it. Colour/attribute SGR codes become spans;
//! every other escape sequence (cursor moves, OSC titles, …) is dropped, as
//! are carriage returns and other control characters. Tabs expand to the
//! next multiple of eight columns.

use std::ops::Range;

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

/// Split `line` into plain text and style spans.
pub fn parse(line: &str) -> (String, Vec<Span>) {
    let mut text = String::with_capacity(line.len());
    let mut spans: Vec<Span> = Vec::new();
    let mut style = SgrStyle::default();
    let mut column = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    for p in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&p) {
                            if p == 'm' {
                                apply_sgr(&mut style, &params);
                            }
                            break;
                        }
                        params.push(p);
                    }
                }
                // OSC: up to BEL or ST (ESC \).
                Some(']') => {
                    while let Some(p) = chars.next() {
                        if p == '\x07' || (p == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let pad = TAB.saturating_sub(column % TAB);
                push(&mut text, &mut spans, style, &" ".repeat(pad));
                column = column.saturating_add(pad);
            }
            c if c.is_control() => {}
            c => {
                let mut buf = [0u8; 4];
                push(&mut text, &mut spans, style, c.encode_utf8(&mut buf));
                column = column.saturating_add(1);
            }
        }
    }
    (text, spans)
}

fn push(text: &mut String, spans: &mut Vec<Span>, style: SgrStyle, s: &str) {
    let start = u32::try_from(text.len()).unwrap_or(u32::MAX);
    text.push_str(s);
    let end = u32::try_from(text.len()).unwrap_or(u32::MAX);
    if style.is_plain() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.end == start && last.style == style => last.end = end,
        _ => spans.push(Span { start, end, style }),
    }
}

fn apply_sgr(style: &mut SgrStyle, params: &str) {
    let mut codes = params.split([';', ':']).map(|p| {
        if p.is_empty() {
            Some(0)
        } else {
            p.parse::<u16>().ok()
        }
    });
    while let Some(code) = codes.next() {
        let Some(code) = code else { return };
        match code {
            0 => *style = SgrStyle::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            30..=37 => style.fg = indexed(code, 30),
            90..=97 => style.fg = indexed(code, 82),
            40..=47 => style.bg = indexed(code, 40),
            100..=107 => style.bg = indexed(code, 92),
            39 => style.fg = None,
            49 => style.bg = None,
            38 => style.fg = extended(&mut codes),
            48 => style.bg = extended(&mut codes),
            _ => {}
        }
    }
}

fn indexed(code: u16, base: u16) -> Option<AnsiColor> {
    u8::try_from(code.checked_sub(base)?)
        .ok()
        .map(AnsiColor::Indexed)
}

/// `5;n` (256-colour) or `2;r;g;b` (truecolour) after a 38/48.
fn extended(codes: &mut impl Iterator<Item = Option<u16>>) -> Option<AnsiColor> {
    let mut next = || codes.next().flatten().and_then(|v| u8::try_from(v).ok());
    match next()? {
        5 => next().map(AnsiColor::Indexed),
        2 => Some(AnsiColor::Rgb(next()?, next()?, next()?)),
        _ => None,
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

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(parse("hello"), ("hello".into(), vec![]));
    }

    #[test]
    fn colours_and_reset() {
        let (text, spans) = parse("\x1b[32mINFO\x1b[0m ready \x1b[1;31mERR\x1b[m");
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
        let (_, spans) = parse("\x1b[91ma\x1b[38;5;208mb\x1b[48;2;1;2;3mc\x1b[38:2:9:8:7md");
        assert_eq!(spans[0].style.fg, Some(AnsiColor::Indexed(9)));
        assert_eq!(spans[1].style.fg, Some(AnsiColor::Indexed(208)));
        assert_eq!(spans[2].style.bg, Some(AnsiColor::Rgb(1, 2, 3)));
        assert_eq!(spans[3].style.fg, Some(AnsiColor::Rgb(9, 8, 7)));
    }

    #[test]
    fn other_sequences_and_controls_are_dropped() {
        let (text, spans) = parse("\x1b]0;title\x07a\x1b[2Kb\r\x1b]8;;x\x1b\\c\x01");
        assert_eq!(text, "abc");
        assert_eq!(spans.len(), 0);
    }

    #[test]
    fn tabs_expand_to_eight_columns() {
        assert_eq!(parse("ab\tc").0, "ab      c");
        assert_eq!(parse("\x1b[1mab\x1b[0m\tc").0, "ab      c");
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let (text, spans) = parse("é\x1b[4mü\x1b[0m");
        assert_eq!(text, "éü");
        assert_eq!(spans[0].range(), 2..4);
    }

    #[test]
    fn segments_split_on_every_boundary() {
        let (_, spans) = parse("\x1b[31mabcdef\x1b[0mgh");
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

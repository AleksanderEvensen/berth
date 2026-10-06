//! A bounded log ring buffer with an incrementally maintained filter
//! (NFR-P4: 1 000 000 lines, constant-time access for virtualized rows).
//!
//! Entries are addressed by a monotonically increasing sequence number, so
//! evicting from the front never invalidates the filter's match list: stale
//! sequence numbers simply fall off its front too.

use std::collections::VecDeque;
use std::ops::Range;

use berth_docker::{LogLine, LogStream};

use super::ansi::{self, Span};

pub const CAPACITY: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub stream: LogStream,
    pub timestamp: Option<i64>,
    /// Plain text, escape sequences removed.
    pub text: Box<str>,
    /// SGR styles over `text`; empty (no allocation) for uncoloured lines.
    pub spans: Box<[Span]>,
}

impl LogEntry {
    pub fn new(line: &LogLine) -> Self {
        let (text, spans) = ansi::parse(&line.line);
        Self {
            stream: line.stream,
            timestamp: line.timestamp,
            text: text.into_boxed_str(),
            spans: spans.into_boxed_slice(),
        }
    }
}

struct Filter {
    /// Lower-cased query.
    needle: String,
    /// Sequence numbers of matching entries, ascending.
    matches: VecDeque<u64>,
}

pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    /// Sequence number of `entries[0]`.
    first_seq: u64,
    capacity: usize,
    filter: Option<Filter>,
}

impl LogBuffer {
    pub const fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            first_seq: 0,
            capacity,
            filter: None,
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= self.capacity.max(1) {
            self.entries.pop_front();
            self.first_seq = self.first_seq.saturating_add(1);
            if let Some(filter) = &mut self.filter {
                while filter.matches.front().is_some_and(|&s| s < self.first_seq) {
                    filter.matches.pop_front();
                }
            }
        }
        let seq = self.next_seq();
        if let Some(filter) = &mut self.filter
            && contains_ci(&entry.text, &filter.needle)
        {
            filter.matches.push_back(seq);
        }
        self.entries.push_back(entry);
    }

    fn next_seq(&self) -> u64 {
        let len = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
        self.first_seq.saturating_add(len)
    }

    pub fn clear(&mut self) {
        self.first_seq = self.next_seq();
        self.entries.clear();
        if let Some(filter) = &mut self.filter {
            filter.matches.clear();
        }
    }

    /// Total lines held, ignoring the filter.
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// Show only lines containing `query` (case-insensitive); empty clears.
    /// Rescans the whole buffer, so callers debounce keystrokes.
    pub fn set_filter(&mut self, query: &str) {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            self.filter = None;
            return;
        }
        let first = self.first_seq;
        let matches = self
            .entries
            .iter()
            .zip(first..)
            .filter(|(entry, _)| contains_ci(&entry.text, &needle))
            .map(|(_, seq)| seq)
            .collect();
        self.filter = Some(Filter { needle, matches });
    }

    pub fn needle(&self) -> Option<&str> {
        self.filter.as_ref().map(|f| f.needle.as_str())
    }

    /// Lines currently shown (after the filter).
    pub fn len(&self) -> usize {
        self.filter
            .as_ref()
            .map_or(self.entries.len(), |f| f.matches.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `ix`th shown line.
    pub fn get(&self, ix: usize) -> Option<&LogEntry> {
        let offset = match &self.filter {
            None => ix,
            Some(filter) => {
                let seq = *filter.matches.get(ix)?;
                usize::try_from(seq.checked_sub(self.first_seq)?).ok()?
            }
        };
        self.entries.get(offset)
    }

    pub fn shown(&self) -> impl Iterator<Item = &LogEntry> {
        (0..self.len()).filter_map(|ix| self.get(ix))
    }
}

/// Case-insensitive substring test; `needle` is already lower-cased.
/// ASCII needles (the common case) compare bytes without allocating.
pub fn contains_ci(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        let needle = needle.as_bytes();
        haystack
            .as_bytes()
            .windows(needle.len().max(1))
            .any(|w| w.eq_ignore_ascii_case(needle))
    } else {
        haystack.to_lowercase().contains(needle)
    }
}

/// Byte ranges of every match of the lower-cased `needle` in `text`, for
/// highlighting. Only ASCII needles are located exactly (case folding can
/// change byte lengths otherwise); others highlight nothing.
pub fn match_ranges(text: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() || !needle.is_ascii() {
        return Vec::new();
    }
    let (hay, pat) = (text.as_bytes(), needle.as_bytes());
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(window) = hay.get(at..) {
        let Some(found) = window
            .windows(pat.len())
            .position(|w| w.eq_ignore_ascii_case(pat))
        else {
            break;
        };
        let start = at.saturating_add(found);
        let end = start.saturating_add(pat.len());
        out.push(start..end);
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> LogEntry {
        LogEntry::new(&LogLine {
            stream: LogStream::Stdout,
            timestamp: None,
            line: text.into(),
        })
    }

    fn shown(buffer: &LogBuffer) -> Vec<String> {
        buffer.shown().map(|e| e.text.to_string()).collect()
    }

    #[test]
    fn ring_evicts_oldest() {
        let mut buffer = LogBuffer::new(3);
        for n in 0..5 {
            buffer.push(entry(&n.to_string()));
        }
        assert_eq!(shown(&buffer), ["2", "3", "4"]);
        assert_eq!(buffer.get(3).map(|e| &*e.text), None);
    }

    #[test]
    fn filter_tracks_pushes_and_evictions() {
        let mut buffer = LogBuffer::new(4);
        for text in ["GET /a", "post /b", "GET /c"] {
            buffer.push(entry(text));
        }
        buffer.set_filter("get");
        assert_eq!(shown(&buffer), ["GET /a", "GET /c"]);
        buffer.push(entry("get /d"));
        buffer.push(entry("noise"));
        // "GET /a" was evicted.
        assert_eq!(shown(&buffer), ["GET /c", "get /d"]);
        assert_eq!(buffer.total(), 4);
        buffer.set_filter("");
        assert_eq!(buffer.len(), 4);
    }

    #[test]
    fn filter_ignores_escape_codes() {
        let mut buffer = LogBuffer::new(10);
        buffer.push(entry("\x1b[32mINFO\x1b[0m ok"));
        buffer.set_filter("32m");
        assert_eq!(buffer.len(), 0);
        buffer.set_filter("info ok");
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn clear_keeps_the_filter_active() {
        let mut buffer = LogBuffer::new(10);
        buffer.push(entry("a"));
        buffer.set_filter("a");
        buffer.clear();
        assert_eq!(buffer.len(), 0);
        buffer.push(entry("ab"));
        buffer.push(entry("b"));
        assert_eq!(shown(&buffer), ["ab"]);
    }

    #[test]
    fn case_insensitive_matching() {
        assert!(contains_ci("Hello World", "o w"));
        assert!(contains_ci("ÉCOLE", "école"));
        assert!(!contains_ci("abc", "abcd"));
        assert_eq!(match_ranges("aXa xa", "xa"), [1..3, 4..6]);
        assert_eq!(match_ranges("abc", "").len(), 0);
    }

    #[test]
    fn holds_a_million_lines() {
        let mut buffer = LogBuffer::new(CAPACITY);
        for n in 0..CAPACITY + 10 {
            buffer.push(LogEntry {
                stream: LogStream::Stdout,
                timestamp: None,
                text: if n % 1000 == 0 {
                    "hit".into()
                } else {
                    "x".into()
                },
                spans: Box::new([]),
            });
        }
        assert_eq!(buffer.total(), CAPACITY);
        buffer.set_filter("hit");
        assert_eq!(buffer.len(), 1000);
    }
}

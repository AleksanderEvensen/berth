//! A bounded log ring buffer with an incrementally maintained filter
//! (NFR-P4: 1 000 000 lines, constant-time access for virtualized rows).
//!
//! Entries are addressed by a monotonically increasing sequence number, so
//! evicting from the front never invalidates the filter's match list: stale
//! sequence numbers simply fall off its front too.

use std::collections::VecDeque;
use std::ops::Range;

use berth_docker::{LogLine, LogStream};

use aho_corasick::AhoCorasick;

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
    needle: Needle,
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
            && filter.needle.is_match(&entry.text)
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
        let Some(needle) = Needle::new(query) else {
            self.filter = None;
            return;
        };
        let first = self.first_seq;
        let matches = self
            .entries
            .iter()
            .zip(first..)
            .filter(|(entry, _)| needle.is_match(&entry.text))
            .map(|(_, seq)| seq)
            .collect();
        self.filter = Some(Filter { needle, matches });
    }

    pub fn needle(&self) -> Option<&Needle> {
        self.filter.as_ref().map(|f| &f.needle)
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

/// A case-insensitive filter query, compiled once.
///
/// ASCII queries (the common case) use an ASCII-case-insensitive
/// Aho-Corasick matcher, which also locates matches for highlighting. Other
/// queries fall back to Unicode lower-casing and highlight nothing, since
/// case folding can change byte lengths.
pub struct Needle {
    lower: String,
    ascii: Option<AhoCorasick>,
}

impl Needle {
    /// `None` for an empty query.
    pub fn new(query: &str) -> Option<Self> {
        let lower = query.to_lowercase();
        if lower.is_empty() {
            return None;
        }
        let ascii = lower.is_ascii().then(|| {
            AhoCorasick::builder()
                .ascii_case_insensitive(true)
                .build([&lower])
        });
        let ascii = match ascii {
            Some(Ok(matcher)) => Some(matcher),
            Some(Err(err)) => {
                log::warn!("log filter falls back to lower-casing: {err}");
                None
            }
            None => None,
        };
        Some(Self { lower, ascii })
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.ascii.as_ref().map_or_else(
            || text.to_lowercase().contains(&self.lower),
            |matcher| matcher.is_match(text),
        )
    }

    /// Byte ranges of every match in `text`, for highlighting.
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        self.ascii.as_ref().map_or_else(Vec::new, |matcher| {
            matcher.find_iter(text).map(|m| m.range()).collect()
        })
    }
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
        let needle = |q: &str| Needle::new(q).unwrap();
        assert!(needle("O W").is_match("Hello World"));
        assert!(needle("école").is_match("ÉCOLE"));
        assert!(!needle("abcd").is_match("abc"));
        assert_eq!(needle("xa").ranges("aXa xa"), [1..3, 4..6]);
        assert_eq!(needle("école").ranges("ÉCOLE").len(), 0);
        assert!(Needle::new("").is_none());
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

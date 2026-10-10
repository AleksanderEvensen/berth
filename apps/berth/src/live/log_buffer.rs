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

use super::ansi::{self, SgrStyle, Span};

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
    /// `style` is the SGR state its stream ended the previous line in; it is
    /// updated, since colours set on one line carry over to the next.
    pub fn new(line: &LogLine, style: &mut SgrStyle) -> Self {
        let (text, spans, end) = ansi::parse(&line.line, *style);
        *style = end;
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
    /// Matches among the history present when the filter was set, found by
    /// [`LogBuffer::scan`] in bounded steps; ascending.
    matches: VecDeque<u64>,
    /// Matches among lines pushed since; ascending, all ≥ `scan_end`.
    live: VecDeque<u64>,
    /// Next history entry to check, and where history ends.
    scan_from: u64,
    scan_end: u64,
}

impl Filter {
    fn len(&self) -> usize {
        self.matches.len().saturating_add(self.live.len())
    }

    fn get(&self, ix: usize) -> Option<u64> {
        ix.checked_sub(self.matches.len()).map_or_else(
            || self.matches.get(ix).copied(),
            |live| self.live.get(live).copied(),
        )
    }

    /// Drop the match for `seq` if it is the oldest; returns whether it was.
    fn evict(&mut self, seq: u64) -> bool {
        let list = if self.matches.is_empty() {
            &mut self.live
        } else {
            &mut self.matches
        };
        let hit = list.front() == Some(&seq);
        if hit {
            list.pop_front();
        }
        hit
    }
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

    /// Append a line, evicting the oldest at capacity. Returns whether the
    /// evicted line was shown, i.e. every shown row moved up by one.
    pub fn push(&mut self, entry: LogEntry) -> bool {
        let mut evicted_shown = false;
        if self.entries.len() >= self.capacity.max(1) {
            self.entries.pop_front();
            let evicted = self.first_seq;
            self.first_seq = self.first_seq.saturating_add(1);
            evicted_shown = self.filter.as_mut().is_none_or(|f| f.evict(evicted));
            if let Some(filter) = &mut self.filter {
                filter.scan_from = filter.scan_from.max(self.first_seq);
            }
        }
        let seq = self.next_seq();
        if let Some(filter) = &mut self.filter
            && filter.needle.is_match(&entry.text)
        {
            filter.live.push_back(seq);
        }
        self.entries.push_back(entry);
        evicted_shown
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
            filter.live.clear();
            filter.scan_from = self.first_seq;
            filter.scan_end = self.first_seq;
        }
    }

    /// Total lines held, ignoring the filter.
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// Show only lines containing `query` (case-insensitive); empty clears.
    /// The existing history is checked by [`Self::scan`], so a million lines
    /// never block the UI in one go; new lines are checked as they arrive.
    pub fn set_filter(&mut self, query: &str) {
        self.filter = Needle::new(query).map(|needle| Filter {
            needle,
            matches: VecDeque::new(),
            live: VecDeque::new(),
            scan_from: self.first_seq,
            scan_end: self.next_seq(),
        });
    }

    /// Check up to `budget` history lines against the filter. Returns
    /// `true` once the history is fully scanned (or there is no filter).
    pub fn scan(&mut self, budget: usize) -> bool {
        let Some(filter) = &mut self.filter else {
            return true;
        };
        let start =
            usize::try_from(filter.scan_from.saturating_sub(self.first_seq)).unwrap_or(usize::MAX);
        let end = usize::try_from(filter.scan_end.saturating_sub(self.first_seq))
            .unwrap_or(usize::MAX)
            .min(self.entries.len());
        let stop = start.saturating_add(budget).min(end);
        for (entry, seq) in self
            .entries
            .range(start.min(stop)..stop)
            .zip(filter.scan_from..)
        {
            if filter.needle.is_match(&entry.text) {
                filter.matches.push_back(seq);
            }
        }
        let checked = u64::try_from(stop.saturating_sub(start)).unwrap_or(u64::MAX);
        filter.scan_from = filter.scan_from.saturating_add(checked);
        filter.scan_from >= filter.scan_end
    }

    pub fn scanning(&self) -> bool {
        self.filter
            .as_ref()
            .is_some_and(|f| f.scan_from < f.scan_end)
    }

    pub fn needle(&self) -> Option<&Needle> {
        self.filter.as_ref().map(|f| &f.needle)
    }

    /// Lines currently shown (after the filter).
    pub fn len(&self) -> usize {
        self.filter.as_ref().map_or(self.entries.len(), Filter::len)
    }

    /// Shown lines that come from the history scan; [`Self::scan`] inserts
    /// new matches at this index, ahead of lines pushed since.
    pub fn scanned_len(&self) -> usize {
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
                let seq = filter.get(ix)?;
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
        LogEntry::new(
            &LogLine {
                stream: LogStream::Stdout,
                timestamp: None,
                line: text.into(),
            },
            &mut SgrStyle::default(),
        )
    }

    fn filter(buffer: &mut LogBuffer, query: &str) {
        buffer.set_filter(query);
        assert!(buffer.scan(usize::MAX));
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
        filter(&mut buffer, "get");
        assert_eq!(shown(&buffer), ["GET /a", "GET /c"]);
        buffer.push(entry("get /d"));
        buffer.push(entry("noise"));
        // "GET /a" was evicted.
        assert_eq!(shown(&buffer), ["GET /c", "get /d"]);
        assert_eq!(buffer.total(), 4);
        filter(&mut buffer, "");
        assert_eq!(buffer.len(), 4);
    }

    #[test]
    fn filter_ignores_escape_codes() {
        let mut buffer = LogBuffer::new(10);
        buffer.push(entry("\x1b[32mINFO\x1b[0m ok"));
        filter(&mut buffer, "32m");
        assert_eq!(buffer.len(), 0);
        filter(&mut buffer, "info ok");
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn clear_keeps_the_filter_active() {
        let mut buffer = LogBuffer::new(10);
        buffer.push(entry("a"));
        filter(&mut buffer, "a");
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
    fn chunked_scan_interleaves_with_new_lines() {
        let mut buffer = LogBuffer::new(100);
        for n in 0..10 {
            buffer.push(entry(&format!("hit {n}")));
        }
        buffer.set_filter("hit");
        assert!(buffer.scanning());
        assert!(!buffer.scan(4));
        buffer.push(entry("hit new"));
        assert_eq!(buffer.len(), 5, "4 scanned + 1 live");
        assert_eq!(buffer.scanned_len(), 4);
        assert!(buffer.scan(100));
        assert_eq!(buffer.scanned_len(), 10);
        assert!(!buffer.scanning());
        let shown: Vec<_> = buffer.shown().map(|e| e.text.to_string()).collect();
        assert_eq!(shown.first().map(String::as_str), Some("hit 0"));
        assert_eq!(shown.last().map(String::as_str), Some("hit new"));
        assert_eq!(shown.len(), 11);
    }

    #[test]
    fn push_reports_evicting_a_shown_row() {
        let mut buffer = LogBuffer::new(2);
        assert!(!buffer.push(entry("a")));
        assert!(!buffer.push(entry("b")));
        assert!(buffer.push(entry("c")), "unfiltered: every row is shown");
        filter(&mut buffer, "c");
        assert!(!buffer.push(entry("d")), "evicted b wasn't shown");
        assert!(buffer.push(entry("e")), "evicted c was shown");
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
        filter(&mut buffer, "hit");
        assert_eq!(buffer.len(), 1000);
    }
}

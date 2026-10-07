use crate::{LogLine, LogStream};

// Bollard's generated query narrows Docker's seconds to i32. For a later
// `since`, ask for the nearest representable lower bound and filter stamped
// lines locally. An earlier bound requests all history instead of wrapping.
pub(super) fn since_seconds(since: Option<i64>) -> i32 {
    let since = since.unwrap_or_default();
    i32::try_from(since).unwrap_or(if since > 0 { i32::MAX } else { 0 })
}
pub(super) const fn after_since(line: &LogLine, since: Option<i64>) -> bool {
    match (line.timestamp, since) {
        (Some(time), Some(since)) => time >= since.saturating_mul(1_000_000_000),
        _ => true,
    }
}

/// Longest line kept whole. A process writing without newlines can't make
/// the assembler buffer without bound; longer runs are emitted in pieces.
const MAX_LINE: usize = 256 * 1024;

#[derive(Default)]
struct Pending {
    bytes: Vec<u8>,
    order: u64,
}
#[derive(Default)]
pub(super) struct Assembler {
    stdout: Pending,
    stderr: Pending,
    sequence: u64,
}
impl Assembler {
    /// Feed one chunk of output. `record` marks a multiplexed frame, which
    /// Docker writes per log record: when it continues a partial line (Docker
    /// splits long lines into 16 KiB records), its own timestamp is dropped.
    pub(super) fn push(&mut self, stream: LogStream, bytes: &[u8], record: bool) -> Vec<LogLine> {
        let pending = match stream {
            LogStream::Stdout => &mut self.stdout,
            LogStream::Stderr => &mut self.stderr,
        };
        let bytes = if record && !pending.bytes.is_empty() {
            strip_timestamp(bytes)
        } else {
            bytes
        };
        let mut lines = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if pending.bytes.is_empty() {
                pending.order = self.sequence;
                self.sequence = self.sequence.saturating_add(1);
            }
            pending.bytes.extend_from_slice(part);
            if pending.bytes.last() == Some(&b'\n') {
                pending.bytes.pop();
                lines.push(line(stream, std::mem::take(&mut pending.bytes)));
            } else if pending.bytes.len() >= MAX_LINE {
                lines.push(line(stream, std::mem::take(&mut pending.bytes)));
            }
        }
        lines
    }
    pub(super) fn finish(self) -> Vec<LogLine> {
        let mut pending = [
            (LogStream::Stdout, self.stdout),
            (LogStream::Stderr, self.stderr),
        ];
        pending.sort_by_key(|(_, p)| p.order);
        pending
            .into_iter()
            .filter(|(_, p)| !p.bytes.is_empty())
            .map(|(stream, p)| line(stream, p.bytes))
            .collect()
    }
}
/// Split Docker's `<RFC3339Nano> ` prefix off a record. `Some(None)` is a
/// timestamp-shaped prefix that doesn't parse (dropped all the same).
fn split_timestamp(bytes: &[u8]) -> Option<(Option<i64>, &[u8])> {
    let space = bytes.iter().take(64).position(|b| *b == b' ')?;
    let (prefix, rest) = (bytes.get(..space)?, bytes.get(space.saturating_add(1)..)?);
    let shaped = prefix.get(4) == Some(&b'-')
        && prefix.get(7) == Some(&b'-')
        && prefix.get(10) == Some(&b'T');
    let timestamp = std::str::from_utf8(prefix)
        .ok()
        .and_then(|p| chrono::DateTime::parse_from_rfc3339(p).ok())
        .and_then(|time| time.timestamp_nanos_opt());
    (timestamp.is_some() || shaped).then_some((timestamp, rest))
}

fn strip_timestamp(bytes: &[u8]) -> &[u8] {
    split_timestamp(bytes).map_or(bytes, |(_, rest)| rest)
}

fn line(stream: LogStream, mut bytes: Vec<u8>) -> LogLine {
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let (timestamp, text) = split_timestamp(&bytes).unwrap_or((None, &bytes));
    LogLine {
        stream,
        timestamp,
        line: String::from_utf8_lossy(text).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuation_records_drop_their_timestamp() {
        let mut a = Assembler::default();
        assert_eq!(
            a.push(LogStream::Stdout, b"2026-10-06T12:00:00Z abc", true),
            []
        );
        let lines = a.push(LogStream::Stdout, b"2026-10-06T12:00:00.5Z def\n", true);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line, "abcdef");
        assert_eq!(lines[0].timestamp, Some(1_791_288_000_000_000_000));
        // TTY output isn't record-framed: a mid-line timestamp is text.
        a.push(LogStream::Stdout, b"x ", false);
        let lines = a.push(LogStream::Stdout, b"2026-10-06T12:00:00Z y\n", false);
        assert_eq!(lines[0].line, "x 2026-10-06T12:00:00Z y");
    }
    #[test]
    fn unterminated_output_is_emitted_in_bounded_pieces() {
        let mut a = Assembler::default();
        let chunk = vec![b'x'; 64 * 1024];
        let mut emitted = Vec::new();
        for _ in 0..10 {
            emitted.extend(a.push(LogStream::Stdout, &chunk, false));
        }
        assert_eq!(emitted.len(), 2);
        assert!(emitted.iter().all(|l| l.line.len() == MAX_LINE));
        assert_eq!(a.finish()[0].line.len(), 2 * 64 * 1024);
    }
    #[test]
    fn partial_frames_multiline_crlf_and_independent_streams() {
        let mut a = Assembler::default();
        assert_eq!(
            a.push(
                LogStream::Stdout,
                b"2026-10-06T12:00:00.123456789Z hel",
                false
            ),
            []
        );
        let stderr = a.push(LogStream::Stderr, b"error\r\nnext\n", false);
        assert_eq!(
            stderr.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
            ["error", "next"]
        );
        assert!(stderr.iter().all(|l| l.stream == LogStream::Stderr));
        let stdout = a.push(
            LogStream::Stdout,
            b"lo\r\n\x1b[32mgreen\x1b[0m\n\npartial\r",
            false,
        );
        assert_eq!(
            stdout.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
            ["hello", "\x1b[32mgreen\x1b[0m", ""]
        );
        assert_eq!(
            stdout.first().and_then(|l| l.timestamp),
            Some(1_791_288_000_123_456_789)
        );
        assert_eq!(a.finish().first().map(|l| l.line.as_str()), Some("partial"));
    }
    #[test]
    fn utf8_split_lossy_bytes_and_remainders() {
        let mut a = Assembler::default();
        a.push(LogStream::Stderr, b"\xf0\x9f", false);
        a.push(LogStream::Stdout, b"last stdout", false);
        a.push(LogStream::Stderr, b"\x98\x80\xff", false);
        let lines = a.finish();
        assert_eq!(
            lines.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
            ["😀�", "last stdout"]
        );
    }
    #[test]
    fn since_bounds_do_not_wrap() {
        assert_eq!(since_seconds(None), 0);
        assert_eq!(since_seconds(Some(123)), 123);
        assert_eq!(since_seconds(Some(i64::MAX)), i32::MAX);
        assert_eq!(since_seconds(Some(i64::MIN)), 0);
        let value = line(LogStream::Stdout, b"1970-01-01T00:00:01Z hello".to_vec());
        assert!(after_since(&value, Some(1)));
        assert!(!after_since(&value, Some(2)));
        assert!(!after_since(&value, Some(i64::MAX)));
        assert!(after_since(&value, Some(i64::MIN)));
    }
    #[test]
    fn timestamps_with_fraction_offset_invalid_or_missing() {
        for (raw, expected, text) in [
            ("1970-01-01T00:00:01Z one", Some(1_000_000_000), "one"),
            (
                "1970-01-01T01:00:01.000000001+01:00 two",
                Some(1_000_000_001),
                "two",
            ),
            ("2026-99-06T12:00:00Z hello", None, "hello"),
            ("hello world", None, "hello world"),
            ("9999-01-01T00:00:00Z overflow", None, "overflow"),
        ] {
            let value = line(LogStream::Stdout, raw.as_bytes().to_vec());
            assert_eq!((value.timestamp, value.line.as_str()), (expected, text));
        }
    }
}

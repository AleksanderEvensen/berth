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
    pub(super) fn push(&mut self, stream: LogStream, bytes: &[u8]) -> Vec<LogLine> {
        let pending = match stream {
            LogStream::Stdout => &mut self.stdout,
            LogStream::Stderr => &mut self.stderr,
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
fn line(stream: LogStream, mut bytes: Vec<u8>) -> LogLine {
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let text = String::from_utf8_lossy(&bytes);
    let (timestamp, text) = text.split_once(' ').map_or_else(
        || (None, text.as_ref()),
        |(prefix, rest)| {
            let timestamp = chrono::DateTime::parse_from_rfc3339(prefix)
                .ok()
                .and_then(|time| time.timestamp_nanos_opt());
            if timestamp.is_some()
                || (prefix.as_bytes().get(4) == Some(&b'-')
                    && prefix.as_bytes().get(7) == Some(&b'-')
                    && prefix.as_bytes().get(10) == Some(&b'T'))
            {
                (timestamp, rest)
            } else {
                (None, text.as_ref())
            }
        },
    );
    LogLine {
        stream,
        timestamp,
        line: text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_frames_multiline_crlf_and_independent_streams() {
        let mut a = Assembler::default();
        assert_eq!(
            a.push(LogStream::Stdout, b"2026-10-06T12:00:00.123456789Z hel"),
            []
        );
        let stderr = a.push(LogStream::Stderr, b"error\r\nnext\n");
        assert_eq!(
            stderr.iter().map(|l| l.line.as_str()).collect::<Vec<_>>(),
            ["error", "next"]
        );
        assert!(stderr.iter().all(|l| l.stream == LogStream::Stderr));
        let stdout = a.push(
            LogStream::Stdout,
            b"lo\r\n\x1b[32mgreen\x1b[0m\n\npartial\r",
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
        a.push(LogStream::Stderr, b"\xf0\x9f");
        a.push(LogStream::Stdout, b"last stdout");
        a.push(LogStream::Stderr, b"\x98\x80\xff");
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

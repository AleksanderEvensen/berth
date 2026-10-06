//! Human formatting for sizes and ages.

use std::time::{SystemTime, UNIX_EPOCH};

/// Decimal units, like Docker and file managers: `68.2 MB`, `1.66 GB`.
/// Three significant digits, integer arithmetic only.
#[must_use]
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let n = u128::from(n);
    let mut unit = 0usize;
    let mut divisor: u128 = 1;
    while unit < UNITS.len().saturating_sub(1) && n >= divisor.saturating_mul(1000) {
        divisor = divisor.saturating_mul(1000);
        unit = unit.saturating_add(1);
    }
    let name = UNITS.get(unit).copied().unwrap_or("PB");
    if unit == 0 {
        return format!("{n} B");
    }
    // Value in hundredths, rounded.
    let hundredths = n
        .saturating_mul(100)
        .saturating_add(divisor.checked_div(2).unwrap_or(0))
        .checked_div(divisor)
        .unwrap_or(0);
    let div_round = |value: u128, by: u128| {
        value
            .saturating_add(by.checked_div(2).unwrap_or(0))
            .checked_div(by)
            .unwrap_or(0)
    };
    if hundredths >= 10_000 {
        format!("{} {name}", div_round(hundredths, 100))
    } else if hundredths >= 1_000 {
        let tenths = div_round(hundredths, 10);
        format!(
            "{}.{} {name}",
            tenths.checked_div(10).unwrap_or(0),
            tenths.checked_rem(10).unwrap_or(0)
        )
    } else {
        format!(
            "{}.{:02} {name}",
            hundredths.checked_div(100).unwrap_or(0),
            hundredths.checked_rem(100).unwrap_or(0)
        )
    }
}

#[must_use]
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// "just now", "5 minutes ago", "2 weeks ago", "1 year ago".
#[must_use]
pub fn age(created_unix: i64, now_unix: i64) -> String {
    const STEPS: [(i64, &str); 6] = [
        (365 * 86_400, "year"),
        (30 * 86_400, "month"),
        (7 * 86_400, "week"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ];
    let elapsed = now_unix.saturating_sub(created_unix).max(0);
    for (secs, unit) in STEPS {
        let count = elapsed.checked_div(secs).unwrap_or(0);
        if count >= 1 {
            let plural = if count == 1 { "" } else { "s" };
            return format!("{count} {unit}{plural} ago");
        }
    }
    "just now".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(94_000), "94.0 KB");
        assert_eq!(bytes(68_200_000), "68.2 MB");
        assert_eq!(bytes(291_100_000), "291 MB");
        assert_eq!(bytes(1_660_000_000), "1.66 GB");
    }

    #[test]
    fn formats_ages() {
        let now = 1_800_000_000;
        assert_eq!(age(now, now), "just now");
        assert_eq!(age(now - 300, now), "5 minutes ago");
        assert_eq!(age(now - 3_600, now), "1 hour ago");
        assert_eq!(age(now - 15 * 86_400, now), "2 weeks ago");
        assert_eq!(age(now - 400 * 86_400, now), "1 year ago");
        assert_eq!(age(now + 50, now), "just now");
    }
}

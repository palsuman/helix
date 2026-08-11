//! Timestamp formatting for log records.
//!
//! Logging needs exactly one thing from a calendar: a fixed-width RFC 3339
//! UTC string with millisecond precision. That is a dozen lines of
//! well-known arithmetic, so it is implemented here rather than adding a
//! date-time crate to the dependency surface (and to the audit that the
//! design document's pinning policy implies for every dependency).
//!
//! Fixed width matters beyond aesthetics: it makes lexicographic comparison
//! of two timestamps chronological, which is what the viewer's time-range
//! filter relies on.

use std::time::{SystemTime, UNIX_EPOCH};

const MILLIS_PER_DAY: i64 = 86_400_000;

/// The current time as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn now_rfc3339_millis() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    rfc3339_millis(millis)
}

/// Format Unix epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn rfc3339_millis(epoch_millis: i64) -> String {
    // Floor division, so a pre-1970 instant (clock skew, a restored
    // snapshot) formats as a real date instead of wrapping.
    let days = epoch_millis.div_euclid(MILLIS_PER_DAY);
    let millis_of_day = epoch_millis.rem_euclid(MILLIS_PER_DAY);

    let (year, month, day) = civil_from_days(days);
    let hour = millis_of_day / 3_600_000;
    let minute = (millis_of_day % 3_600_000) / 60_000;
    let second = (millis_of_day % 60_000) / 1_000;
    let milli = millis_of_day % 1_000;

    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z",
        year = year,
        month = month,
        day = day,
        hour = hour,
        minute = minute,
        second = second,
        milli = milli,
    )
}

/// Days since the Unix epoch to a proleptic Gregorian calendar date.
///
/// Howard Hinnant's `civil_from_days`, which is exact for the whole range of
/// an `i64` day count and needs no lookup tables.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    // Shift the era so the leap-day irregularity lands at the end of a
    // 400-year cycle rather than in the middle of one.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_epoch() {
        assert_eq!(rfc3339_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn formats_the_design_documents_example_instant() {
        // 2026-08-07T10:30:00.123Z
        let millis = 1_786_098_600_123;
        assert_eq!(rfc3339_millis(millis), "2026-08-07T10:30:00.123Z");
    }

    #[test]
    fn handles_a_leap_day() {
        // 2024-02-29T23:59:59.999Z
        let millis = 1_709_251_199_999;
        assert_eq!(rfc3339_millis(millis), "2024-02-29T23:59:59.999Z");
    }

    #[test]
    fn handles_an_instant_before_the_epoch_without_wrapping() {
        assert_eq!(rfc3339_millis(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn timestamps_are_fixed_width_so_string_order_is_time_order() {
        let earlier = rfc3339_millis(1_000);
        let later = rfc3339_millis(1_786_098_600_123);
        assert_eq!(earlier.len(), later.len());
        assert!(earlier < later);
    }

    #[test]
    fn now_produces_a_parseable_fixed_width_timestamp() {
        let now = now_rfc3339_millis();
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z'));
        assert!(now.as_str() > "2024-01-01T00:00:00.000Z");
    }
}

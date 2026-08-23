//! When a plan stops being applicable.
//!
//! The contract is explicit that a mismatched or expired plan has no effect, so
//! the check has to happen before the lock does any work. The consumer emits one
//! fixed shape — `datetime.isoformat(timespec="milliseconds")` with `+00:00`
//! replaced by `Z` — which is a fixed-width UTC instant:
//!
//! ```text
//! 2026-08-23T15:04:05.123Z
//! ```
//!
//! Parsing exactly that shape needs no calendar library and no dependency. A
//! value in any other shape is refused rather than interpreted: a plan whose
//! expiry could not be read is a plan whose expiry cannot be honoured, and
//! guessing it would be guessing about whether an effect is still authorized.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, parsed from the consumer's exact shape.
///
/// Returns `None` for any value that is not `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[must_use]
pub fn parse_utc_seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 24 {
        return None;
    }
    for (index, expected) in [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
    ] {
        if bytes.get(index) != Some(&expected) {
            return None;
        }
    }
    if bytes.last() != Some(&b'Z') {
        return None;
    }

    let year = number(text.get(0..4)?)?;
    let month = number(text.get(5..7)?)?;
    let day = number(text.get(8..10)?)?;
    let hour = number(text.get(11..13)?)?;
    let minute = number(text.get(14..16)?)?;
    let second = number(text.get(17..19)?)?;
    // Milliseconds are parsed to prove the shape, then dropped: expiry is
    // compared at second resolution and a sub-second claim would be false
    // precision about when authorization ends.
    let _milliseconds = number(text.get(20..23)?)?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    if day > days_in_month(year, month) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Whether `expires_at` is already in the past.
///
/// A value that cannot be parsed counts as expired. An unreadable expiry is not
/// evidence that a plan is still valid, and treating it as open-ended would make
/// a malformed field the most permissive one.
#[must_use]
pub fn has_expired(expires_at: &str, now: SystemTime) -> bool {
    let Some(deadline) = parse_utc_seconds(expires_at) else {
        return true;
    };
    let Ok(elapsed) = now.duration_since(UNIX_EPOCH) else {
        // A clock before 1970 cannot be compared against a UTC deadline.
        return true;
    };
    let seconds = i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX);
    seconds > deadline
}

fn number(text: &str) -> Option<i64> {
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

const fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to a civil date, by Howard Hinnant's algorithm.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_shift = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::time::Duration;

    use super::*;

    #[test]
    fn the_epoch_itself_is_zero() {
        assert_eq!(parse_utc_seconds("1970-01-01T00:00:00.000Z"), Some(0));
    }

    #[test]
    fn a_known_instant_matches_its_unix_seconds() {
        // 2026-08-23T15:04:05Z
        assert_eq!(
            parse_utc_seconds("2026-08-23T15:04:05.123Z"),
            Some(1_787_497_445)
        );
    }

    #[test]
    fn leap_days_are_accepted_only_in_leap_years() {
        assert!(parse_utc_seconds("2024-02-29T00:00:00.000Z").is_some());
        assert!(parse_utc_seconds("2026-02-29T00:00:00.000Z").is_none());
        assert!(parse_utc_seconds("2000-02-29T00:00:00.000Z").is_some());
        assert!(parse_utc_seconds("1900-02-29T00:00:00.000Z").is_none());
    }

    #[test]
    fn any_shape_but_the_consumers_is_refused_rather_than_interpreted() {
        for hostile in [
            "2026-08-23T15:04:05Z",          // no milliseconds
            "2026-08-23T15:04:05.123+00:00", // offset instead of Z
            "2026-08-23 15:04:05.123Z",      // space instead of T
            "2026-13-01T00:00:00.000Z",      // month 13
            "2026-08-32T00:00:00.000Z",      // day 32
            "2026-08-23T24:00:00.000Z",      // hour 24
            "2026-08-23T15:60:00.000Z",      // minute 60
            "not-a-timestamp---------",      // right length, wrong everything
            "",
        ] {
            assert!(parse_utc_seconds(hostile).is_none(), "accepted {hostile:?}");
        }
    }

    #[test]
    fn an_unparseable_expiry_counts_as_expired() {
        // Otherwise a malformed field would be the most permissive one.
        assert!(has_expired("whenever", SystemTime::UNIX_EPOCH));
        assert!(has_expired("", SystemTime::now()));
    }

    #[test]
    fn a_deadline_in_the_future_has_not_passed_and_one_in_the_past_has() {
        let now = UNIX_EPOCH + Duration::from_secs(1_787_497_445);
        assert!(!has_expired("2026-08-23T15:04:05.123Z", now));
        assert!(!has_expired("2026-08-23T15:04:06.000Z", now));
        assert!(has_expired("2026-08-23T15:04:04.999Z", now));
    }

    #[test]
    fn the_boundary_second_is_still_valid() {
        // The plan expires *after* its instant, not on it.
        let now = UNIX_EPOCH + Duration::from_secs(1_787_497_445);
        assert!(!has_expired("2026-08-23T15:04:05.000Z", now));
    }
}

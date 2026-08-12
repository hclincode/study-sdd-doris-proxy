//! UTC timestamp formatting.
//!
//! Log records need a human-readable timestamp alongside the epoch value. The
//! calendar conversion is short enough to carry directly rather than take on a
//! date-time dependency for one field.

use std::time::{SystemTime, UNIX_EPOCH};

/// A moment in time, captured once and rendered two ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub unix_ms: i64,
}

impl Timestamp {
    pub fn now() -> Self {
        let unix_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            // A clock before the epoch is pathological but must not panic.
            Err(e) => -(e.duration().as_millis() as i64),
        };
        Timestamp { unix_ms }
    }

    /// RFC 3339 in UTC with millisecond precision.
    pub fn to_rfc3339(self) -> String {
        // Euclidean division so that instants before the epoch borrow correctly
        // instead of truncating toward zero.
        let days = self.unix_ms.div_euclid(86_400_000);
        let ms_of_day = self.unix_ms.rem_euclid(86_400_000);
        let (y, m, d) = civil_from_days(days);
        let ms = ms_of_day % 1000;
        let secs_of_day = ms_of_day / 1000;
        let (h, min, s) = (
            secs_of_day / 3600,
            (secs_of_day % 3600) / 60,
            secs_of_day % 60,
        );
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
    }
}

/// Days since 1970-01-01 to a civil (year, month, day).
///
/// Howard Hinnant's `civil_from_days`, which is exact for the proleptic
/// Gregorian calendar over the range we care about.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> String {
        Timestamp { unix_ms: ms }.to_rfc3339()
    }

    #[test]
    fn formats_the_epoch() {
        assert_eq!(at(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn formats_a_leap_day() {
        assert_eq!(at(1_709_209_996_789), "2024-02-29T12:33:16.789Z");
    }

    #[test]
    fn formats_millisecond_precision() {
        assert_eq!(at(1_000), "1970-01-01T00:00:01.000Z");
        assert_eq!(at(1_234), "1970-01-01T00:00:01.234Z");
    }

    #[test]
    fn handles_a_pre_epoch_instant() {
        assert_eq!(at(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn crosses_year_boundaries() {
        assert_eq!(at(1_704_067_199_000), "2023-12-31T23:59:59.000Z");
        assert_eq!(at(1_704_067_200_000), "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn now_is_plausible() {
        let t = Timestamp::now();
        // Some time after 2020 and before 2100.
        assert!(t.unix_ms > 1_577_836_800_000);
        assert!(t.unix_ms < 4_102_444_800_000);
        assert!(t.to_rfc3339().ends_with('Z'));
    }
}

//! Wall-clock timestamps for write paths, without a chrono dependency.
//!
//! Quipu is bitemporal: every fact carries a valid-time. When a writer omits a
//! timestamp we must stamp it with the *real* current instant — defaulting to
//! the Unix epoch (1970) silently corrupts the time-travel log (hq-tb4). This
//! module converts the system clock to an ISO-8601 UTC string using the
//! proleptic-Gregorian days-from-civil algorithm, so dates are correct across
//! leap years (unlike the older approximate `/365,/30` formatter).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current UTC instant as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_iso(secs)
}

/// Format Unix-epoch seconds as an ISO-8601 UTC timestamp.
fn format_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Convert a count of days since the Unix epoch to a `(year, month, day)` civil
/// date. Howard Hinnant's `civil_from_days`, valid for the full proleptic
/// Gregorian range (leap years and century rules handled exactly).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_instants_round_trip() {
        // Well-known reference points (verified against date -u).
        assert_eq!(format_iso(1_609_459_200), "2021-01-01T00:00:00Z");
        assert_eq!(format_iso(1_700_000_000), "2023-11-14T22:13:20Z");
        // A leap day: 2024 is a leap year, so day 60 of 2024 is Feb 29.
        assert_eq!(format_iso(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn now_is_not_epoch() {
        // The whole point of hq-tb4: a real clock must not read as 1970.
        let ts = now_iso();
        assert!(
            ts.starts_with("20"),
            "expected a 21st-century year, got {ts}"
        );
        assert!(ts.ends_with('Z') && ts.len() == 20, "bad shape: {ts}");
    }
}

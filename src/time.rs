//! Wall-clock timestamps for write paths, without a chrono dependency.
//!
//! Quipu is bitemporal: every fact carries a valid-time. When a writer omits a
//! timestamp we must stamp it with the *real* current instant — defaulting to
//! the Unix epoch (1970) silently corrupts the time-travel log (hq-tb4). This
//! module converts the system clock to an ISO-8601 UTC string using the
//! proleptic-Gregorian days-from-civil algorithm, so dates are correct across
//! leap years (unlike the older approximate `/365,/30` formatter).

/// Seconds since the Unix epoch — the ONE place the crate reads the wall
/// clock (quipu-gsg).
///
/// `std::time::SystemTime::now()` panics on wasm32-unknown-unknown, so every
/// lib path routes through here: the wasm arm reads `js_sys::Date::now()`
/// (milliseconds as f64) instead. Design: `docs/design/wasm-support.md` §4.4.
#[cfg(not(target_arch = "wasm32"))]
pub fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(target_arch = "wasm32")]
pub fn epoch_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// A cancellation deadline usable on wasm32, where `std::time::Instant::now()`
/// panics (quipu-gsg). Native keeps `Instant` (monotonic); the wasm arm uses
/// `Date.now()` milliseconds — wall-clock, but a query budget does not need
/// monotonicity, it needs "roughly N ms from now".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deadline(
    #[cfg(not(target_arch = "wasm32"))] std::time::Instant,
    #[cfg(target_arch = "wasm32")] f64,
);

impl Deadline {
    /// A deadline `ms` milliseconds from now.
    #[must_use]
    pub fn after_millis(ms: u64) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self(std::time::Instant::now() + std::time::Duration::from_millis(ms))
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self(js_sys::Date::now() + ms as f64)
        }
    }

    /// Has the deadline passed?
    #[must_use]
    pub fn passed(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::time::Instant::now() >= self.0
        }
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::now() >= self.0
        }
    }

    /// Milliseconds remaining until this deadline, saturating at zero.
    #[must_use]
    pub fn remaining_millis(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            (self.0 - js_sys::Date::now()).max(0.0) as u64
        }
    }

    /// Milliseconds between `start` and this deadline (saturating) — the
    /// effective budget a query ran under, for timeout reporting.
    #[must_use]
    pub fn millis_from(&self, start: &Stopwatch) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.saturating_duration_since(start.0).as_millis()
        }
        #[cfg(target_arch = "wasm32")]
        {
            (self.0 - start.0).max(0.0) as u128
        }
    }
}

/// Elapsed-time measurement usable on wasm32 (quipu-gsg). Native wraps the
/// monotonic `Instant`; the wasm arm uses `Date.now()` milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Stopwatch(
    #[cfg(not(target_arch = "wasm32"))] std::time::Instant,
    #[cfg(target_arch = "wasm32")] f64,
);

impl Stopwatch {
    /// Start measuring now.
    #[must_use]
    pub fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self(std::time::Instant::now())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self(js_sys::Date::now())
        }
    }

    /// Milliseconds since [`Stopwatch::start`].
    #[must_use]
    pub fn elapsed_ms(&self) -> u128 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.0.elapsed().as_millis()
        }
        #[cfg(target_arch = "wasm32")]
        {
            (js_sys::Date::now() - self.0).max(0.0) as u128
        }
    }
}

/// Current UTC instant as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    format_iso(epoch_secs())
}

/// The instant `days` days before now, as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Retention cutoffs (quipu-9z9): `prune_events(&iso_days_ago(n))` deletes
/// what is older than n days. Saturates at the epoch rather than wrapping.
pub fn iso_days_ago(days: u64) -> String {
    format_iso(epoch_secs().saturating_sub(days.saturating_mul(86_400)))
}

/// Format Unix-epoch seconds as an ISO-8601 UTC timestamp.
fn format_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Days since the Unix epoch for a civil `(year, month, day)`.
///
/// Howard Hinnant's `days_from_civil`, the exact inverse of
/// [`civil_from_days`]. Needed because normalising an RFC 3339 timestamp with a
/// UTC offset is arithmetic on an instant, not string surgery: `+01:00` moves
/// the date across a boundary for any time in the first hour of the day.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Format Unix-epoch seconds as `YYYY-MM-DDTHH:MM:SSZ`, for signed instants.
///
/// [`format_iso`] takes `u64` and so cannot express a pre-1970 valid-time. A
/// historical valid-time is exactly the case this exists for — a datum may
/// legitimately be dated before the epoch even though nothing in this store was
/// ever *written* then. Floor division, not truncation: `-1 / 86_400` is 0 in
/// Rust, which would place 1969-12-31T23:59:59Z on 1970-01-01.
fn format_iso_signed(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3_600, (rem % 3_600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Parse an RFC 3339 date-time and re-render it as `YYYY-MM-DDTHH:MM:SSZ`, or
/// `None` if it is not a well-formed RFC 3339 instant.
///
/// ## Why normalising is not optional here
///
/// Valid-time is compared **as text**. `facts_as_of` filters with
/// `valid_from <= ?1 AND (valid_to IS NULL OR valid_to > ?1)`, and those are
/// SQLite TEXT comparisons — so the store's time ordering is *lexicographic
/// byte order*, and it coincides with chronological order only while every
/// stamp shares one fixed-width UTC shape. Two RFC 3339 spellings of the same
/// instant are not merely untidy, they sort differently:
///
/// | spelling | instant | sorts |
/// |---|---|---|
/// | `2026-09-07T00:30:00+01:00` | 2026-09-06T23:30:00Z | AFTER `2026-09-06T23:45:00Z`, though it is earlier |
/// | `2026-09-07T05:19:41.5Z` | 41.5s past the minute | BEFORE `2026-09-07T05:19:41Z` (`.` is 0x2E, `Z` is 0x5A) |
///
/// This matters for the caller this was written for. Git's `%aI` emits the
/// author's **local offset**, so a historical commit valid-time arrives as
/// `2026-09-07T05:19:41+02:00` for anyone east of UTC — and stored verbatim it
/// would answer time-travel queries wrongly, silently, only for contributors in
/// certain time zones. Rejecting offsets would push that conversion onto every
/// caller; normalising accepts git's own output and keeps one comparable key.
///
/// Sub-second precision is DROPPED, not rejected: every timestamp this crate
/// writes itself comes from [`now_iso`] at whole-second resolution, and a
/// fractional stamp would sort before the whole second it belongs to. Commit
/// times are second-granular, so this is lossless for the motivating caller and
/// stated rather than silent for everyone else.
///
/// Leap seconds (`:60`) are accepted and carry into the following minute, which
/// is what the epoch arithmetic does anyway; RFC 3339 permits the spelling and
/// refusing it would reject a legitimate instant.
#[must_use]
pub fn normalize_rfc3339_utc(s: &str) -> Option<String> {
    let b = s.as_bytes();
    // YYYY-MM-DDTHH:MM:SS is 19 bytes, plus at least a one-byte offset.
    if b.len() < 20 {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let part = s.get(from..to)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mi, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=days_in_month(y, mo)).contains(&d) {
        return None;
    }
    // 60 is RFC 3339's leap second; 24:00 is not permitted by this grammar.
    if hh > 23 || mi > 59 || ss > 60 {
        return None;
    }

    // Optional fractional seconds: at least one digit after the dot.
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }

    let offset_secs = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() => {
            if b[i + 3] != b':' {
                return None;
            }
            let (oh, om) = (num(i + 1, i + 3)?, num(i + 4, i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let magnitude = oh * 3_600 + om * 60;
            if *sign == b'+' { magnitude } else { -magnitude }
        }
        _ => return None,
    };

    // The offset is what the local clock is AHEAD of UTC, so subtract it.
    let secs = days_from_civil(y, mo, d) * 86_400 + hh * 3_600 + mi * 60 + ss - offset_secs;
    Some(format_iso_signed(secs))
}

/// Days in `m` of year `y`, Gregorian.
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Convert a count of days since the Unix epoch to a `(year, month, day)` civil
/// date. Howard Hinnant's `civil_from_days`, valid for the full proleptic
/// Gregorian range (leap years and century rules handled exactly).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
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
    fn normalizes_offsets_to_the_same_key_as_their_utc_spelling() {
        // The whole point: two spellings of ONE instant must produce one
        // storable key, because the store compares valid-time as text.
        let utc = normalize_rfc3339_utc("2026-09-06T23:30:00Z").unwrap();
        assert_eq!(utc, "2026-09-06T23:30:00Z");
        assert_eq!(
            normalize_rfc3339_utc("2026-09-07T00:30:00+01:00"),
            Some(utc)
        );
        assert_eq!(
            normalize_rfc3339_utc("2026-09-06T18:30:00-05:00").as_deref(),
            Some("2026-09-06T23:30:00Z")
        );
    }

    #[test]
    fn offset_can_move_the_date_across_a_boundary() {
        // String surgery on the offset would leave the date alone and be wrong
        // for exactly these two cases.
        assert_eq!(
            normalize_rfc3339_utc("2026-09-07T00:30:00+01:00").as_deref(),
            Some("2026-09-06T23:30:00Z")
        );
        assert_eq!(
            normalize_rfc3339_utc("2026-09-06T23:30:00-01:00").as_deref(),
            Some("2026-09-07T00:30:00Z")
        );
    }

    #[test]
    fn normalized_keys_sort_chronologically_where_raw_ones_do_not() {
        // This is the defect the normalizer exists to prevent, asserted as an
        // ordering rather than as a string: the raw pair sorts BACKWARDS.
        let (earlier_raw, later_raw) = ("2026-09-07T00:30:00+01:00", "2026-09-06T23:45:00Z");
        assert!(
            earlier_raw > later_raw,
            "precondition: raw spellings sort wrongly, which is why we normalize"
        );
        let earlier = normalize_rfc3339_utc(earlier_raw).unwrap();
        let later = normalize_rfc3339_utc(later_raw).unwrap();
        assert!(earlier < later, "{earlier} should sort before {later}");
    }

    #[test]
    fn fractional_seconds_are_truncated_not_rejected() {
        // ".5Z" sorts BEFORE "Z" (0x2E < 0x5A), so a fractional stamp kept
        // verbatim would precede the whole second containing it.
        assert_eq!(
            normalize_rfc3339_utc("2026-09-07T05:19:41.5Z").as_deref(),
            Some("2026-09-07T05:19:41Z")
        );
        assert_eq!(
            normalize_rfc3339_utc("2026-09-07T05:19:41.123456789Z").as_deref(),
            Some("2026-09-07T05:19:41Z")
        );
        // A dot with no digits is malformed, not a zero fraction.
        assert_eq!(normalize_rfc3339_utc("2026-09-07T05:19:41.Z"), None);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "not a timestamp",
            "2026-09-07",                // date only
            "2026-09-07T05:19:41",       // no offset at all
            "2026-13-07T05:19:41Z",      // month 13
            "2026-02-30T05:19:41Z",      // February has no 30th
            "2025-02-29T05:19:41Z",      // 2025 is not a leap year
            "2026-09-07T24:00:00Z",      // 24:00 not permitted by this grammar
            "2026-09-07T05:61:41Z",      // minute 61
            "2026-09-07T05:19:61Z",      // second 61 (60 is the leap second)
            "2026-09-07 05:19:41Z",      // space separator is ISO 8601, not 3339
            "2026-09-07T05:19:41+0100",  // offset needs the colon
            "2026-09-07T05:19:41+24:00", // offset hour out of range
            "2026-09-07T05:19:41Zextra", // trailing junk
            "2026-09-07T05:19:41ZZ",
        ] {
            assert_eq!(normalize_rfc3339_utc(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn accepts_the_shapes_the_motivating_caller_emits() {
        // git --date=iso-strict / %aI, the source of Yupana's commit times.
        assert!(normalize_rfc3339_utc("2026-09-07T05:19:41+02:00").is_some());
        assert!(normalize_rfc3339_utc("2026-09-07T05:19:41-07:00").is_some());
        // Lowercase 't'/'z' are legal RFC 3339.
        assert_eq!(
            normalize_rfc3339_utc("2026-09-07t05:19:41z").as_deref(),
            Some("2026-09-07T05:19:41Z")
        );
        // Leap second carries into the next minute rather than being refused.
        assert_eq!(
            normalize_rfc3339_utc("2016-12-31T23:59:60Z").as_deref(),
            Some("2017-01-01T00:00:00Z")
        );
    }

    #[test]
    fn round_trips_what_the_store_itself_writes() {
        // Normalising now_iso() must be the identity, or the new parameter
        // would write keys shaped unlike every other timestamp in the store.
        let now = now_iso();
        assert_eq!(normalize_rfc3339_utc(&now).as_deref(), Some(now.as_str()));
    }

    #[test]
    fn handles_instants_before_the_epoch() {
        // A historical valid-time may predate 1970 even though nothing was ever
        // WRITTEN then. Truncating division would put this on 1970-01-01.
        assert_eq!(
            normalize_rfc3339_utc("1969-12-31T23:59:59Z").as_deref(),
            Some("1969-12-31T23:59:59Z")
        );
        assert_eq!(
            normalize_rfc3339_utc("1900-02-28T12:00:00Z").as_deref(),
            Some("1900-02-28T12:00:00Z")
        );
    }

    #[test]
    fn days_from_civil_inverts_civil_from_days() {
        // Both directions over a range that crosses leap years and a century
        // rule, so the offset arithmetic above rests on a checked inverse.
        for days in -30_000..30_000_i64 {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "at day {days}");
        }
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

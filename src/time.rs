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

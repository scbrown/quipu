//! Duration distributions shared by aggregate and bounded caller metrics.
use std::fmt::Write as _;

// Retain the old edges while resolving subsecond and 1–10s tails more finely.
// Quantiles still interpolate; bucket deltas measure exact boundary counts.
const BUCKETS: [f64; 12] = [
    0.005, 0.025, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.5, 5.0, 10.0, 30.0,
];

#[derive(Default, Clone)]
pub(super) struct Hist {
    counts: [u64; BUCKETS.len()],
    sum: f64,
    total: u64,
}

impl Hist {
    pub(super) fn observe(&mut self, seconds: f64) {
        if let Some(i) = BUCKETS.iter().position(|bound| seconds <= *bound) {
            self.counts[i] += 1;
        }
        self.sum += seconds;
        self.total += 1;
    }

    /// Labels are escaped by the caller, just like the counter labels.
    pub(super) fn render(&self, out: &mut String, name: &str, labels: &str) {
        let mut cumulative = 0;
        for (i, bound) in BUCKETS.iter().enumerate() {
            cumulative += self.counts[i];
            let _ = writeln!(out, "{name}_bucket{{{labels},le=\"{bound}\"}} {cumulative}");
        }
        let _ = writeln!(out, "{name}_bucket{{{labels},le=\"+Inf\"}} {}", self.total);
        let _ = writeln!(out, "{name}_sum{{{labels}}} {}", self.sum);
        let _ = writeln!(out, "{name}_count{{{labels}}} {}", self.total);
    }
}

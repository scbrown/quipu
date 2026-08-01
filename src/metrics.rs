//! In-process Prometheus metrics — the usage-measurement layer.
//!
//! Everything here is a plain in-memory registry rendered in the Prometheus
//! text exposition format by [`render`]; there is no metrics crate dependency.
//! Counters are updated from the request middleware and the policy handler;
//! graph-size gauges are computed by the caller at scrape time (one cheap SQL
//! COUNT — never the full-scan the /stats endpoint does) and passed in.
//!
//! Label vocabulary notes (deliberate, do not "fix"):
//!   - `quipu_policy_check_total{outcome=...}` uses /policy/check's OWN
//!     three-valued vocabulary: satisfied | unsatisfied | unknown. Any
//!     block/warn tiering is the CALLER's mapping (enforcement tier x
//!     exposure), applied downstream — labelling it here would mint a second,
//!     conflicting tiering vocabulary.
//!   - `endpoint` is the ROUTE TEMPLATE (e.g. `/entity/{iri}`), not the raw
//!     path, so cardinality stays bounded. Unrouted requests are `unmatched`.
//!   - Histogram buckets are sized to measured baselines: /policy/check ~5ms,
//!     /query 0.03-0.6s healthy with a >20s tail while wedged — the tail
//!     buckets exist precisely so a wedge is visible per-endpoint (a
//!     service-level probe stayed green through a real 20s /query wedge).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Current process (resident, virtual) memory in bytes, from `/proc/self/statm`
/// on Linux; `(0, 0)` elsewhere. `statm` fields are in pages; on the `x86_64`
/// Linux quipu runs on the page size is 4096. Cheap in-kernel read — safe to
/// call at scrape time and after each write (memory telemetry: the balloon was
/// invisible because NO memory metric existed).
#[must_use]
pub fn process_memory() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let mut it = statm.split_whitespace();
            let vsz_pages: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let rss_pages: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            const PAGE: u64 = 4096;
            return (rss_pages * PAGE, vsz_pages * PAGE);
        }
    }
    (0, 0)
}

/// Upper bounds (seconds) of the duration histogram buckets; +Inf is implicit.
const BUCKETS: [f64; 8] = [0.005, 0.025, 0.1, 0.5, 1.0, 2.5, 10.0, 30.0];

#[derive(Default, Clone)]
struct Hist {
    counts: [u64; BUCKETS.len()],
    sum: f64,
    total: u64,
}

#[derive(Default)]
pub struct Metrics {
    /// (endpoint template, status) -> request count.
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    /// endpoint template -> duration histogram.
    durations: Mutex<BTreeMap<String, Hist>>,
    /// /policy/check outcome -> count.
    policy: Mutex<BTreeMap<String, u64>>,
    /// Total datums committed to the store — correlates an RSS rise with the
    /// write volume that drove it (memory telemetry).
    facts_written: AtomicU64,
    /// High-water-mark RSS in bytes, sampled after each write and at scrape.
    /// A 15s scrape can miss the peak of a burst export; this catches it.
    peak_rss_bytes: AtomicU64,
}

/// The process-wide registry.
pub fn metrics() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(Metrics::default)
}

impl Metrics {
    /// Record one served request: endpoint template, response status, duration.
    pub fn observe_request(&self, endpoint: &str, status: u16, seconds: f64) {
        *self
            .requests
            .lock()
            .unwrap()
            .entry((endpoint.to_string(), status))
            .or_insert(0) += 1;
        let mut durs = self.durations.lock().unwrap();
        let h = durs.entry(endpoint.to_string()).or_default();
        for (i, ub) in BUCKETS.iter().enumerate() {
            if seconds <= *ub {
                h.counts[i] += 1;
                break;
            }
        }
        h.sum += seconds;
        h.total += 1;
    }

    /// Record `n` datums committed, and sample RSS into the high-water mark so
    /// a burst export's peak is captured even between 15s scrapes (memory telemetry).
    pub fn observe_write(&self, n: u64) {
        self.facts_written.fetch_add(n, Ordering::Relaxed);
        self.sample_rss();
    }

    /// Update the RSS high-water mark from the current process RSS.
    pub fn sample_rss(&self) {
        let (rss, _vsz) = process_memory();
        self.peak_rss_bytes.fetch_max(rss, Ordering::Relaxed);
    }

    /// Record one /policy/check evaluation by its own outcome vocabulary.
    pub fn observe_policy_outcome(&self, outcome: &str) {
        *self
            .policy
            .lock()
            .unwrap()
            .entry(outcome.to_string())
            .or_insert(0) += 1;
    }

    /// Render the Prometheus text exposition. Graph-size gauges are computed by
    /// the caller (cheap SQL count under the store lock) and passed in.
    pub fn render(&self, entities: u64, facts: u64, predicates: u64) -> String {
        let mut out = String::new();

        out.push_str(
            "# HELP quipu_http_requests_total Requests served, by route template and status.\n\
             # TYPE quipu_http_requests_total counter\n",
        );
        for ((ep, status), n) in self.requests.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_http_requests_total{{endpoint=\"{}\",status=\"{status}\"}} {n}",
                esc(ep)
            );
        }

        out.push_str(
            "# HELP quipu_http_request_duration_seconds Request duration, by route template.\n\
             # TYPE quipu_http_request_duration_seconds histogram\n",
        );
        for (ep, h) in self.durations.lock().unwrap().iter() {
            let ep = esc(ep);
            let mut cum = 0u64;
            for (i, ub) in BUCKETS.iter().enumerate() {
                cum += h.counts[i];
                let _ = writeln!(
                    out,
                    "quipu_http_request_duration_seconds_bucket{{endpoint=\"{ep}\",le=\"{ub}\"}} {cum}"
                );
            }
            let _ = writeln!(
                out,
                "quipu_http_request_duration_seconds_bucket{{endpoint=\"{ep}\",le=\"+Inf\"}} {}",
                h.total
            );
            let _ = writeln!(
                out,
                "quipu_http_request_duration_seconds_sum{{endpoint=\"{ep}\"}} {}",
                h.sum
            );
            let _ = writeln!(
                out,
                "quipu_http_request_duration_seconds_count{{endpoint=\"{ep}\"}} {}",
                h.total
            );
        }

        out.push_str(
            "# HELP quipu_policy_check_total Policy evaluations by /policy/check's own outcome vocabulary.\n\
             # TYPE quipu_policy_check_total counter\n",
        );
        for (outcome, n) in self.policy.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_policy_check_total{{outcome=\"{}\"}} {n}",
                esc(outcome)
            );
        }

        out.push_str(
            "# HELP quipu_graph_entities Distinct live subjects in the root graph.\n\
             # TYPE quipu_graph_entities gauge\n",
        );
        let _ = writeln!(out, "quipu_graph_entities {entities}");
        out.push_str(
            "# HELP quipu_graph_facts Live facts in the root graph.\n\
             # TYPE quipu_graph_facts gauge\n",
        );
        let _ = writeln!(out, "quipu_graph_facts {facts}");
        out.push_str(
            "# HELP quipu_graph_predicates Distinct live predicates in the root graph.\n\
             # TYPE quipu_graph_predicates gauge\n",
        );
        let _ = writeln!(out, "quipu_graph_predicates {predicates}");

        // Memory (memory telemetry): the balloon that OOM-killed the service was
        // invisible because no memory metric existed. RSS/VSZ are read fresh at
        // scrape; the peak is the high-water mark since start, which catches a
        // burst-export spike a coarse scrape would miss.
        let (rss, vsz) = process_memory();
        self.peak_rss_bytes.fetch_max(rss, Ordering::Relaxed);
        let peak = self.peak_rss_bytes.load(Ordering::Relaxed);
        out.push_str(
            "# HELP process_resident_memory_bytes Resident set size of the quipu-server process.\n\
             # TYPE process_resident_memory_bytes gauge\n",
        );
        let _ = writeln!(out, "process_resident_memory_bytes {rss}");
        out.push_str(
            "# HELP process_virtual_memory_bytes Virtual memory size of the quipu-server process.\n\
             # TYPE process_virtual_memory_bytes gauge\n",
        );
        let _ = writeln!(out, "process_virtual_memory_bytes {vsz}");
        out.push_str(
            "# HELP quipu_process_peak_rss_bytes High-water-mark RSS since start, sampled after each write.\n\
             # TYPE quipu_process_peak_rss_bytes gauge\n",
        );
        let _ = writeln!(out, "quipu_process_peak_rss_bytes {peak}");
        out.push_str(
            "# HELP quipu_facts_written_total Datums committed to the store (correlates RSS with write volume).\n\
             # TYPE quipu_facts_written_total counter\n",
        );
        let _ = writeln!(
            out,
            "quipu_facts_written_total {}",
            self.facts_written.load(Ordering::Relaxed)
        );

        out
    }
}

/// Escape a Prometheus label value.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_histogram_and_gauges_render_in_exposition_format() {
        let m = Metrics::default();
        m.observe_request("/query", 200, 0.03);
        m.observe_request("/query", 200, 21.0); // the wedge-tail case
        m.observe_request("/knot", 400, 0.004);
        m.observe_policy_outcome("satisfied");
        m.observe_policy_outcome("unknown");
        let text = m.render(10, 200, 7);

        assert!(text.contains("quipu_http_requests_total{endpoint=\"/query\",status=\"200\"} 2"));
        assert!(text.contains("quipu_http_requests_total{endpoint=\"/knot\",status=\"400\"} 1"));
        // 21s lands past every finite bucket: le="30" cumulative picks it up,
        // and only +Inf and le="30" see the second observation.
        assert!(text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"0.1\"} 1"
        ));
        assert!(text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"30\"} 2"
        ));
        assert!(text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"+Inf\"} 2"
        ));
        assert!(text.contains("quipu_http_request_duration_seconds_count{endpoint=\"/query\"} 2"));
        // The policy vocabulary is quipu's own three-valued one.
        assert!(text.contains("quipu_policy_check_total{outcome=\"satisfied\"} 1"));
        assert!(text.contains("quipu_policy_check_total{outcome=\"unknown\"} 1"));
        assert!(text.contains("quipu_graph_facts 200"));
    }

    #[test]
    fn label_values_are_escaped() {
        let m = Metrics::default();
        m.observe_request("/od\"d\\path", 200, 0.01);
        let text = m.render(0, 0, 0);
        assert!(text.contains("endpoint=\"/od\\\"d\\\\path\""));
    }

    #[test]
    fn memory_metrics_render_and_count_writes() {
        let m = Metrics::default();
        m.observe_write(5);
        m.observe_write(3);
        let text = m.render(0, 0, 0);
        assert!(text.contains("quipu_facts_written_total 8"));
        assert!(text.contains("process_resident_memory_bytes"));
        assert!(text.contains("process_virtual_memory_bytes"));
        assert!(text.contains("quipu_process_peak_rss_bytes"));
        // On the Linux target quipu runs on, RSS must be readable and non-zero.
        #[cfg(target_os = "linux")]
        {
            let (rss, vsz) = process_memory();
            assert!(
                rss > 0 && vsz >= rss,
                "RSS/VSZ readable from /proc/self/statm"
            );
        }
    }
}

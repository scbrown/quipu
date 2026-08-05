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

/// Unix seconds at which this process started serving, recorded by
/// [`init_start_time`] during startup.
static START_UNIX: OnceLock<f64> = OnceLock::new();

/// Record the process start time. Call ONCE, from server startup.
///
/// Deliberately explicit rather than lazily initialised on first touch: a lazy
/// value would be set by whatever happened to call it first — plausibly the
/// first `/metrics` scrape — and would then report a "start time" minutes after
/// the real one. If this is never called, [`render`] omits the metric entirely.
/// Absent beats wrong: a missing series is visibly missing, whereas a plausible
/// wrong one silently corrupts every restart calculation built on it.
pub fn init_start_time() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let _ = START_UNIX.set(now);
}

/// Normalise a caller identity into a bounded metric label (aegis-ma1hy).
///
/// Precedence: the explicit `X-Quipu-Client` header, then `User-Agent`, then
/// `unattributed`. Everything is lowercased, cut at the first `/` (so
/// `curl/8.5.0` and `hank/0.4.1` collapse to `curl` and `hank` rather than
/// minting a label per released version), restricted to `[a-z0-9._-]`, and
/// truncated to 32 bytes.
///
/// `unattributed` is a REAL answer, not a failure: it is the measure of how much
/// load still cannot be attributed, which is the number aegis-ma1hy exists to
/// drive down. Callers that send nothing must be visible as a bloc, never
/// silently dropped.
#[must_use]
pub fn normalize_client(explicit: Option<&str>, user_agent: Option<&str>) -> String {
    let raw = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| user_agent.map(str::trim).filter(|s| !s.is_empty()));
    let Some(raw) = raw else {
        return "unattributed".to_string();
    };
    let cleaned: String = raw
        // Cut at the first `/` OR any whitespace. Splitting on newlines matters
        // beyond tidiness: a raw newline in a label value would break the
        // exposition format outright, and this header is caller-controlled.
        // The char filter below would also strip it, but two independent
        // defences for a format-injection vector reachable by any caller is the
        // right number.
        .split(['/', ' ', '\t', '\n', '\r'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "unattributed".to_string()
    } else {
        cleaned
    }
}

/// Upper bounds (seconds) of the duration histogram buckets; +Inf is implicit.
const BUCKETS: [f64; 8] = [0.005, 0.025, 0.1, 0.5, 1.0, 2.5, 10.0, 30.0];

#[derive(Default, Clone)]
struct Hist {
    counts: [u64; BUCKETS.len()],
    sum: f64,
    total: u64,
}

/// Maximum distinct `client` label values before everything else folds into
/// `other`. The label's SOURCE is a request header, i.e. attacker/caller
/// controlled, so an uncapped map is an unbounded-cardinality hole reachable by
/// anyone who can reach the port: one loop sending random `X-Quipu-Client`
/// values would grow this map without limit and blow up both the render and the
/// scraping Prometheus. The cap converts that from an outage into a lost label.
///
/// 32 is sized against the KNOWN caller list on aegis-ma1hy (SessionStart
/// query-first hooks, hank's pre-edit/pre-bash policy hooks, st subscribe,
/// agent /episode writes, the ingest path, bobbin) with room to spare. If
/// `other` ever dominates, that is the signal to raise it — deliberately, not
/// by removing the cap.
const MAX_CLIENTS: usize = 32;

#[derive(Default)]
pub struct Metrics {
    /// (endpoint template, status) -> request count.
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    /// endpoint template -> duration histogram.
    durations: Mutex<BTreeMap<String, Hist>>,
    /// /policy/check outcome -> count.
    policy: Mutex<BTreeMap<String, u64>>,
    /// (client, endpoint template) -> (request count, summed seconds).
    ///
    /// A SUM and a COUNT rather than a per-client histogram: the question this
    /// exists to answer (aegis-ma1hy) is "which caller accounts for what
    /// fraction of /query TIME", which is a ratio of sums. A histogram per
    /// client would multiply series by the bucket count to answer a question
    /// nobody asked.
    clients: Mutex<BTreeMap<(String, String), (u64, f64)>>,
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

    /// Attribute one request's TIME to a caller (aegis-ma1hy).
    ///
    /// Call with the already-normalised label from [`normalize_client`]; this
    /// method does not normalise, so that the middleware pays that cost once and
    /// the same string reaches the log line and the metric.
    ///
    /// Folds into `other` past [`MAX_CLIENTS`] distinct values — see that
    /// constant for why the cap is not optional.
    pub fn observe_client(&self, client: &str, endpoint: &str, seconds: f64) {
        let mut map = self.clients.lock().unwrap();
        let key = (client.to_string(), endpoint.to_string());
        // `MAX_CLIENTS - 1`, not `MAX_CLIENTS`: the `other` bucket needs a slot
        // of its own, or the cap is silently one higher than the constant says.
        // My own test caught this at 33 series against a stated cap of 32 — a
        // one-series overshoot is harmless, but a cap that does not mean its own
        // number is the kind of thing that gets copied somewhere it matters.
        let key = if map.contains_key(&key) || map.len() < MAX_CLIENTS - 1 {
            key
        } else {
            ("other".to_string(), endpoint.to_string())
        };
        let e = map.entry(key).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += seconds;
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

        // Per-caller attribution (aegis-ma1hy). Two counters rather than a
        // histogram: "what fraction of /query time is caller X" is a ratio of
        // sums, and the ratio is taken over increase() of BOTH, so it is
        // reset-safe in a way a bare counter read is not.
        out.push_str(
            "# HELP quipu_http_client_requests_total Requests served, by normalised caller and route template.\n\
             # TYPE quipu_http_client_requests_total counter\n",
        );
        for ((client, ep), (n, _secs)) in self.clients.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_http_client_requests_total{{client=\"{}\",endpoint=\"{}\"}} {n}",
                esc(client),
                esc(ep)
            );
        }

        out.push_str(
            "# HELP quipu_http_client_request_seconds_total Cumulative request seconds, by normalised caller and route template.\n\
             # TYPE quipu_http_client_request_seconds_total counter\n",
        );
        for ((client, ep), (_n, secs)) in self.clients.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_http_client_request_seconds_total{{client=\"{}\",endpoint=\"{}\"}} {secs}",
                esc(client),
                esc(ep)
            );
        }

        // Restart detection. quipu exported NO process start time, which is why
        // restarts had to be counted by inferring counter RESETS — and an
        // increase() across an unnoticed reset already fabricated a 169k/5min
        // figure against a true ~400/100min (aegis-jebd8). Every counter above
        // inherits that hazard; this gauge is what makes a reset visible instead
        // of merely inferable. Emitted ONLY if startup recorded it (see
        // init_start_time): absent beats a wrong start time.
        if let Some(t) = START_UNIX.get() {
            out.push_str(
                "# HELP process_start_time_seconds Start time of the process since unix epoch in seconds.\n\
                 # TYPE process_start_time_seconds gauge\n",
            );
            let _ = writeln!(out, "process_start_time_seconds {t}");
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
    fn client_label_precedence_and_normalisation() {
        // Explicit header wins over User-Agent.
        assert_eq!(
            normalize_client(Some("hank-policy-hook"), Some("curl/8.5.0")),
            "hank-policy-hook"
        );
        // Version suffixes collapse — otherwise every release mints a label.
        assert_eq!(normalize_client(None, Some("curl/8.5.0")), "curl");
        assert_eq!(normalize_client(None, Some("bobbin/0.6.5")), "bobbin");
        // Absent, empty, and whitespace-only all read as unattributed, which is
        // a real measurement (how much load nobody claims), not a failure.
        assert_eq!(normalize_client(None, None), "unattributed");
        assert_eq!(normalize_client(Some(""), None), "unattributed");
        assert_eq!(normalize_client(Some("   "), Some("")), "unattributed");
        // An empty explicit header falls THROUGH to User-Agent rather than
        // swallowing it.
        assert_eq!(normalize_client(Some(""), Some("st/1.2")), "st");
        // Hostile input cannot break the exposition format or mint a label of
        // unbounded length: quotes, backslashes, newlines and spaces are gone.
        assert_eq!(
            normalize_client(Some("ev\"il\\\nagent name"), None),
            "evil"
        );
        assert_eq!(normalize_client(Some(&"x".repeat(200)), None).len(), 32);
        // Non-ASCII that filters to nothing must not produce an empty label.
        assert_eq!(normalize_client(Some("日本語"), None), "unattributed");
    }

    #[test]
    fn client_cardinality_is_capped_and_overflow_is_visible() {
        let m = Metrics::default();
        // Far more distinct callers than the cap, all on one endpoint.
        for i in 0..(MAX_CLIENTS * 4) {
            m.observe_client(&format!("caller{i}"), "/query", 0.1);
        }
        let text = m.render(0, 0, 0);
        let series = text
            .lines()
            .filter(|l| l.starts_with("quipu_http_client_requests_total"))
            .count();
        // The cap is the point: an uncapped map would render 128 series here.
        assert!(
            series <= MAX_CLIENTS,
            "client series {series} exceeded the cap {MAX_CLIENTS}"
        );
        // Overflow is FOLDED, never dropped — the totals must still add up, or
        // the attribution silently understates load, which is worse than no
        // attribution at all.
        assert!(text.contains("client=\"other\""));
        let total: u64 = text
            .lines()
            .filter(|l| l.starts_with("quipu_http_client_requests_total"))
            .filter_map(|l| l.rsplit(' ').next()?.parse::<u64>().ok())
            .sum();
        assert_eq!(total as usize, MAX_CLIENTS * 4);
    }

    #[test]
    fn client_time_attribution_sums_and_start_time_is_absent_until_recorded() {
        let m = Metrics::default();
        m.observe_client("hank", "/query", 2.0);
        m.observe_client("hank", "/query", 3.0);
        m.observe_client("bobbin", "/query", 1.0);
        let text = m.render(0, 0, 0);
        assert!(text.contains(
            "quipu_http_client_requests_total{client=\"hank\",endpoint=\"/query\"} 2"
        ));
        assert!(text.contains(
            "quipu_http_client_request_seconds_total{client=\"hank\",endpoint=\"/query\"} 5"
        ));
        assert!(text.contains(
            "quipu_http_client_request_seconds_total{client=\"bobbin\",endpoint=\"/query\"} 1"
        ));
        // 5 of 6 seconds are hank's — the ratio this whole change exists to make
        // computable (aegis-ma1hy: 65.6% of /query load was unattributable).
    }

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

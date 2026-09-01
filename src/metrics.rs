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

use std::collections::{BTreeMap, BTreeSet};
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
    // Whole seconds via the wasm-safe clock shim (quipu-gsg); sub-second
    // start-time precision buys nothing for restart detection.
    let _ = START_UNIX.set(crate::time::epoch_secs() as f64);
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

/// Normalise the optional work-item identity supplied in `X-Quipu-Task`.
///
/// Unlike [`normalize_client`], this has no fallback: a User-Agent identifies
/// software, never the bead whose work caused the request. Missing or invalid
/// values are therefore explicitly `unattributed` rather than silently joined
/// to an arbitrary task. The independent cardinality cap in [`request_key`]
/// folds excess task identities into `other`.
#[must_use]
pub fn normalize_task(explicit: Option<&str>) -> String {
    let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) else {
        return "unattributed".to_string();
    };
    let cleaned: String = raw
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
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
/// 32 is sized against the KNOWN caller list on `aegis-ma1hy` (`SessionStart`
/// query-first hooks, hank's pre-edit/pre-bash policy hooks, st subscribe,
/// agent `/episode` writes, the ingest path, bobbin) with room to spare. If
/// `other` ever dominates, that is the signal to raise it — deliberately, not
/// by removing the cap.
const MAX_CLIENTS: usize = 32;

/// Maximum distinct task identities retained per process. Task ids are more
/// numerous than client kinds, but remain caller-controlled metric labels, so
/// they need a separate finite budget rather than consuming the client cap.
const MAX_TASKS: usize = 128;

type ClientTaskEndpoint = (String, String, String);
type RequestObservation = (u64, f64);

fn client_key<V>(
    map: &BTreeMap<(String, String), V>,
    client: &str,
    endpoint: &str,
) -> (String, String) {
    let key = (client.to_string(), endpoint.to_string());
    let known_clients: BTreeSet<&str> = map
        .keys()
        .map(|(known_client, _)| known_client.as_str())
        .filter(|known_client| *known_client != "other")
        .collect();
    if map.contains_key(&key)
        || known_clients.contains(client)
        || known_clients.len() < MAX_CLIENTS - 1
    {
        key
    } else {
        ("other".to_string(), endpoint.to_string())
    }
}

fn request_key<V>(
    map: &BTreeMap<ClientTaskEndpoint, V>,
    client: &str,
    task: &str,
    endpoint: &str,
) -> ClientTaskEndpoint {
    let known_clients: BTreeSet<&str> = map
        .keys()
        .map(|(known_client, _, _)| known_client.as_str())
        .filter(|known_client| *known_client != "other")
        .collect();
    let known_tasks: BTreeSet<&str> = map
        .keys()
        .map(|(_, known_task, _)| known_task.as_str())
        .filter(|known_task| *known_task != "other")
        .collect();
    let bounded_client = if known_clients.contains(client) || known_clients.len() < MAX_CLIENTS - 1
    {
        client
    } else {
        "other"
    };
    let bounded_task = if known_tasks.contains(task) || known_tasks.len() < MAX_TASKS - 1 {
        task
    } else {
        "other"
    };
    (
        bounded_client.to_string(),
        bounded_task.to_string(),
        endpoint.to_string(),
    )
}

#[derive(Default)]
pub struct Metrics {
    /// (endpoint template, status) -> request count.
    requests: Mutex<BTreeMap<(String, u16), u64>>,
    /// endpoint template -> duration histogram.
    durations: Mutex<BTreeMap<String, Hist>>,
    /// /policy/check outcome -> count.
    policy: Mutex<BTreeMap<String, u64>>,
    /// (client, task, endpoint template) -> (request count, summed seconds).
    ///
    /// A SUM and a COUNT rather than a per-client histogram: the question this
    /// exists to answer (aegis-ma1hy) is "which caller accounts for what
    /// fraction of /query TIME", which is a ratio of sums. A histogram per
    /// client would multiply series by the bucket count to answer a question
    /// nobody asked.
    clients: Mutex<BTreeMap<ClientTaskEndpoint, RequestObservation>>,
    /// (client, endpoint) -> (seconds WAITING for the store lock, seconds HOLDING it).
    ///
    /// The distinction this exists for (`aegis-vxl81`): the wall-clock figure in
    /// `clients` above is, on a serialised store, mostly QUEUE WAIT — which is
    /// time caused by OTHER callers. Reporting it as a caller's share attributes
    /// SUFFERING as if it were CAUSATION, and that misreading nearly got the only
    /// working store monitor cut for consuming "26.7% of /query time" when its
    /// real capacity share was under 1%.
    ///
    /// HELD time is the one that adds up to capacity: the store is serialised, so
    /// the held seconds of all callers sum to the wall-clock the store was busy.
    /// WAIT is kept alongside because their RATIO is the saturation signal — wait
    /// >> held means the store is queueing, not working hard.
    store_time: Mutex<BTreeMap<(String, String), (f64, f64)>>,
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
    pub fn observe_client(&self, client: &str, task: &str, endpoint: &str, seconds: f64) {
        let mut map = self.clients.lock().unwrap();
        // `MAX_CLIENTS - 1`, not `MAX_CLIENTS`: the `other` bucket needs a slot
        // of its own, or the cap is silently one higher than the constant says.
        // The reserved value makes the maximum 31 named callers plus `other`.
        // Endpoint series do not consume this identity budget: route templates
        // are bounded independently, and counting pairs caused real callers to
        // fold after a few multi-endpoint clients (aegis-vxl81).
        let key = request_key(&map, client, task, endpoint);
        let e = map.entry(key).or_insert((0, 0.0));
        e.0 += 1;
        e.1 += seconds;
    }

    /// Attribute store-lock WAIT and HELD seconds to a caller (`aegis-vxl81`).
    ///
    /// `held` is capacity consumed; `wait` is capacity consumed by everyone else.
    /// Same [`MAX_CLIENTS`] fold as [`observe_client`], for the same reason — the
    /// label comes from a caller-controlled header.
    pub fn observe_store_time(&self, client: &str, endpoint: &str, wait: f64, held: f64) {
        let mut map = self.store_time.lock().unwrap();
        let key = client_key(&map, client, endpoint);
        let e = map.entry(key).or_insert((0.0, 0.0));
        e.0 += wait;
        e.1 += held;
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
            "# HELP quipu_http_client_requests_total Requests served, by normalised caller, task, and route template.\n\
             # TYPE quipu_http_client_requests_total counter\n",
        );
        for ((client, task, ep), (n, _secs)) in self.clients.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_http_client_requests_total{{client=\"{}\",task=\"{}\",endpoint=\"{}\"}} {n}",
                esc(client),
                esc(task),
                esc(ep)
            );
        }

        out.push_str(
            "# HELP quipu_http_client_request_seconds_total Cumulative request seconds, by normalised caller, task, and route template.\n\
             # TYPE quipu_http_client_request_seconds_total counter\n",
        );
        for ((client, task, ep), (_n, secs)) in self.clients.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_http_client_request_seconds_total{{client=\"{}\",task=\"{}\",endpoint=\"{}\"}} {secs}",
                esc(client),
                esc(task),
                esc(ep)
            );
        }

        // Store-lock time, split into the two quantities the wall-clock counter
        // above cannot separate (aegis-vxl81). Use `held` to answer "who is
        // BURNING the store" — on a serialised store held seconds sum to the time
        // the store was busy, so a caller's share of held IS its share of
        // capacity. Use `wait` only as a saturation signal: wait >> held means
        // queueing. Answering the capacity question with the wall-clock counter
        // reports the biggest SUFFERER as the biggest cause.
        out.push_str(
            "# HELP quipu_store_wait_seconds_total Seconds spent WAITING for the store lock, by caller. Time caused by OTHER callers.\n\
             # TYPE quipu_store_wait_seconds_total counter\n",
        );
        for ((client, ep), (wait, _held)) in self.store_time.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_store_wait_seconds_total{{client=\"{}\",endpoint=\"{}\"}} {wait}",
                esc(client),
                esc(ep)
            );
        }

        out.push_str(
            "# HELP quipu_store_held_seconds_total Seconds spent HOLDING the store lock, by caller. This is capacity actually consumed.\n\
             # TYPE quipu_store_held_seconds_total counter\n",
        );
        for ((client, ep), (_wait, held)) in self.store_time.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_store_held_seconds_total{{client=\"{}\",endpoint=\"{}\"}} {held}",
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
mod tests;

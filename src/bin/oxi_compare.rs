//! Quipu vs Oxigraph: a storage-and-evaluation comparison in ONE process (aegis-j0yaxj.2).
//!
//! ⚠️ READ THE HEADLINE RULE FIRST. **Quipu and Oxigraph share the parser and the
//! data model.** `Cargo.toml` pins `oxrdf`, `oxttl`, `oxrdfio`, `sparesults` and
//! `spargebra` as direct dependencies, and the lockfile resolves exactly ONE
//! version of each -- so both arms here parse with the same `spargebra` and build
//! the same `oxrdf` terms. What differs is the **storage and evaluation layer**,
//! and nothing this harness emits may be presented as engine-versus-engine.
//! That framing is not editorial: it is a required field in the ledger
//! (`comparison_scope`), so a bundle physically cannot be published without it.
//!
//! ## Three properties this harness is built to make unrepresentable
//!
//! 1. **It cannot emit a single-run figure.** Timings are only ever reported as a
//!    [`Spread`], which carries `n`, `min`, `p50`, `p95`, `max` and no scalar
//!    "value" field a reader could quote alone. Constructing one from fewer than
//!    [`MIN_REPEATS`] samples is an error, not a warning. A benchmark that can
//!    print one number will eventually have one number quoted.
//!
//! 2. **The arms are interleaved, not run in sequence.** Running all of Quipu and
//!    then all of Oxigraph on a thermally throttled host (aegis-afu1ge measured
//!    92 °C and 400-700 MHz on this fleet) attributes the drift to whichever arm
//!    ran second. Here every repeat runs both arms back to back, and the ORDER
//!    ALTERNATES by repeat parity so first-mover advantage cancels too.
//!
//! 3. **It refuses to measure an unverified load.** malcolm's ruling on
//!    aegis-j0yaxj.2: a partial ingest is byte-for-byte what a smaller successful
//!    one looks like, and it fails in the FLATTERING direction -- 700M triples has
//!    better latency than 1B, so a silently short load produces a good-looking
//!    result nobody investigates. The declared-count/-digest markers written by
//!    `quipu ingest` are checked in the measurement path, where they fire without
//!    anyone deciding to run them.
//!
//! ## What is deliberately NOT excluded, and why
//!
//! Parse time is shared ground, so the honest treatment is to keep both arms
//! SYMMETRIC rather than to subtract an estimate. Both are handed the query
//! STRING and both parse it with the same `spargebra`, so the parse cost is an
//! identical additive constant. The harness measures it as its own series
//! (`parse_only`) so a reader can see how large that constant is and subtract it
//! if they want evaluation-only figures. Pre-parsing one arm and not the other
//! would be worse than including it in both: Quipu's post-parse entry point
//! (`sparql::eval_parsed`) is private, so only Oxigraph could be given an AST,
//! and the asymmetry would land entirely in Quipu's column.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Fewer samples than this cannot describe a distribution, so [`Spread::new`]
/// refuses them. This is the mechanism behind property 1 in the module docs.
pub const MIN_REPEATS: usize = 5;

/// A timing distribution. **There is deliberately no scalar accessor.**
#[derive(Debug, Clone)]
pub struct Spread {
    /// How many samples the distribution was built from.
    pub n: usize,
    /// Fastest observed run, microseconds.
    pub min_us: u128,
    /// Median run, microseconds.
    pub p50_us: u128,
    /// 95th percentile run, microseconds.
    pub p95_us: u128,
    /// Slowest observed run, microseconds.
    pub max_us: u128,
}

impl Spread {
    /// Build a spread, or refuse. Refusing is the point: a caller that has only
    /// one sample must not be able to report a timing at all.
    pub fn new(mut samples: Vec<Duration>) -> Result<Self, String> {
        if samples.len() < MIN_REPEATS {
            return Err(format!(
                "refusing to report a timing from {} sample(s): a spread needs at least {}. \
                 A single-run figure is exactly what this harness exists to make impossible.",
                samples.len(),
                MIN_REPEATS
            ));
        }
        samples.sort_unstable();
        let us = |d: &Duration| d.as_micros();
        // Integer arithmetic: a float cast here trips cast_sign_loss and, worse,
        // makes the index a function of rounding mode.
        let idx = |num: usize, den: usize| {
            let i = ((samples.len() - 1) * num).div_ceil(den);
            us(&samples[i])
        };
        Ok(Self {
            n: samples.len(),
            min_us: us(&samples[0]),
            p50_us: idx(1, 2),
            p95_us: idx(19, 20),
            max_us: us(&samples[samples.len() - 1]),
        })
    }

    fn to_json(&self) -> String {
        format!(
            r#"{{"n":{},"min_us":{},"p50_us":{},"p95_us":{},"max_us":{}}}"#,
            self.n, self.min_us, self.p50_us, self.p95_us, self.max_us
        )
    }
}

/// Which arm a sample came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arm {
    /// The Quipu storage and evaluation layer.
    Quipu,
    /// The Oxigraph storage and evaluation layer.
    Oxigraph,
}

impl Arm {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quipu => "quipu",
            Self::Oxigraph => "oxigraph",
        }
    }
}

/// One query in the workload.
pub struct Workload {
    /// Stable identifier for this query in the ledger.
    pub name: String,
    /// The query text, handed verbatim to BOTH arms so the parse is symmetric.
    pub sparql: String,
}

/// Result of one query across both arms, plus the shared parse cost.
#[derive(Debug)]
pub struct QueryOutcome {
    /// Matches the [`Workload`] name.
    pub name: String,
    /// Timing distribution per arm. Never a scalar.
    pub per_arm: BTreeMap<&'static str, Spread>,
    /// The shared parse cost, measured with the same `spargebra` both arms use.
    pub parse_only: Spread,
    /// Row counts seen per arm. A comparison of engines that DISAGREE on the
    /// answer is not a performance result, so this is reported and checked.
    pub rows: BTreeMap<&'static str, usize>,
}

/// Refuse to measure a Quipu graph whose declared ingest is absent or unmet.
///
/// This is malcolm's ruling made executable. It runs in the measurement path
/// rather than being offered as a separate check, because the entire failure
/// mode is that nobody thinks to ask.
pub fn assert_declared_complete(store: &quipu::Store, graph_iri: &str) -> Result<u64, String> {
    let ns = quipu::rdf::INGEST_NS;
    // GRAPH-SCOPED, and that is not a detail. `ingest_rdf_declared` writes the
    // markers through `transact_to_graph` into the INGEST graph, while a bare
    // SPARQL query here evaluates against the default scope (ROOT). An unscoped
    // query therefore finds nothing for a perfectly good load -- measured: this
    // gate refused a graph it had just filled itself. A gate that refuses
    // everything is as useless as one that refuses nothing, and it fails in the
    // direction that gets the gate deleted.
    let q = format!(
        "SELECT ?c ?h ?done WHERE {{ GRAPH <{graph_iri}> {{ \
           <{graph_iri}> <{ns}declaredTriples> ?c . \
           <{graph_iri}> <{ns}sourceSha256> ?h . \
           <{graph_iri}> <{ns}complete> ?done }} }}"
    );
    let res = quipu::sparql_query(store, &q)
        .map_err(|e| format!("completion-marker query failed: {e}"))?;
    let rows = res.rows();
    if rows.is_empty() {
        return Err(format!(
            "REFUSING TO MEASURE <{graph_iri}>: no ingest declaration found.\n  \
             A graph with no declaration is indistinguishable from a partial load, and a short \
             load fails in the FLATTERING direction (fewer triples => better latency).\n  \
             Load it with `quipu ingest --declare-count --declare-sha256` (aegis-j0yaxj.2)."
        ));
    }
    let declared = rows[0]
        .get("c")
        .map(|v| format!("{v:?}"))
        .unwrap_or_default();
    let n: u64 = declared
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .map_err(|_| format!("declared count is not a number: {declared}"))?;
    Ok(n)
}

/// Count live facts in a Quipu store. Throughput is a before/after delta of
/// THIS, never `IngestReport.parsed` -- quipu #127 established that the parse
/// count is not a write count (it reported 4 for a re-apply that stored zero),
/// and reporting the cheap half as the whole is wrong in the flattering
/// direction on a public trust page.
pub fn quipu_live_facts(store: &quipu::Store, graph_iri: &str) -> Result<usize, String> {
    // Also GRAPH-scoped, for the same reason as the gate above: an ingest lands
    // in a named graph, so a ROOT-scoped count returns 0 before AND after and the
    // delta is a confident zero. That is worse than an error -- it would have
    // published "0 facts written" for a load that worked.
    let q = format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{graph_iri}> {{ ?s ?p ?o }} }}");
    let res = quipu::sparql_query(store, &q).map_err(|e| format!("live-fact count failed: {e}"))?;
    Ok(res.rows().len())
}

/// Count the ingest-bookkeeping triples Quipu wrote into the measured graph.
///
/// These are provenance about the load, not dataset data, and Oxigraph's copy
/// has no equivalent -- so a workload that matches them makes the arms disagree
/// for a reason that has nothing to do with storage or evaluation.
pub fn quipu_marker_triples(store: &quipu::Store, graph_iri: &str) -> Result<usize, String> {
    let ns = quipu::rdf::INGEST_NS;
    let q = format!(
        "SELECT ?p ?o WHERE {{ GRAPH <{graph_iri}> {{ <{graph_iri}> ?p ?o . \
         FILTER(STRSTARTS(STR(?p), \"{ns}\")) }} }}"
    );
    let res = quipu::sparql_query(store, &q).map_err(|e| format!("marker count failed: {e}"))?;
    Ok(res.rows().len())
}

/// Run the workload with the arms INTERLEAVED and the order alternating.
///
/// `repeats` must be at least [`MIN_REPEATS`]; `warmup` runs are executed and
/// discarded (and reported as discarded, so the ledger says what was thrown
/// away rather than leaving a reader to assume nothing was).
pub fn measure_interleaved(
    quipu_store: &quipu::Store,
    oxi_store: &oxigraph::store::Store,
    workload: &[Workload],
    repeats: usize,
    warmup: usize,
) -> Result<Vec<QueryOutcome>, String> {
    if repeats < MIN_REPEATS {
        return Err(format!(
            "--repeats {repeats} is below the minimum of {MIN_REPEATS}: this harness cannot \
             report a timing it could not describe as a distribution."
        ));
    }
    let mut out = Vec::new();

    for w in workload {
        let mut samples: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
        let mut parse_samples: Vec<Duration> = Vec::new();
        let mut rows: BTreeMap<&'static str, usize> = BTreeMap::new();

        for _ in 0..warmup {
            let _ = run_quipu(quipu_store, &w.sparql);
            let _ = run_oxigraph(oxi_store, &w.sparql);
        }

        for r in 0..repeats {
            // ALTERNATE the order by repeat parity. On a drifting host the arm
            // that runs second inherits the drift; alternating cancels it
            // instead of attributing it.
            let order: [Arm; 2] = if r % 2 == 0 {
                [Arm::Quipu, Arm::Oxigraph]
            } else {
                [Arm::Oxigraph, Arm::Quipu]
            };
            for arm in order {
                let (elapsed, n) = match arm {
                    Arm::Quipu => run_quipu(quipu_store, &w.sparql)?,
                    Arm::Oxigraph => run_oxigraph(oxi_store, &w.sparql)?,
                };
                samples.entry(arm.as_str()).or_default().push(elapsed);
                rows.insert(arm.as_str(), n);
            }
            // The shared component, measured with the same parser both arms use.
            let t = Instant::now();
            let _ = spargebra::SparqlParser::new().parse_query(&w.sparql);
            parse_samples.push(t.elapsed());
        }

        let mut per_arm = BTreeMap::new();
        for (k, v) in samples {
            per_arm.insert(k, Spread::new(v)?);
        }
        out.push(QueryOutcome {
            name: w.name.clone(),
            per_arm,
            parse_only: Spread::new(parse_samples)?,
            rows,
        });
    }
    Ok(out)
}

fn run_quipu(store: &quipu::Store, sparql: &str) -> Result<(Duration, usize), String> {
    let t = Instant::now();
    let res = quipu::sparql_query(store, sparql).map_err(|e| format!("quipu query failed: {e}"))?;
    let e = t.elapsed();
    Ok((e, res.rows().len()))
}

fn run_oxigraph(store: &oxigraph::store::Store, sparql: &str) -> Result<(Duration, usize), String> {
    use oxigraph::sparql::{QueryResults, SparqlEvaluator};
    // parse AND execute inside the timed region, because quipu::sparql_query
    // parses inside too. `SparqlEvaluator` can separate them; deliberately not
    // used that way here -- giving only this arm a pre-parsed AST would put the
    // whole asymmetry in Quipu's column (see the module docs).
    let t = Instant::now();
    let res = SparqlEvaluator::new()
        .parse_query(sparql)
        .map_err(|e| format!("oxigraph parse failed: {e}"))?
        .on_store(store)
        .execute()
        .map_err(|e| format!("oxigraph query failed: {e}"))?;
    let n = match res {
        QueryResults::Solutions(sols) => sols.count(),
        QueryResults::Boolean(_) => 1,
        QueryResults::Graph(g) => g.count(),
    };
    Ok((t.elapsed(), n))
}

/// Emit the ledger. `comparison_scope` is REQUIRED and hardcoded: the caveat
/// cannot be omitted by a caller who forgot it.
pub fn ledger_json(outcomes: &[QueryOutcome], meta: &BTreeMap<String, String>) -> String {
    let mut s = String::from("{\n");
    s.push_str(
        "  \"comparison_scope\": \"STORAGE AND EVALUATION LAYERS ONLY. Quipu and Oxigraph share \
         the SPARQL parser (spargebra) and RDF data model (oxrdf) as direct dependencies resolved \
         to a single version each, so this is not an engine-versus-engine comparison and no \
         headline may imply an independent engine.\",\n",
    );
    s.push_str(&format!(
        "  \"min_repeats_enforced\": {MIN_REPEATS},\n  \"interleaved\": true,\n  \
         \"order_alternates_by_repeat_parity\": true,\n"
    ));
    // A benchmark built without optimisation is not a benchmark. This is recorded
    // rather than inferred from a flag someone remembered to pass, and it is
    // stated as a refusal-shaped string so a debug ledger cannot be quoted
    // without the caveat travelling with it.
    s.push_str(&format!(
        "  \"build_profile\": \"{}\",\n",
        if cfg!(debug_assertions) {
            "DEBUG -- UNOPTIMISED. These timings measure a debug build and are NOT a              performance result under any reading."
        } else {
            "release"
        }
    ));
    for (k, v) in meta {
        s.push_str(&format!("  \"{k}\": \"{v}\",\n"));
    }
    s.push_str("  \"queries\": [\n");
    for (i, o) in outcomes.iter().enumerate() {
        s.push_str(&format!("    {{\"name\": \"{}\", \"arms\": {{", o.name));
        let arms: Vec<String> = o
            .per_arm
            .iter()
            .map(|(k, v)| format!("\"{k}\": {}", v.to_json()))
            .collect();
        s.push_str(&arms.join(", "));
        s.push_str(&format!("}}, \"parse_only\": {}", o.parse_only.to_json()));
        let rows: Vec<String> = o
            .rows
            .iter()
            .map(|(k, v)| format!("\"{k}\": {v}"))
            .collect();
        s.push_str(&format!(", \"rows\": {{{}}}", rows.join(", ")));
        let agree = o
            .rows
            .values()
            .collect::<std::collections::HashSet<_>>()
            .len()
            <= 1;
        s.push_str(&format!(", \"arms_agree_on_row_count\": {agree}}}"));
        if i + 1 < outcomes.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ]\n}\n");
    s
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

fn usage() -> ! {
    eprintln!(
        "quipu-oxi-compare --data <file.nt> --graph <iri> --timestamp <ISO-8601>\n\
        \x20                 --declare-count <n> --declare-sha256 <hex>\n\
        \x20                 --query-dir <dir of *.rq> --repeats <n> [--warmup <n>]\n\
        \x20                 [--quipu-db <path>] [--oxi-dir <path>]\n\n\
         Every declaration flag is MANDATORY: a default would be a second source of truth for a \
         number whose whole job is to come from outside this process.\n\n\
         ⚠ THE MEASURED COMPARISON IS HELD (aegis-j0yaxj.2) until aegis-afu1ge closes -- this \
         fleet measured 92 C and 400-700 MHz against a 4.5 GHz max, and a comparison taken \
         through that cannot support acceptance in either direction. Runs before then are SMOKE \
         TESTS of the harness and the ledger labels them so."
    );
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(data), Some(graph), Some(ts), Some(dc), Some(sha), Some(qdir)) = (
        flag(&args, "--data"),
        flag(&args, "--graph"),
        flag(&args, "--timestamp"),
        flag(&args, "--declare-count"),
        flag(&args, "--declare-sha256"),
        flag(&args, "--query-dir"),
    ) else {
        usage()
    };
    let repeats: usize = flag(&args, "--repeats")
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIN_REPEATS);
    let warmup: usize = flag(&args, "--warmup")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let declared_count: usize = match dc.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("--declare-count must be a number, got {dc:?}");
            std::process::exit(2);
        }
    };

    if let Err(e) = run(
        data,
        graph,
        ts,
        declared_count,
        sha,
        qdir,
        repeats,
        warmup,
        flag(&args, "--quipu-db"),
        flag(&args, "--oxi-dir"),
    ) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    data: &str,
    graph_iri: &str,
    timestamp: &str,
    declared_count: usize,
    declared_sha: &str,
    query_dir: &str,
    repeats: usize,
    warmup: usize,
    quipu_db: Option<&str>,
    oxi_dir: Option<&str>,
) -> Result<(), String> {
    // ---- Quipu arm: load through the DECLARED path, so a short load is refused
    // at ingest as well as at measurement (two independent gates, deliberately).
    let mut qstore = match quipu_db {
        Some(p) => quipu::Store::open(p).map_err(|e| format!("quipu open {p}: {e}"))?,
        None => quipu::Store::open_in_memory().map_err(|e| format!("quipu in-memory: {e}"))?,
    };
    let g = qstore
        .graph_create(graph_iri)
        .map_err(|e| format!("graph_create {graph_iri}: {e}"))?;
    let before = quipu_live_facts(&qstore, graph_iri)?;
    let f = std::fs::File::open(data).map_err(|e| format!("open {data}: {e}"))?;
    let t_q = Instant::now();
    quipu::ingest_rdf_declared(
        &mut qstore,
        std::io::BufReader::new(f),
        oxrdfio::RdfFormat::NTriples,
        None,
        timestamp,
        Some("oxi-compare"),
        Some(data),
        g,
        50_000,
        &quipu::LoadDeclaration {
            triples: declared_count,
            sha256: declared_sha.to_string(),
        },
    )
    .map_err(|e| format!("quipu declared ingest refused or failed: {e}"))?;
    let quipu_load_s = t_q.elapsed().as_secs_f64();
    let after = quipu_live_facts(&qstore, graph_iri)?;
    // THE throughput number: a before/after delta of live facts. Never
    // IngestReport.parsed -- quipu #127 showed it reports 4 for a re-apply that
    // stored nothing, so a rate from it publishes the cheap half as the whole.
    let quipu_written = after.saturating_sub(before);

    // The gate. Runs even though we just loaded it: the harness must refuse a
    // store handed to it by anyone, not only one it filled itself.
    let declared = assert_declared_complete(&qstore, graph_iri)?;

    // Quipu's declared ingest writes its completion markers INTO the measured
    // graph (they must ride the final chunk's transaction, which is what makes a
    // partial load detectable). Oxigraph's copy has no such bookkeeping, so a
    // bare `?s ?p ?o` scan legitimately differs by exactly the marker count --
    // measured 203 vs 200 on the first scoped run. COUNTED from the store, never
    // assumed to be 3: if the marker set ever changes, an assumed constant would
    // turn a real divergence into a passing check.
    let markers = quipu_marker_triples(&qstore, graph_iri)?;

    // ---- Oxigraph arm, on disk when a path is given. See the `oxicompare`
    // feature comment: in-memory-vs-on-disk would flatter this arm.
    let ostore = match oxi_dir {
        Some(p) => {
            oxigraph::store::Store::open(p).map_err(|e| format!("oxigraph open {p}: {e}"))?
        }
        None => oxigraph::store::Store::new().map_err(|e| format!("oxigraph memory: {e}"))?,
    };
    let f2 = std::fs::File::open(data).map_err(|e| format!("open {data}: {e}"))?;
    // INTO THE SAME NAMED GRAPH. Quipu's declared ingest REFUSES ROOT by design,
    // so its data is always in a named graph; Oxigraph's loader would otherwise
    // put it in the DEFAULT graph. The arms would then be querying different
    // places, and an unscoped workload query returns everything from one arm and
    // NOTHING from the other -- measured on the first end-to-end run, where Quipu
    // answered 0 rows to every query and looked 25-60x "faster" as a result.
    let gname = oxigraph::model::NamedNode::new(graph_iri)
        .map_err(|e| format!("graph IRI {graph_iri} is not a valid IRI: {e}"))?;
    let parser =
        oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::NTriples).with_default_graph(gname);
    let t_o = Instant::now();
    ostore
        .load_from_reader(parser, std::io::BufReader::new(f2))
        .map_err(|e| format!("oxigraph load: {e}"))?;
    let oxi_load_s = t_o.elapsed().as_secs_f64();
    let oxi_len = ostore.len().map_err(|e| format!("oxigraph len: {e}"))?;

    // ---- Workload
    let mut workload = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(query_dir)
        .map_err(|e| format!("read_dir {query_dir}: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rq"))
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(format!("no *.rq files in {query_dir}: nothing to measure"));
    }
    for path in entries {
        let sparql = std::fs::read_to_string(&path).map_err(|e| format!("read {path:?}: {e}"))?;
        workload.push(Workload {
            name: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sparql,
        });
    }

    let outcomes = measure_interleaved(&qstore, &ostore, &workload, repeats, warmup)?;

    // A TIMING COMPARISON BETWEEN TWO DIFFERENT COMPUTATIONS IS NOT A RESULT.
    // Recording the disagreement in the ledger is not enough -- the whole failure
    // mode is that nobody reads the field. The first end-to-end run of this
    // harness had Quipu answering 0 rows to every query while looking 25-60x
    // faster, which is a publishable-looking number produced by an arm doing no
    // work. So this REFUSES, the same way the declaration gate does.
    let disagreed: Vec<&str> = outcomes
        .iter()
        .filter(|o| {
            o.rows
                .values()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1
        })
        .map(|o| o.name.as_str())
        .collect();
    if !disagreed.is_empty() {
        return Err(format!(
            "REFUSING TO REPORT: the arms returned DIFFERENT row counts for: {}.\n               They are not answering the same question, so their timings are not comparable \
             and the faster arm is probably the one doing less work.\n               Two causes, both real and both seen on this harness:\n                 (a) the query is UNSCOPED -- it reads Quipu's ROOT (empty) while reading \
             Oxigraph's loaded graph. Scope it to <{graph_iri}> in BOTH arms.\n                 (b) the query matches Quipu's {markers} ingest-bookkeeping triple(s) about \
             <{graph_iri}> itself, which Oxigraph's copy does not have. Exclude them, e.g. \
             FILTER(?s != <{graph_iri}>).",
            disagreed.join(", ")
        ));
    }

    let mut meta = BTreeMap::new();
    meta.insert("dataset".into(), data.to_string());
    meta.insert("dataset_sha256_declared".into(), declared_sha.to_string());
    meta.insert("declared_triples".into(), declared.to_string());
    meta.insert("quipu_ingest_marker_triples".into(), markers.to_string());
    meta.insert("quipu_live_facts_written".into(), quipu_written.to_string());
    meta.insert("oxigraph_quads".into(), oxi_len.to_string());
    meta.insert("quipu_load_seconds".into(), format!("{quipu_load_s:.3}"));
    meta.insert("oxigraph_load_seconds".into(), format!("{oxi_load_s:.3}"));
    meta.insert("ingest_timestamp".into(), timestamp.to_string());
    meta.insert("warmup_runs_discarded".into(), warmup.to_string());
    meta.insert(
        "quipu_backing".into(),
        quipu_db.map_or("memory".into(), str::to_string),
    );
    meta.insert(
        "oxigraph_backing".into(),
        oxi_dir.map_or("memory".into(), str::to_string),
    );
    meta.insert(
        "throughput_source".into(),
        "before/after delta of live facts, never IngestReport.parsed (quipu #127)".into(),
    );
    meta.insert("status".into(), std::env::var("OXI_COMPARE_STATUS")
        .unwrap_or_else(|_| "SMOKE TEST -- not a data point; the measured comparison is held until aegis-afu1ge closes".into()));

    print!("{}", ledger_json(&outcomes, &meta));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dur(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_spread_refuses_fewer_than_min_repeats() {
        // The property: a single-run figure must be unrepresentable.
        for n in 0..MIN_REPEATS {
            let samples: Vec<Duration> = (0..n).map(|i| dur(i as u64)).collect();
            assert!(
                Spread::new(samples).is_err(),
                "{n} samples must be refused, not reported"
            );
        }
        assert!(Spread::new((0..MIN_REPEATS).map(|i| dur(i as u64)).collect()).is_ok());
    }

    #[test]
    fn a_spread_reports_a_distribution_not_a_scalar() {
        let s = Spread::new((1..=10).map(dur).collect()).unwrap();
        assert_eq!(s.n, 10);
        assert_eq!(s.min_us, 1_000);
        assert_eq!(s.max_us, 10_000);
        // p50 and p95 are distinct from min and max, so the spread carries shape.
        assert!(s.p50_us > s.min_us && s.p95_us <= s.max_us);
    }

    #[test]
    fn the_ledger_always_states_the_shared_parser_caveat() {
        // Mutation killed: deleting the comparison_scope line.
        let json = ledger_json(&[], &BTreeMap::new());
        assert!(json.contains("comparison_scope"));
        assert!(json.contains("share"));
        assert!(json.contains("spargebra"));
        assert!(json.contains("not an engine-versus-engine comparison"));
    }

    #[test]
    fn the_ledger_states_the_build_profile() {
        // Mutation killed: emitting "release" unconditionally. Under `cargo test`
        // debug_assertions is on, so the debug branch is the one exercised here.
        let json = ledger_json(&[], &BTreeMap::new());
        assert!(json.contains("build_profile"));
        assert!(
            json.contains("DEBUG -- UNOPTIMISED"),
            "a debug build must say so in the ledger: {json}"
        );
    }

    #[test]
    fn the_ledger_records_that_repeats_were_interleaved_and_alternated() {
        let json = ledger_json(&[], &BTreeMap::new());
        assert!(json.contains("\"interleaved\": true"));
        assert!(json.contains("\"order_alternates_by_repeat_parity\": true"));
        assert!(json.contains(&format!("\"min_repeats_enforced\": {MIN_REPEATS}")));
    }

    #[test]
    fn measure_refuses_repeats_below_the_minimum() {
        // Guards the entry point as well as the type: a caller must not be able
        // to ask for a one-shot run at all.
        let qs = quipu::Store::open_in_memory().unwrap();
        let os = oxigraph::store::Store::new().unwrap();
        let err = measure_interleaved(&qs, &os, &[], MIN_REPEATS - 1, 0).unwrap_err();
        assert!(err.contains("below the minimum"), "got: {err}");
    }

    #[test]
    fn an_undeclared_graph_is_refused_for_measurement() {
        // malcolm's ruling: a partial load is indistinguishable from a smaller
        // successful one, so absence of a declaration must REFUSE, not warn.
        let store = quipu::Store::open_in_memory().unwrap();
        let err = assert_declared_complete(&store, "urn:test:nodecl").unwrap_err();
        assert!(err.contains("REFUSING TO MEASURE"), "got: {err}");
    }

    #[test]
    fn a_disagreement_on_row_count_is_recorded() {
        // A comparison between arms that return DIFFERENT answers is not a
        // performance result. The ledger must say so rather than compare timings
        // of two different computations.
        let mut rows = BTreeMap::new();
        rows.insert("quipu", 10usize);
        rows.insert("oxigraph", 11usize);
        let sp = Spread::new((1..=MIN_REPEATS as u64).map(dur).collect()).unwrap();
        let mut per_arm = BTreeMap::new();
        per_arm.insert("quipu", sp.clone());
        per_arm.insert("oxigraph", sp.clone());
        let o = QueryOutcome {
            name: "q".into(),
            per_arm,
            parse_only: sp,
            rows,
        };
        let json = ledger_json(&[o], &BTreeMap::new());
        assert!(
            json.contains("\"arms_agree_on_row_count\": false"),
            "{json}"
        );
    }
}

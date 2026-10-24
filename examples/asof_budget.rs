//! Measure an as-of cut against the design budget (docs/design/as-of-share.md
//! §Budget): read-phase wall time, peak RSS growth and spool size.
//!
//! Run it against a scratch COPY of a production-sized store, never against a
//! live one: opening a store can migrate it.
//!
//! ```text
//! cargo run --release --example asof_budget -- <store.db> <spool-dir> [max-graphs]
//! ```

use std::time::Duration;

use quipu::Store;
use quipu::asof_share::{Limits, Request, cut};

fn status_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix(field)
                    .and_then(|rest| rest.trim().trim_end_matches(" kB").parse().ok())
            })
        })
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (path, spool) = (&args[1], std::path::Path::new(&args[2]));
    let max_graphs: usize = args.get(3).map_or(Ok(64), |n| n.parse())?;
    let store = Store::open(path)?;

    // The largest registered committed graphs by current facts, up to the
    // request cap. Counted before the cut, outside its read phase.
    let counter = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut stmt = counter.prepare(
        "SELECT g, COUNT(*) FROM facts \
         WHERE op = 1 AND retracted_tx IS NULL AND g <> 0 \
         GROUP BY g ORDER BY 2 DESC",
    )?;
    let counted: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let counted: Vec<(String, i64)> = counted
        .into_iter()
        .map(|(g, n)| Ok((store.resolve(g)?, n)))
        .collect::<quipu::Result<_>>()?;
    drop(stmt);
    let total_current: i64 = counter.query_row(
        "SELECT COUNT(*) FROM facts WHERE op = 1 AND retracted_tx IS NULL",
        [],
        |r| r.get(0),
    )?;
    // ROOT first: the default graph is cut only when named.
    let root: i64 = counter.query_row(
        "SELECT COUNT(*) FROM facts WHERE op = 1 AND retracted_tx IS NULL AND g = 0",
        [],
        |r| r.get(0),
    )?;
    let mut graphs = vec!["ROOT".to_string()];
    let mut selected_facts = root;
    for (iri, n) in &counted {
        if graphs.len() == max_graphs {
            break;
        }
        let Some(id) = store.registered_graph_id(iri)? else {
            continue;
        };
        if store.graph_class(id)?.as_deref() != Some("committed") || store.is_dataset(iri)? {
            println!("skipped {iri} ({n} facts): not a committed graph");
            continue;
        }
        graphs.push(iri.clone());
        selected_facts += n;
    }
    println!(
        "store: {path}\ncurrent facts: {total_current}\ngraphs selected: {} of {} with facts, \
         covering {selected_facts} current facts ({:.1}%)",
        graphs.len(),
        counted.len(),
        100.0 * selected_facts as f64 / total_current.max(1) as f64
    );

    drop(counter);
    let request = Request {
        graphs,
        context_graphs: Vec::new(),
        tx: None,
        expected_store_id: store.store_id()?,
    };
    let limits = Limits {
        // Measure, do not abort: the targets are judged on the numbers.
        read_timeout: Duration::from_secs(3600),
        wal_growth_max_bytes: u64::MAX,
        spool_quota_bytes: u64::MAX,
        ..Limits::default()
    };
    // Reset the peak so it describes this cut alone (Linux clear_refs 5).
    std::fs::write("/proc/self/clear_refs", "5")?;
    let rss_before = status_kib("VmRSS:");
    let result = cut(&store, &request, spool, &limits);
    let peak = status_kib("VmHWM:");
    let cut = match result {
        Ok(cut) => cut,
        Err(error) => return Err(format!("cut failed: {error:?}").into()),
    };
    let spool_bytes = std::fs::metadata(&cut.spool)?.len();
    let secs = cut.read_phase.as_secs_f64();
    println!(
        "profile: {}\nwatermark_tx: {}\nrecords: {}\nparts: {}\nread phase: {secs:.1} s \
         (target <= 120 s)\nrss before: {} MiB\npeak rss growth: {} MiB (target <= 256 MiB)\n\
         spool: {:.2} GiB ({:.0} bytes/record)\nper record: {:.2} us",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        cut.manifest.watermark_tx,
        cut.parts.records,
        cut.parts.parts,
        rss_before / 1024,
        peak.saturating_sub(rss_before) / 1024,
        spool_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        spool_bytes as f64 / cut.parts.records.max(1) as f64,
        secs * 1e6 / cut.parts.records.max(1) as f64,
    );
    let p = &cut.profile;
    // The baselines' boundary: the read plus the digest-run merge after it.
    let with_merge = secs + p.finish.as_secs_f64();
    println!(
        "read phase + digest merge: {with_merge:.1} s (target <= 120 s, the baseline boundary)"
    );
    let n = cut.parts.records.max(1) as f64;
    println!("per-record split (us/record, % of read phase):");
    for (name, d) in [
        ("sql_step", p.sql_step),
        ("decode", p.decode),
        ("tx_meta", p.tx_meta),
        ("terms", p.terms),
        ("value_json", p.value_json),
        ("serialize", p.serialize),
        ("digest", p.digest),
        ("frame", p.frame),
        ("other", p.other),
        ("finish", p.finish),
    ] {
        println!(
            "  {name:<11} {:>7.2} us  {:>5.1}%",
            d.as_secs_f64() * 1e6 / n,
            100.0 * d.as_secs_f64() / secs
        );
    }
    std::fs::remove_file(&cut.spool)?;
    Ok(())
}

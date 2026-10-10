//! Measure bounded FTS5 backfill only on a restored database under `temp_dir`.
//! Run: `cargo run --example lexical_profile -- /tmp/restored.db 500`

use quipu::Store;

fn rss_kib(field: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(field)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = std::fs::canonicalize(args.get(1).ok_or("supply restored temp database path")?)?;
    if !path.starts_with(std::fs::canonicalize(std::env::temp_dir())?) {
        return Err(
            "profile refuses paths outside canonical temp_dir; restore an isolated copy first"
                .into(),
        );
    }
    let batch = args.get(2).map_or(Ok(500), |n| n.parse::<usize>())?;
    let store = Store::open(path.to_str().ok_or("non-UTF8 database path")?)?;
    let before = rss_kib("VmRSS:");
    let started = std::time::Instant::now();
    let mut batches = 0;
    let mut max_batch_ms = 0;
    loop {
        let now = quipu::time::now_iso();
        let minute = now
            .get(14..16)
            .ok_or("unreadable UTC minute")?
            .parse::<u32>()?;
        if (10..=20).contains(&minute) {
            return Err(
                "backfill paused in protected ingest lane; rerun outside UTC minutes10..20".into(),
            );
        }
        let tick = std::time::Instant::now();
        let progress = store.backfill_lexical_batch(batch)?;
        max_batch_ms = max_batch_ms.max(tick.elapsed().as_millis());
        batches += 1;
        if progress.complete {
            println!(
                "{}",
                serde_json::json!({"index":progress,"batches":batches,
                "elapsed_ms":started.elapsed().as_millis(),"max_batch_ms":max_batch_ms,
                "rss_before_kib":before,"rss_after_kib":rss_kib("VmRSS:"),
                "peak_rss_kib":rss_kib("VmHWM:"),"batch_size":batch})
            );
            break;
        }
        if batches % 100 == 0 {
            eprintln!(
                "{}",
                serde_json::json!({"batches":batches,"index":progress})
            );
        }
    }
    Ok(())
}

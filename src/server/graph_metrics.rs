//! Graph counts refreshed independently of scrapes. The aggregate scans the
//! live root graph; it must never run once per monitoring request.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::SharedStore;

const REFRESH_INTERVAL: Duration = Duration::from_secs(300);
type Counts = (u64, u64, u64);

#[derive(Clone, Copy)]
struct Snapshot {
    counts: Counts,
    completed: Instant,
    unix_seconds: u64,
}

#[derive(Default)]
struct State {
    snapshot: Option<Snapshot>,
    failures: u64,
    duration: f64,
}

/// Owned by one served store, so neither values nor refresh failures leak
/// between stores. The mutex protects only a few scalars, never database work.
pub(crate) struct GraphMetrics {
    state: Mutex<State>,
    wal_path: Option<PathBuf>,
}

impl GraphMetrics {
    pub(crate) fn new(db_path: &str) -> Self {
        Self {
            state: Mutex::new(State::default()),
            wal_path: (db_path != ":memory:").then(|| PathBuf::from(format!("{db_path}-wal"))),
        }
    }

    fn record(&self, result: quipu::Result<Counts>, started: Instant) {
        let mut state = self.state.lock();
        state.duration = started.elapsed().as_secs_f64();
        match result {
            Ok(counts) => {
                state.snapshot = Some(Snapshot {
                    counts,
                    completed: Instant::now(),
                    unix_seconds: quipu::time::epoch_secs(),
                });
            }
            Err(error) => {
                state.failures += 1;
                drop(state);
                eprintln!(
                    "graph metrics refresh failed (retaining last successful counts): {error}"
                );
            }
        }
    }

    pub(crate) fn render(&self) -> String {
        let state = self.state.lock();
        let (snapshot, failures, duration) = (state.snapshot, state.failures, state.duration);
        drop(state);
        // Keep the WAL gauge current without acquiring any database connection.
        // Missing/unreadable files remain unknown, never a fabricated zero.
        let wal_bytes = self
            .wal_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        let mut body = quipu::metrics::metrics()
            .render_with_graph_counts(snapshot.map(|s| s.counts), wal_bytes);
        body.push_str("# HELP quipu_graph_counts_ready Whether a graph-count refresh has succeeded.\n# TYPE quipu_graph_counts_ready gauge\n");
        let _ = writeln!(
            body,
            "quipu_graph_counts_ready {}",
            u8::from(snapshot.is_some())
        );
        body.push_str("# HELP quipu_graph_counts_refresh_failures_total Failed background count refreshes.\n# TYPE quipu_graph_counts_refresh_failures_total counter\n");
        let _ = writeln!(body, "quipu_graph_counts_refresh_failures_total {failures}");
        body.push_str("# HELP quipu_graph_counts_refresh_duration_seconds Duration of the last refresh attempt, including pool wait.\n# TYPE quipu_graph_counts_refresh_duration_seconds gauge\n");
        let _ = writeln!(
            body,
            "quipu_graph_counts_refresh_duration_seconds {duration}"
        );
        if let Some(snapshot) = snapshot {
            body.push_str("# HELP quipu_graph_counts_age_seconds Age of the last successful graph-count snapshot.\n# TYPE quipu_graph_counts_age_seconds gauge\n");
            let _ = writeln!(
                body,
                "quipu_graph_counts_age_seconds {}",
                snapshot.completed.elapsed().as_secs_f64()
            );
            body.push_str("# HELP quipu_graph_counts_last_success_timestamp_seconds Unix time of the last successful graph-count refresh.\n# TYPE quipu_graph_counts_last_success_timestamp_seconds gauge\n");
            let _ = writeln!(
                body,
                "quipu_graph_counts_last_success_timestamp_seconds {}",
                snapshot.unix_seconds
            );
        }
        body
    }
}

/// First refresh is immediate; each subsequent one starts five minutes after
/// completion. Awaiting each scan bounds the work to one even on a slow store.
/// Scrapes never start refreshes, and a dropped handle stops this task.
pub(crate) fn spawn_refresh(store: &SharedStore) {
    let weak = Arc::downgrade(store);
    tokio::spawn(async move {
        loop {
            let Some(store) = weak.upgrade() else { break };
            let started = Instant::now();
            let reader = store.clone();
            let result = tokio::task::spawn_blocking(move || reader.read().graph_counts()).await;
            let result = result.unwrap_or_else(|e| {
                Err(quipu::Error::InvalidValue(format!(
                    "graph metrics task failed: {e}"
                )))
            });
            store.graph_metrics.record(result, started);
            drop(store);
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

#[cfg(test)]
#[path = "graph_metrics_tests.rs"]
mod tests;

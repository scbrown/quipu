//! Bounded, anonymous diagnostics for completed slow HTTP searches.
//!
//! The scope lives inside blocking search work. Library/CLI vector operations
//! without that scope do not acquire a clock, read procfs, or emit a record.

use std::cell::RefCell;
use std::io::Write;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::time::Stopwatch;

/// Search handlers sharing the embedding and retrieval pipeline.
#[derive(Clone, Copy)]
pub enum Endpoint {
    /// Semantic or keyword search.
    Search,
    /// Hybrid vector search.
    HybridSearch,
    /// Structured search-query endpoint.
    SearchQuery,
}

impl Endpoint {
    fn label(self) -> &'static str {
        match self {
            Self::Search => "/search",
            Self::HybridSearch => "/hybrid-search",
            Self::SearchQuery => "/search-query",
        }
    }
}

/// A fixed vocabulary; query text and caller-controlled labels never enter it.
#[derive(Clone, Copy)]
pub enum Phase {
    /// Query embedding, including any provider-internal wait.
    Embedding,
    /// Acquiring a vector-compatible reader, including attachment sync.
    Reader,
    /// The complete search tool, containing the SQLite phases below.
    Tool,
    /// Fetching, decoding, and scoring SQLite embedding rows.
    Scan,
    /// Sorting scores and selecting survivors.
    Sort,
    /// Loading survivor text and temporal metadata.
    Metadata,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Embedding => "embedding",
            Self::Reader => "reader_acquisition",
            Self::Tool => "search_tool",
            Self::Scan => "sqlite_scan_decode_score",
            Self::Sort => "sqlite_sort",
            Self::Metadata => "sqlite_metadata",
        }
    }
}

const PHASES: [Phase; 6] = [
    Phase::Embedding,
    Phase::Reader,
    Phase::Tool,
    Phase::Scan,
    Phase::Sort,
    Phase::Metadata,
];

type State = Arc<Mutex<[Option<u128>; PHASES.len()]>>;

thread_local! {
    static ACTIVE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct Scope(Option<State>);

impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = self.0.take());
    }
}

/// Records even a phase that returns an error. Dropping it never reads procfs.
pub struct PhaseTimer {
    state: State,
    phase: Phase,
    start: Stopwatch,
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let mut state = self.state.lock();
        let old = state[self.phase as usize].unwrap_or(0);
        state[self.phase as usize] = Some(old.saturating_add(self.start.elapsed_ms()));
    }
}

/// Begin a phase only when the calling thread has an active HTTP search scope.
#[must_use]
pub fn phase(phase: Phase) -> Option<PhaseTimer> {
    ACTIVE.with(|active| {
        active.borrow().as_ref().map(|state| PhaseTimer {
            state: Arc::clone(state),
            phase,
            start: Stopwatch::start(),
        })
    })
}

#[derive(Clone, Copy)]
struct ThreadIo {
    logical: u64,
    physical: u64,
}

impl ThreadIo {
    fn parse(text: &str) -> Option<Self> {
        let value = |name| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key == name).then(|| value.trim().parse().ok()).flatten()
            })
        };
        Some(Self {
            logical: value("rchar")?,
            physical: value("read_bytes")?,
        })
    }

    fn read() -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            Self::parse(&std::fs::read_to_string("/proc/thread-self/io").ok()?)
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    fn delta(self, before: Self) -> Option<Self> {
        Some(Self {
            logical: self.logical.checked_sub(before.logical)?,
            physical: self.physical.checked_sub(before.physical)?,
        })
    }
}

fn record(
    endpoint: Endpoint,
    elapsed: u128,
    dispatch: u128,
    state: &State,
    io: Option<ThreadIo>,
    ok: bool,
) -> Value {
    let values = state.lock();
    let phases: serde_json::Map<String, Value> = PHASES
        .iter()
        .map(|phase| (phase.label().to_owned(), json!(values[*phase as usize])))
        .collect();
    json!({
        "event": "search_phase_slow",
        "timestamp": crate::time::now_iso(),
        "endpoint": endpoint.label(),
        "pipeline_ms": elapsed,
        "admission_dispatch_ms": dispatch,
        "phase_ms": phases,
        "thread_io_known": io.is_some(),
        "thread_logical_read_bytes": io.map(|value| value.logical),
        "thread_physical_read_bytes": io.map(|value| value.physical),
        "ok": ok,
    })
}

/// Trace blocking work whose admission/dispatch clock began in the HTTP handler.
///
/// Emits one JSON line only when that pipeline exceeds 500ms. An IO read or
/// diagnostic write failure cannot change the operation's result. The caller's
/// lock and admission guards stay in their existing scopes. Cancelled requests
/// that never enter blocking work have no phase record.
pub fn run<T, E>(
    endpoint: Endpoint,
    queued: Stopwatch,
    work: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let dispatch = queued.elapsed_ms();
    let state = Arc::new(Mutex::new([None; PHASES.len()]));
    let previous = ACTIVE.with(|active| active.replace(Some(Arc::clone(&state))));
    let _scope = Scope(previous);
    let before = ThreadIo::read();
    let result = work();
    let elapsed = queued.elapsed_ms();
    if elapsed > 500 {
        let io = ThreadIo::read().and_then(|after| after.delta(before?));
        let value = record(endpoint, elapsed, dispatch, &state, io, result.is_ok());
        let _ = writeln!(std::io::stderr().lock(), "{value}");
    }
    result
}

#[cfg(test)]
mod tests;

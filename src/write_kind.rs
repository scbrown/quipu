//! Which code path a write came through, as a CLOSED set (aegis-gwkd76).
//!
//! `quipu_facts_written_total` was one unlabeled number, so 7.4M datums a day
//! against ~1M/week of net growth could not be attributed. Labelling it by the
//! client-supplied `source` tag is unbounded cardinality, so the label is the
//! code path instead: an HTTP write endpoint (mapped from the route template by
//! one total table over `http_auth::WRITE_ENDPOINTS`), an internal writer that
//! sets its own scope, the reasoner (recognised by its actor or engine source),
//! or STARTUP: anything committed before the server begins serving, which is
//! what the 28 restarts a day multiply.

use std::cell::Cell;
use std::sync::atomic::{AtomicU8, Ordering};

/// The writer label. Adding a variant is a deliberate change to a metric
/// label set; `ALL` must list it and the tests enforce that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteKind {
    Episode,
    Knot,
    Promote,
    Import,
    Update,
    Set,
    Retract,
    Proposal,
    Overlay,
    GraphAdmin,
    Derive,
    Ontology,
    Reasoner,
    Migration,
    Verdict,
    Startup,
    Cli,
    Unclassified,
}

impl WriteKind {
    /// Every variant, for rendering and for the totality tests.
    pub const ALL: [WriteKind; 18] = [
        Self::Episode,
        Self::Knot,
        Self::Promote,
        Self::Import,
        Self::Update,
        Self::Set,
        Self::Retract,
        Self::Proposal,
        Self::Overlay,
        Self::GraphAdmin,
        Self::Derive,
        Self::Ontology,
        Self::Reasoner,
        Self::Migration,
        Self::Verdict,
        Self::Startup,
        Self::Cli,
        Self::Unclassified,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::Knot => "knot",
            Self::Promote => "promote",
            Self::Import => "import",
            Self::Update => "update",
            Self::Set => "set",
            Self::Retract => "retract",
            Self::Proposal => "proposal",
            Self::Overlay => "overlay",
            Self::GraphAdmin => "graph_admin",
            Self::Derive => "derive",
            Self::Ontology => "ontology",
            Self::Reasoner => "reasoner",
            Self::Migration => "migration",
            Self::Verdict => "verdict",
            Self::Startup => "startup",
            Self::Cli => "cli",
            Self::Unclassified => "unclassified",
        }
    }

    /// The kind of an HTTP write endpoint, from its route template. Total over
    /// `WRITE_ENDPOINTS`: a write route this table does not name is a test
    /// failure, not a silent `unclassified` (see the tests).
    #[must_use]
    pub fn for_route(template: &str) -> Option<Self> {
        Some(match template {
            "/episode" | "/episodes/complete" | "/episode/retract" => Self::Episode,
            "/knot" | "/knot/stage" => Self::Knot,
            "/knot/promote" | "/import/promote" => Self::Promote,
            "/import" => Self::Import,
            "/update" => Self::Update,
            "/set" => Self::Set,
            "/retract" | "/retract/source" => Self::Retract,
            "/propose" | "/proposal/accept" | "/proposal/reject" => Self::Proposal,
            "/overlay/write" | "/overlay/create" => Self::Overlay,
            "/graph/create" | "/graph/label" | "/graph/freeze" | "/graph/thaw" | "/datasets"
            | "/queries" | "/shapes" | "/subscriptions" | "/events/commit" => Self::GraphAdmin,
            "/project" | "/impact" | "/embed_backfill" | "/align/apply" => Self::Derive,
            "/ontology" => Self::Ontology,
            "/reason" => Self::Reasoner,
            _ => return None,
        })
    }
}

thread_local! {
    static CURRENT: Cell<Option<WriteKind>> = const { Cell::new(None) };
}

/// What an unscoped write in this process is: STARTUP until the server begins
/// serving, then UNCLASSIFIED (a path nobody scoped, which is a finding); a
/// CLI process is CLI throughout.
const STARTING: u8 = 0;
const SERVING: u8 = 1;
const CLI: u8 = 2;
static MODE: AtomicU8 = AtomicU8::new(STARTING);

/// Mark the end of server startup.
pub fn set_serving() {
    MODE.store(SERVING, Ordering::Relaxed);
}

/// Mark this process as the command-line tool.
pub fn set_cli() {
    MODE.store(CLI, Ordering::Relaxed);
}

/// Run `f` with `kind` as this thread's write path, restoring the previous one.
pub fn scoped<R>(kind: Option<WriteKind>, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT.with(|c| c.replace(kind.or(c.get())));
    struct Restore(Option<WriteKind>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(previous);
    f()
}

/// The scoped kind on this thread, if any (to carry it across a thread hop).
#[must_use]
pub fn current() -> Option<WriteKind> {
    CURRENT.with(Cell::get)
}

/// Classify one committed write. The engine's own writers are recognised by
/// what they ARE (actor / source), ahead of the scope of the request that
/// happened to trigger them: inference materialized during an `/ontology` load
/// is reasoner work, and should be counted as such.
#[must_use]
pub fn classify(actor: Option<&str>, source: Option<&str>) -> WriteKind {
    if source == Some(crate::store::inferred::MIGRATE_SOURCE) {
        return WriteKind::Migration;
    }
    if actor == Some("reasoner") || source == Some(crate::store::inferred::PLANE_SOURCE) {
        return WriteKind::Reasoner;
    }
    current().unwrap_or(match MODE.load(Ordering::Relaxed) {
        SERVING => WriteKind::Unclassified,
        CLI => WriteKind::Cli,
        _ => WriteKind::Startup,
    })
}

#[cfg(test)]
#[path = "write_kind_tests.rs"]
mod tests;

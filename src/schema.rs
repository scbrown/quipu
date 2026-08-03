/// The reserved ROOT / default committed graph (quipu #36).
///
/// Term ids are rowids and always `>= 1`, so `0` can never collide with a named
/// graph's id. Committed reads are ROOT-scoped by default (Decision 4, see
/// `docs/design/named-graphs.md` §4) — a read that omits this predicate spans
/// every tenant overlay, which is the defect quipu #56 fixed.
pub const ROOT_GRAPH: i64 = 0;

/// The IRI naming the ROOT graph in authority grants. ROOT has term id 0 rather
/// than an interned IRI, so an authority covering it needs a name to grant.
pub const ROOT_GRAPH_IRI: &str = "urn:quipu:graph:root";

/// SQL statements for initialising the Quipu fact log schema.
pub const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS terms (
    id  INTEGER PRIMARY KEY,
    iri TEXT    NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS transactions (
    id        INTEGER PRIMARY KEY,
    timestamp TEXT    NOT NULL,
    actor     TEXT,
    source    TEXT
);

CREATE TABLE IF NOT EXISTS facts (
    e         INTEGER NOT NULL,
    a         INTEGER NOT NULL,
    v         BLOB    NOT NULL,
    -- Named graph (aegis-g1al / quipu #36). g=0 is the reserved ROOT / default
    -- graph (the source of truth, per Stiwi's sign-off); a named-graph OVERLAY
    -- uses the term id of its graph IRI (term ids are rowids, always >= 1, so 0
    -- never collides). g is NOT in the PK: each graph-write is its own tx, so a
    -- base fact and an overlay fact for the same (e,a,v) already coexist as
    -- separate rows keyed by tx — g just denormalizes tx->graph for query-time
    -- dataset filtering. No table rebuild needed; the column is purely additive.
    g         INTEGER NOT NULL DEFAULT 0,
    tx        INTEGER NOT NULL REFERENCES transactions(id),
    valid_from TEXT   NOT NULL,
    valid_to   TEXT,
    op        INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (e, a, v, tx)
);

-- Index permutations for the four standard Datomic-style access patterns.
CREATE INDEX IF NOT EXISTS idx_eavt ON facts(e, a, v, valid_from);
CREATE INDEX IF NOT EXISTS idx_aevt ON facts(a, e, v, valid_from);
CREATE INDEX IF NOT EXISTS idx_vaet ON facts(v, a, e, valid_from);
CREATE INDEX IF NOT EXISTS idx_tx   ON facts(tx);
-- NOTE: the graph-scoped index idx_geav ON facts(g, ...) is created in
-- Store::migrate_named_graphs, NOT here. INIT_SQL runs against pre-quad stores
-- too (CREATE TABLE IF NOT EXISTS is a no-op there), and a CREATE INDEX on the
-- not-yet-added `g` column would hard-fail with `no such column: g` before the
-- migration's ALTER could add it. The migration owns both the ALTER and the index.

-- Named-graph registry (aegis-g1al / quipu #36). One row per graph, keyed by
-- the graph's term id (g). g=0 is the reserved ROOT / default committed graph.
-- `class` is the enforced committed|overlay invariant: overlay-class graphs are
-- transient, compose-only, and excluded from bitemporal/SHACL/promotion.
-- `parent_branch` binds an overlay to its committed parent branch AT CREATE
-- (bind-once) — compose resolves an overlay against exactly this root, never a
-- blanket union; NULL for committed graphs.
CREATE TABLE IF NOT EXISTS graphs (
    g             INTEGER PRIMARY KEY,
    class         TEXT    NOT NULL CHECK (class IN ('committed','overlay')),
    parent_branch INTEGER REFERENCES graphs(g),
    created_at    TEXT    NOT NULL
);
-- Seed the ROOT graph (g=0, committed, self-rooted). Idempotent.
INSERT OR IGNORE INTO graphs (g, class, parent_branch, created_at)
    VALUES (0, 'committed', NULL, '1970-01-01T00:00:00Z');

-- Persistent SHACL shape storage for auto-validation on writes.
CREATE TABLE IF NOT EXISTS shapes (
    name      TEXT PRIMARY KEY,
    turtle    TEXT NOT NULL,
    loaded_at TEXT NOT NULL
);

-- Schema evolution proposals for agent-driven ontology changes.
CREATE TABLE IF NOT EXISTS proposals (
    id            INTEGER PRIMARY KEY,
    kind          TEXT NOT NULL,
    target        TEXT NOT NULL,
    diff          TEXT NOT NULL,
    rationale     TEXT,
    proposer      TEXT NOT NULL,
    trigger_ref   TEXT,
    status        TEXT NOT NULL DEFAULT 'pending',
    decided_by    TEXT,
    decided_at    TEXT,
    decision_note TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_proposals_status ON proposals(status, created_at);

-- Persistent OWL ontology storage for class hierarchy and reasoning.
CREATE TABLE IF NOT EXISTS ontologies (
    name      TEXT PRIMARY KEY,
    turtle    TEXT NOT NULL,
    loaded_at TEXT NOT NULL
);

-- Event log (event-log P1): append-only, offset-ordered log of
-- graph-change events, written IN THE SAME TRANSACTION as the graph write so
-- offset order == commit order and a rolled-back write leaves no event behind.
-- Durable append-only per the durable-retention decision: no expiry — a consumer
-- down for weeks replays from its committed offset (the reactor-down-6wk fix).
-- AUTOINCREMENT (not bare rowid) so offsets are never reused even after a
-- savepoint rollback discards staged rows — a consumer's cursor can trust that
-- offset N is forever the same event or a gap, never a DIFFERENT event.
CREATE TABLE IF NOT EXISTS events (
    "offset"  INTEGER PRIMARY KEY AUTOINCREMENT,
    type      TEXT    NOT NULL,
    ts        TEXT    NOT NULL,
    subject   TEXT,
    group_id  TEXT,
    tx_id     INTEGER NOT NULL,
    payload   TEXT    NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_events_type  ON events(type, "offset");
CREATE INDEX IF NOT EXISTS idx_events_group ON events(group_id, "offset");

-- Durable consumer cursor registry (pull-resume). committed_offset is the
-- HIGHEST offset the consumer has processed; resume returns events AFTER it.
CREATE TABLE IF NOT EXISTS consumers (
    consumer_id      TEXT PRIMARY KEY,
    committed_offset INTEGER NOT NULL DEFAULT 0,
    filter           TEXT,
    updated_at       TEXT
);

-- Event push subscriptions (event-log P2): who wants which events delivered
-- where. Delivery cursors reuse the consumers table under "sub:<id>", so the
-- pull and push APIs share one at-least-once offset semantics.
CREATE TABLE IF NOT EXISTS subscriptions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    consumer_id   TEXT    NOT NULL UNIQUE,
    types         TEXT,                 -- JSON array of event types; NULL = all
    sparql_ask    TEXT,                 -- reserved; creation REFUSES it until evaluated
    mode          TEXT    NOT NULL DEFAULT 'realtime',  -- realtime | batch
    webhook_url   TEXT    NOT NULL,
    batch_size    INTEGER NOT NULL DEFAULT 50,
    batch_window_s INTEGER NOT NULL DEFAULT 30,
    created_at    TEXT    NOT NULL
);

-- Schema-term first-sight registry (a hard requirement of the event-log P1
-- spec): powers type.new / predicate.new — emitted exactly ONCE, the first
-- time a node type or a predicate IRI is observed in the store. first_offset
-- records the event that announced it.
CREATE TABLE IF NOT EXISTS schema_terms (
    term         TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('type','predicate')),
    first_offset INTEGER NOT NULL,
    PRIMARY KEY (term, kind)
);
"#;

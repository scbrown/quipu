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
"#;

//! The respace column classification: every column of every table quipu
//! creates, and whether it carries term identity. Split from
//! `store/respace.rs` for the file-size ratchet — the module docs there
//! explain why the LIVE schema is iterated and this table only consulted.
//! Public paths are unchanged (`respace` re-exports both items).

/// How a stored column relates to term identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermIdKind {
    /// Holds a term id directly. Remapped arithmetically.
    ///
    /// [`ROOT_GRAPH`](crate::schema::ROOT_GRAPH) (`0`) is exempt wherever it can appear: it is a reserved
    /// sentinel naming the default committed graph, not an interned term, and
    /// it means the same thing in every space. Shifting it would invent a graph
    /// id that no `terms` row backs.
    Id,
    /// Holds a `Value` blob. `Value::Ref` embeds a term id; every other variant
    /// is opaque payload and is left exactly as it is.
    RefBlob,
    /// Carries no term identity — remapping it would corrupt the store.
    ///
    /// Note what is deliberately here: `facts.tx` and `graphs.labels_tx` are
    /// transaction ids, `term_spaces.space` is a space number, `events.subject`
    /// and `dataset_members.graph_iri` are IRIs. Several are integers that look
    /// exactly like term ids and are not.
    None,
}

/// Every column of every table quipu creates, and whether it carries term
/// identity.
///
/// Kept in schema order (`schema::INIT_SQL`, then the `migrate_*` functions,
/// then `vector::VECTORS_SQL`) so it reads against the DDL side by side.
///
/// This is **not** the list respace iterates. Respace iterates the live
/// schema and consults this; the difference is the whole point, because a
/// column that exists in the database and not here is the failure case, and a
/// list that drives the work can never notice it.
pub const COLUMN_CLASSIFICATION: &[(&str, &str, TermIdKind)] = &[
    // -- schema::INIT_SQL --
    ("terms", "id", TermIdKind::Id),
    ("terms", "iri", TermIdKind::None),
    ("transactions", "id", TermIdKind::None),
    ("transactions", "timestamp", TermIdKind::None),
    ("transactions", "actor", TermIdKind::None),
    ("transactions", "source", TermIdKind::None),
    ("store_identity", "id", TermIdKind::None),
    ("store_identity", "store_id", TermIdKind::None),
    ("facts", "e", TermIdKind::Id),
    ("facts", "a", TermIdKind::Id),
    ("facts", "v", TermIdKind::RefBlob),
    ("facts", "g", TermIdKind::Id),
    ("facts", "tx", TermIdKind::None),
    ("facts", "valid_from", TermIdKind::None),
    ("facts", "valid_to", TermIdKind::None),
    ("facts", "op", TermIdKind::None),
    ("graphs", "g", TermIdKind::Id),
    ("graphs", "class", TermIdKind::None),
    ("graphs", "parent_branch", TermIdKind::Id),
    ("graphs", "created_at", TermIdKind::None),
    ("shapes", "name", TermIdKind::None),
    ("shapes", "turtle", TermIdKind::None),
    ("shapes", "loaded_at", TermIdKind::None),
    ("proposals", "id", TermIdKind::None),
    ("proposals", "kind", TermIdKind::None),
    ("proposals", "target", TermIdKind::None),
    ("proposals", "diff", TermIdKind::None),
    ("proposals", "rationale", TermIdKind::None),
    ("proposals", "proposer", TermIdKind::None),
    ("proposals", "trigger_ref", TermIdKind::None),
    ("proposals", "status", TermIdKind::None),
    ("proposals", "decided_by", TermIdKind::None),
    ("proposals", "decided_at", TermIdKind::None),
    ("proposals", "decision_note", TermIdKind::None),
    ("proposals", "created_at", TermIdKind::None),
    ("ontologies", "name", TermIdKind::None),
    ("ontologies", "turtle", TermIdKind::None),
    ("ontologies", "loaded_at", TermIdKind::None),
    // `events.subject` and every id inside `events.payload` are RESOLVED IRIs,
    // not term ids — `store::events` calls `resolve_cached` before writing
    // either. Checked, because an integer-bearing JSON payload is exactly where
    // a term id would hide from this table.
    ("events", "offset", TermIdKind::None),
    ("events", "type", TermIdKind::None),
    ("events", "ts", TermIdKind::None),
    ("events", "subject", TermIdKind::None),
    ("events", "group_id", TermIdKind::None),
    ("events", "tx_id", TermIdKind::None),
    ("events", "payload", TermIdKind::None),
    ("consumers", "consumer_id", TermIdKind::None),
    ("consumers", "committed_offset", TermIdKind::None),
    ("consumers", "filter", TermIdKind::None),
    ("consumers", "updated_at", TermIdKind::None),
    ("subscriptions", "id", TermIdKind::None),
    ("subscriptions", "consumer_id", TermIdKind::None),
    ("subscriptions", "types", TermIdKind::None),
    ("subscriptions", "sparql_ask", TermIdKind::None),
    ("subscriptions", "mode", TermIdKind::None),
    ("subscriptions", "webhook_url", TermIdKind::None),
    ("subscriptions", "batch_size", TermIdKind::None),
    ("subscriptions", "batch_window_s", TermIdKind::None),
    ("subscriptions", "created_at", TermIdKind::None),
    ("schema_terms", "term", TermIdKind::None),
    ("schema_terms", "kind", TermIdKind::None),
    ("schema_terms", "first_offset", TermIdKind::None),
    ("term_spaces", "space", TermIdKind::None),
    ("term_spaces", "db", TermIdKind::None),
    ("term_spaces", "local", TermIdKind::None),
    // -- Store::migrate_graph_labels (quipu #65) --
    // `trust_chain` is TEXT holding the chain's IRI, deliberately not interned,
    // precisely to stay off this surface. `labels_tx` is a transaction id.
    ("graphs", "fresh_rank", TermIdKind::None),
    ("graphs", "durability_rank", TermIdKind::None),
    ("graphs", "trust_rank", TermIdKind::None),
    ("graphs", "trust_chain", TermIdKind::None),
    ("graphs", "policy", TermIdKind::None),
    ("graphs", "labels_tx", TermIdKind::None),
    ("graphs", "labels_valid_to", TermIdKind::None),
    // Kind and lifecycle are TEXT tokens (`archive`, `frozen`), never ids.
    ("graphs", "data_kind", TermIdKind::None),
    ("graphs", "lifecycle", TermIdKind::None),
    // The frozen-pack registry: IRIs, paths and hashes as TEXT; `space` is a
    // term-SPACE number, not a term id, so nothing here carries term identity.
    ("frozen_packs", "id", TermIdKind::None),
    ("frozen_packs", "graph_iri", TermIdKind::None),
    ("frozen_packs", "alias", TermIdKind::None),
    ("frozen_packs", "path", TermIdKind::None),
    ("frozen_packs", "space", TermIdKind::None),
    ("frozen_packs", "content_hash", TermIdKind::None),
    ("frozen_packs", "frozen_at", TermIdKind::None),
    ("frozen_packs", "thawed_at", TermIdKind::None),
    // -- attach::migrate_graph_source (quipu #75) --
    // TEXT holding an attachment ALIAS, not a term id. Added by #75 — and this
    // entry exists because respace REFUSED to run until it did, one hour after
    // the gate was built, in the ordinary course of building the next issue.
    // That is the acceptance-5 mechanism working on its author.
    ("graphs", "source", TermIdKind::None),
    // -- Store::migrate_datasets (quipu #69) --
    // Members are graph IRIs, not term ids, for the same reason.
    ("datasets", "name", TermIdKind::None),
    ("datasets", "created_at", TermIdKind::None),
    ("dataset_members", "dataset", TermIdKind::None),
    ("dataset_members", "graph_iri", TermIdKind::None),
    ("dataset_members", "ord", TermIdKind::None),
    // -- Store::migrate_bitemporal_registries (quipu #71) --
    ("shapes", "valid_from", TermIdKind::None),
    ("shapes", "valid_to", TermIdKind::None),
    ("shapes", "tx", TermIdKind::None),
    ("ontologies", "valid_from", TermIdKind::None),
    ("ontologies", "valid_to", TermIdKind::None),
    ("ontologies", "tx", TermIdKind::None),
    // -- Store::migrate_query_registry (quipu #79) --
    ("queries", "name", TermIdKind::None),
    ("queries", "description", TermIdKind::None),
    ("queries", "template", TermIdKind::None),
    ("queries", "dataset", TermIdKind::None),
    ("queries", "valid_from", TermIdKind::None),
    ("queries", "valid_to", TermIdKind::None),
    ("queries", "tx", TermIdKind::None),
    ("query_params", "name", TermIdKind::None),
    ("query_params", "valid_from", TermIdKind::None),
    ("query_params", "ord", TermIdKind::None),
    ("query_params", "param", TermIdKind::None),
    ("query_params", "kind", TermIdKind::None),
    ("query_params", "required", TermIdKind::None),
    ("query_params", "default_val", TermIdKind::None),
    ("query_params", "description", TermIdKind::None),
    // -- Store::migrate_retraction_tx (quipu #83) --
    ("facts", "retracted_tx", TermIdKind::None),
    // -- Store::migrate_forks (quipu-gp5) --
    // `g` is the fork graph's term id; `parent_branch` matches
    // `graphs.parent_branch` (v1 only writes ROOT `0`, the exempt sentinel).
    // `fork_tx` is a TRANSACTION id, like `facts.tx` — an integer that looks
    // exactly like a term id and is not.
    ("forks", "name", TermIdKind::None),
    ("forks", "g", TermIdKind::Id),
    ("forks", "parent_branch", TermIdKind::Id),
    ("forks", "fork_tx", TermIdKind::None),
    ("forks", "created_at", TermIdKind::None),
    ("forks", "status", TermIdKind::None),
    // -- vector::VECTORS_SQL --
    // `entity_id` IS a term id: `embedding::build_entity_text` feeds it straight
    // to `Store::entity_facts`, i.e. it is a `facts.e`. Nothing in #74's scope,
    // its acceptance amendment, or any comment in the store named this column —
    // it was found by enumerating the schema, which is the argument for
    // enumerating the schema.
    ("vectors", "entity_id", TermIdKind::Id),
    ("vectors", "text", TermIdKind::None),
    ("vectors", "embedding", TermIdKind::None),
    ("vectors", "valid_from", TermIdKind::None),
    ("vectors", "valid_to", TermIdKind::None),
];

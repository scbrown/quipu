//! Central namespace constants for RDF/OWL/SHACL URIs.
//!
//! All namespace prefixes and commonly-used IRIs are defined here to avoid
//! hardcoded strings scattered across the codebase.

// ── Project namespace ──────────────────────────────────────────

/// Default base namespace for the Aegis ontology. The fallback when no
/// namespace is configured; a deployment overrides it via `[quipu].base_ns`,
/// which the server applies with `Store::set_base_ns` at startup so the episode
/// write paths mint IRIs under it (aegis-4h3x). The CLI honours the same config
/// value and lets `--base-ns` override per invocation.
pub const DEFAULT_BASE_NS: &str = "http://aegis.gastown.local/ontology/";

// ── W3C standard namespaces ────────────────────────────────────

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
pub const PROV: &str = "http://www.w3.org/ns/prov#";
pub const SHACL: &str = "http://www.w3.org/ns/shacl#";
pub const OWL: &str = "http://www.w3.org/2002/07/owl#";
pub const SKOS: &str = "http://www.w3.org/2004/02/skos/core#";

// ── Bobbin namespace ──────────────────────────────────────────
//
// `bobbin:` ≡ `aegis:` — decided 2026-08-21 (bobbin×quipu roadmap). The live
// ingest lane (`shapes/code-entities.ttl`, `scripts/ingest-repos.py`, ~15k
// live entities) has always bound `bobbin:` to the aegis base, and the
// `https://bobbin.dev/ontology#` spelling this constant used to carry was
// referenced by nothing outside `reconcile` — two spellings intern as two
// different terms that never join, the exact failure the `quipu:` http/https
// note below warns about. Entity IRIs live under `CODE_BASE`, vocabulary
// under `BOBBIN`/`DEFAULT_BASE_NS`.

pub const BOBBIN: &str = DEFAULT_BASE_NS;

/// Base namespace for code/document ENTITY IRIs (as opposed to vocabulary).
/// Matches `scripts/ingest-repos.py`'s `BASE`: path segments are
/// percent-encoded with `/` escaped, so a relative path is one opaque
/// segment — `http://aegis.gastown.local/code/{repo}/{src%2Flib.rs}`.
///
/// ⚠️ **SUPERSEDED IN PRACTICE — this scheme has ZERO live instances, and
/// saying so here is the whole point of this paragraph** (aegis-6noan,
/// measured 2026-08-23 across ROOT and both named graphs):
///
/// | population | count |
/// |---|---|
/// | any subject under `CODE_BASE` | **0** |
/// | `CodeSymbol` under `{DEFAULT_BASE_NS}code/…::{name}` | **10,425** |
///
/// The live producer of every code entity is **hank** (`hank-src/src/export.rs`),
/// which mints `{DEFAULT_BASE_NS}code/{repo}/{path}::{scope}::{name}` — under the
/// ONTOLOGY base, `::`-separated, with a hierarchical scope chain and no line
/// number. Neither difference is cosmetic: the scope chain exists because
/// without it 42 same-kind symbols silently merged and unioned their call edges
/// (aegis-1q14).
///
/// This constant stays because `reconcile` still parses against it — but a
/// declared-and-unproduced scheme is not a neutral leftover, it is an active
/// trap. bobbin read this constant and `ingest-repos.py`, implemented them
/// faithfully, and would have forked the code graph into two disjoint
/// populations of the same referents had the push shipped. It was caught by
/// hand, twice, because nothing about a declaration says whether anything
/// produces it. If you are about to build against `CODE_BASE`, measure the
/// graph first — and if you are the one who retires it, retire
/// `ingest-repos.py`'s `BASE` in the same change.
pub const CODE_BASE: &str = "http://aegis.gastown.local/code/";

// ── Quipu namespace ───────────────────────────────────────────
// Quipu's own ontology terms (graph-analysis qualifiers it mints itself, as
// opposed to the `aegis:` domain ontology). Matches the schemaSpace advertised
// by the reconciliation endpoint (semweb::reconcile_manifest).

pub const QUIPU: &str = "https://quipu.dev/ontology/";

// ── Graph labels (quipu #65) ───────────────────────────────────
//
// The reserved meta-graph and the three label-axis predicates from
// `docs/design/graph-labels.md`. Built from `QUIPU` deliberately: the design
// doc writes these as `http://quipu.dev/…` while this codebase's namespace has
// always been `https://`, and two spellings would intern as two DIFFERENT
// terms — a graph labelled through one and read through the other would come
// back undeclared, with nothing to see but a silent miss.

/// The reserved meta-graph holding every graph's labels.
///
/// Unlike ROOT (`g = 0`, a constant), this graph's `g` is
/// `intern(META_GRAPH_IRI)` — a runtime rowid. That is why it is seeded in the
/// migration function and never in `INIT_SQL`.
pub const META_GRAPH_IRI: &str = "urn:quipu:graph:meta";

/// `quipu:freshness` — how current a graph's contents are.
pub const QUIPU_FRESHNESS: &str = "https://quipu.dev/ontology/freshness";
/// `quipu:trust` — the graph's trust value (an IRI, ranked by a chain).
pub const QUIPU_TRUST: &str = "https://quipu.dev/ontology/trust";
/// `quipu:policyClass` — an obligation token carried by the graph.
pub const QUIPU_POLICY_CLASS: &str = "https://quipu.dev/ontology/policyClass";
/// `quipu:durability` — declared recoverability of a graph or statement.
pub const QUIPU_DURABILITY: &str = "https://quipu.dev/ontology/durability";
/// `quipu:trustRank` — a trust value's rank within its chain.
pub const QUIPU_TRUST_RANK: &str = "https://quipu.dev/ontology/trustRank";
/// `quipu:inChain` — the chain that ranks a trust value.
pub const QUIPU_IN_CHAIN: &str = "https://quipu.dev/ontology/inChain";
/// `quipu:dataKind` — what sort of data a graph holds (categorical; see
/// [`crate::lattice_kind`]). Conventioned values: `knowledge`, `operational`,
/// `identity`, `archive`.
pub const QUIPU_DATA_KIND: &str = "https://quipu.dev/ontology/dataKind";
/// `quipu:lifecycleState` — a graph's storage lifecycle (`frozen` after a
/// deep-freeze; absent otherwise). Written by freeze/thaw, never by hand.
pub const QUIPU_LIFECYCLE_STATE: &str = "https://quipu.dev/ontology/lifecycleState";
/// `quipu:frozenInto` — the content hash of the pack a frozen graph's rows
/// were relocated into.
pub const QUIPU_FROZEN_INTO: &str = "https://quipu.dev/ontology/frozenInto";
/// `quipu:frozenAt` — when the freeze happened.
pub const QUIPU_FROZEN_AT: &str = "https://quipu.dev/ontology/frozenAt";
/// `quipu:derivedBy` — a statement's derivation method resource.
pub const QUIPU_DERIVED_BY: &str = "https://quipu.dev/ontology/derivedBy";
/// `quipu:derivationSystem` — the system capable of executing a method.
pub const QUIPU_DERIVATION_SYSTEM: &str = "https://quipu.dev/ontology/derivationSystem";
/// `quipu:derivationQuery` — the query/command understood by that system.
pub const QUIPU_DERIVATION_QUERY: &str = "https://quipu.dev/ontology/derivationQuery";
/// `quipu:derivationParams` — canonical JSON parameters for the query.
pub const QUIPU_DERIVATION_PARAMS: &str = "https://quipu.dev/ontology/derivationParams";

/// `quipu:distinctFrom` — this entity is DELIBERATELY not the entity named by
/// the object, even though entity resolution scores them as near-duplicates.
///
/// Asserted by a writer to override a strict-mode resolution refusal for one
/// specific pairing, rather than by disabling `strict_mode` for every entity.
/// It is a durable fact, so the override survives the write that made it: a
/// later re-ingest of the same entity reads it back and stays silent about the
/// pairing it excuses. It excuses exactly the named pairing and nothing else.
pub const QUIPU_DISTINCT_FROM: &str = "https://quipu.dev/ontology/distinctFrom";

/// RDF reified-statement class.
pub const RDF_STATEMENT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#Statement";
/// RDF reified-statement subject predicate.
pub const RDF_SUBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#subject";
/// RDF reified-statement predicate predicate.
pub const RDF_PREDICATE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#predicate";
/// RDF reified-statement object predicate.
pub const RDF_OBJECT: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#object";

// ── Named datasets (quipu #69) ─────────────────────────────────

/// `quipu:Dataset` — the class of a named graph set.
pub const QUIPU_DATASET: &str = "https://quipu.dev/ontology/Dataset";
/// `quipu:includesGraph` — a dataset's membership edge.
pub const QUIPU_INCLUDES_GRAPH: &str = "https://quipu.dev/ontology/includesGraph";

// ── Persistent named forks (quipu-gp5) ─────────────────────────

/// `quipu:Fork` — the class of a persistent named fork's graph.
pub const QUIPU_FORK: &str = "https://quipu.dev/ontology/Fork";
/// `quipu:forkTx` — the parent transaction a fork is pinned to.
pub const QUIPU_FORK_TX: &str = "https://quipu.dev/ontology/forkTx";

// ── Retrieval policy (quipu #80) ───────────────────────────────
//
// A pack RECOMMENDS; the consumer's `[quipu.labels]` config ENFORCES. These
// predicates are read and surfaced, never applied.

/// `quipu:defaultDataset` — the dataset a graph expects to be activated with.
pub const QUIPU_DEFAULT_DATASET: &str = "https://quipu.dev/ontology/defaultDataset";
/// `quipu:recommendsFreshness` — the minimum freshness a producer considers
/// safe for consumers of this layer. Advisory.
pub const QUIPU_RECOMMENDS_FRESHNESS: &str = "https://quipu.dev/ontology/recommendsFreshness";
/// `quipu:recommendsTrust` — the minimum trust value a producer considers safe.
/// Advisory.
pub const QUIPU_RECOMMENDS_TRUST: &str = "https://quipu.dev/ontology/recommendsTrust";

// ── Commonly-used IRIs ─────────────────────────────────────────

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
pub const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

// ── Bobbin property IRIs ──────────────────────────────────────
// Under the aegis base, matching what the live lane emits.

/// `bobbin:imports` — unresolved import edge (target may be literal or ref).
pub const BOBBIN_IMPORTS: &str = "http://aegis.gastown.local/ontology/imports";
/// `bobbin:name` — symbol / entity name.
pub const BOBBIN_NAME: &str = "http://aegis.gastown.local/ontology/name";
/// `bobbin:language` — programming language of a `CodeModule`.
pub const BOBBIN_LANGUAGE: &str = "http://aegis.gastown.local/ontology/language";
/// `bobbin:definedIn` — links a `CodeSymbol` to its parent `CodeModule`.
pub const BOBBIN_DEFINED_IN: &str = "http://aegis.gastown.local/ontology/definedIn";
/// `bobbin:filePath` — file path of a `CodeModule`.
pub const BOBBIN_FILE_PATH: &str = "http://aegis.gastown.local/ontology/filePath";

// ── XSD datatype IRIs ──────────────────────────────────────────

pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
pub const XSD_LONG: &str = "http://www.w3.org/2001/XMLSchema#long";
pub const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#int";
pub const XSD_SHORT: &str = "http://www.w3.org/2001/XMLSchema#short";
pub const XSD_BYTE: &str = "http://www.w3.org/2001/XMLSchema#byte";
pub const XSD_NON_NEGATIVE_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#nonNegativeInteger";
pub const XSD_POSITIVE_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#positiveInteger";
pub const XSD_UNSIGNED_LONG: &str = "http://www.w3.org/2001/XMLSchema#unsignedLong";
pub const XSD_UNSIGNED_INT: &str = "http://www.w3.org/2001/XMLSchema#unsignedInt";
pub const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
pub const XSD_FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";
pub const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
pub const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// `rdf:langString` — the datatype oxrdf reports for language-tagged literals.
pub const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

/// XSD datatypes whose lexical form is a number, for ordering and aggregation.
///
/// Used by `Value::as_f64` so a `Typed` literal that kept its datatype (e.g.
/// `xsd:long`, `xsd:decimal`) still compares and sums numerically instead of
/// having to be collapsed into `Int`/`Float` at parse time (aegis-fmyi).
pub fn is_numeric_datatype(dt: &str) -> bool {
    matches!(
        dt,
        XSD_INTEGER
            | XSD_LONG
            | XSD_INT
            | XSD_SHORT
            | XSD_BYTE
            | XSD_NON_NEGATIVE_INTEGER
            | XSD_POSITIVE_INTEGER
            | XSD_UNSIGNED_LONG
            | XSD_UNSIGNED_INT
            | XSD_DOUBLE
            | XSD_FLOAT
            | XSD_DECIMAL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bobbin_vocabulary_is_the_aegis_base() {
        // Two spellings would intern as two different terms; the live lane's
        // binding is the one true namespace.
        assert_eq!(BOBBIN, DEFAULT_BASE_NS);
        assert!(BOBBIN_IMPORTS.starts_with(DEFAULT_BASE_NS));
        assert!(CODE_BASE.starts_with("http://aegis.gastown.local/"));
        assert_ne!(CODE_BASE, DEFAULT_BASE_NS);
    }
}

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

pub const BOBBIN: &str = "https://bobbin.dev/ontology#";

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

/// `bobbin:imports` — unresolved import edge (target may be literal or ref).
pub const BOBBIN_IMPORTS: &str = "https://bobbin.dev/ontology#imports";
/// `bobbin:name` — symbol / entity name.
pub const BOBBIN_NAME: &str = "https://bobbin.dev/ontology#name";
/// `bobbin:language` — programming language of a `CodeModule`.
pub const BOBBIN_LANGUAGE: &str = "https://bobbin.dev/ontology#language";
/// `bobbin:definedIn` — links a `CodeSymbol` to its parent `CodeModule`.
pub const BOBBIN_DEFINED_IN: &str = "https://bobbin.dev/ontology#definedIn";
/// `bobbin:filePath` — file path of a `CodeModule`.
pub const BOBBIN_FILE_PATH: &str = "https://bobbin.dev/ontology#filePath";

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

// ── Bobbin IRI constructors ───────────────────────────────────

/// Build a `bobbin:code/{repo}/{path}` IRI (`CodeModule`).
pub fn code_module_iri(repo: &str, path: &str) -> String {
    format!("{BOBBIN}code/{repo}/{path}")
}

/// Build a `bobbin:code/{repo}/{path}::{symbol}` IRI (`CodeSymbol`).
pub fn code_symbol_iri(repo: &str, path: &str, symbol: &str) -> String {
    format!("{BOBBIN}code/{repo}/{path}::{symbol}")
}

/// Build a `bobbin:doc/{repo}/{path}` IRI (`Document`).
pub fn document_iri(repo: &str, path: &str) -> String {
    format!("{BOBBIN}doc/{repo}/{path}")
}

/// Build a `bobbin:doc/{repo}/{path}#section-slug` IRI (`Section`).
pub fn section_iri(repo: &str, path: &str, section_slug: &str) -> String {
    format!("{BOBBIN}doc/{repo}/{path}#{section_slug}")
}

/// Build a `bobbin:bundle/{name}` IRI (`Bundle`).
pub fn bundle_iri(name: &str) -> String {
    format!("{BOBBIN}bundle/{name}")
}

// ── Bobbin IRI parsing ────────────────────────────────────────

/// Parsed components of a Bobbin code entity IRI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BobbinIri<'a> {
    /// `bobbin:code/{repo}/{path}`
    CodeModule { repo: &'a str, path: &'a str },
    /// `bobbin:code/{repo}/{path}::{symbol}`
    CodeSymbol {
        repo: &'a str,
        path: &'a str,
        symbol: &'a str,
    },
    /// `bobbin:doc/{repo}/{path}`
    Document { repo: &'a str, path: &'a str },
    /// `bobbin:doc/{repo}/{path}#section-slug`
    Section {
        repo: &'a str,
        path: &'a str,
        section: &'a str,
    },
    /// `bobbin:bundle/{name}`
    Bundle { name: &'a str },
}

/// Parse a full IRI into its Bobbin components, or `None` if it does not match.
pub fn parse_bobbin_iri(iri: &str) -> Option<BobbinIri<'_>> {
    let rest = iri.strip_prefix(BOBBIN)?;

    if let Some(rest) = rest.strip_prefix("bundle/") {
        if rest.is_empty() {
            return None;
        }
        return Some(BobbinIri::Bundle { name: rest });
    }

    if let Some(rest) = rest.strip_prefix("code/") {
        let (repo, path_and_maybe_symbol) = rest.split_once('/')?;
        if repo.is_empty() {
            return None;
        }
        // Check for `::symbol` suffix (split at first `::` — paths never contain `::`)
        if let Some((path, symbol)) = path_and_maybe_symbol.split_once("::") {
            if path.is_empty() || symbol.is_empty() {
                return None;
            }
            return Some(BobbinIri::CodeSymbol { repo, path, symbol });
        }
        if path_and_maybe_symbol.is_empty() {
            return None;
        }
        return Some(BobbinIri::CodeModule {
            repo,
            path: path_and_maybe_symbol,
        });
    }

    if let Some(rest) = rest.strip_prefix("doc/") {
        let (repo, path_and_maybe_section) = rest.split_once('/')?;
        if repo.is_empty() {
            return None;
        }
        // Check for `#section` suffix
        if let Some((path, section)) = path_and_maybe_section.rsplit_once('#') {
            if path.is_empty() || section.is_empty() {
                return None;
            }
            return Some(BobbinIri::Section {
                repo,
                path,
                section,
            });
        }
        if path_and_maybe_section.is_empty() {
            return None;
        }
        return Some(BobbinIri::Document {
            repo,
            path: path_and_maybe_section,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_module_iri() {
        assert_eq!(
            code_module_iri("quipu", "src/namespace.rs"),
            "https://bobbin.dev/ontology#code/quipu/src/namespace.rs"
        );
    }

    #[test]
    fn test_code_symbol_iri() {
        assert_eq!(
            code_symbol_iri("quipu", "src/store.rs", "Store::insert"),
            "https://bobbin.dev/ontology#code/quipu/src/store.rs::Store::insert"
        );
    }

    #[test]
    fn test_document_iri() {
        assert_eq!(
            document_iri("quipu", "docs/architecture.md"),
            "https://bobbin.dev/ontology#doc/quipu/docs/architecture.md"
        );
    }

    #[test]
    fn test_section_iri() {
        assert_eq!(
            section_iri("quipu", "docs/architecture.md", "overview"),
            "https://bobbin.dev/ontology#doc/quipu/docs/architecture.md#overview"
        );
    }

    #[test]
    fn test_bundle_iri() {
        assert_eq!(
            bundle_iri("my-bundle"),
            "https://bobbin.dev/ontology#bundle/my-bundle"
        );
    }

    #[test]
    fn test_parse_code_module() {
        let iri = code_module_iri("quipu", "src/lib.rs");
        assert_eq!(
            parse_bobbin_iri(&iri),
            Some(BobbinIri::CodeModule {
                repo: "quipu",
                path: "src/lib.rs",
            })
        );
    }

    #[test]
    fn test_parse_code_symbol() {
        let iri = code_symbol_iri("quipu", "src/store.rs", "Store::insert");
        assert_eq!(
            parse_bobbin_iri(&iri),
            Some(BobbinIri::CodeSymbol {
                repo: "quipu",
                path: "src/store.rs",
                symbol: "Store::insert",
            })
        );
    }

    #[test]
    fn test_parse_document() {
        let iri = document_iri("quipu", "docs/arch.md");
        assert_eq!(
            parse_bobbin_iri(&iri),
            Some(BobbinIri::Document {
                repo: "quipu",
                path: "docs/arch.md",
            })
        );
    }

    #[test]
    fn test_parse_section() {
        let iri = section_iri("quipu", "docs/arch.md", "overview");
        assert_eq!(
            parse_bobbin_iri(&iri),
            Some(BobbinIri::Section {
                repo: "quipu",
                path: "docs/arch.md",
                section: "overview",
            })
        );
    }

    #[test]
    fn test_parse_bundle() {
        let iri = bundle_iri("my-bundle");
        assert_eq!(
            parse_bobbin_iri(&iri),
            Some(BobbinIri::Bundle { name: "my-bundle" })
        );
    }

    #[test]
    fn test_parse_non_bobbin_iri() {
        assert_eq!(parse_bobbin_iri("http://example.com/foo"), None);
    }

    #[test]
    fn test_parse_empty_segments() {
        // Empty repo
        assert_eq!(parse_bobbin_iri(&format!("{BOBBIN}code//src/lib.rs")), None);
        // Empty path
        assert_eq!(parse_bobbin_iri(&format!("{BOBBIN}code/quipu/")), None);
        // Empty bundle name
        assert_eq!(parse_bobbin_iri(&format!("{BOBBIN}bundle/")), None);
    }

    #[test]
    fn test_roundtrip_all_variants() {
        // Verify constructors and parser are consistent
        let cases: Vec<(String, BobbinIri<'_>)> = vec![
            (
                code_module_iri("r", "a/b.rs"),
                BobbinIri::CodeModule {
                    repo: "r",
                    path: "a/b.rs",
                },
            ),
            (
                code_symbol_iri("r", "a/b.rs", "Foo"),
                BobbinIri::CodeSymbol {
                    repo: "r",
                    path: "a/b.rs",
                    symbol: "Foo",
                },
            ),
            (
                document_iri("r", "d.md"),
                BobbinIri::Document {
                    repo: "r",
                    path: "d.md",
                },
            ),
            (
                section_iri("r", "d.md", "s"),
                BobbinIri::Section {
                    repo: "r",
                    path: "d.md",
                    section: "s",
                },
            ),
            (bundle_iri("b"), BobbinIri::Bundle { name: "b" }),
        ];
        for (iri, expected) in &cases {
            assert_eq!(
                parse_bobbin_iri(iri).as_ref(),
                Some(expected),
                "failed for {iri}"
            );
        }
    }
}

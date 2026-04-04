//! Central namespace constants for RDF/OWL/SHACL URIs.
//!
//! All namespace prefixes and commonly-used IRIs are defined here to avoid
//! hardcoded strings scattered across the codebase.

// ── Project namespace ──────────────────────────────────────────

/// Default base namespace for the Aegis ontology.
/// Override via `QuipuConfig::base_ns`.
pub const DEFAULT_BASE_NS: &str = "http://aegis.gastown.local/ontology/";

// ── W3C standard namespaces ────────────────────────────────────

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
pub const PROV: &str = "http://www.w3.org/ns/prov#";
pub const SHACL: &str = "http://www.w3.org/ns/shacl#";

// ── Bobbin namespace ──────────────────────────────────────────

pub const BOBBIN: &str = "https://bobbin.dev/ontology#";

// ── Commonly-used IRIs ─────────────────────────────────────────

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

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

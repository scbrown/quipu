//! Post-index reconciliation — resolves cross-repo import edges.
//!
//! After Bobbin syncs code entities from any repo, this module runs a
//! reconciliation pass to resolve dangling import references (stored as
//! literal strings) into concrete entity reference edges.
//!
//! The algorithm is idempotent: re-running after any repo reindex
//! produces the same result without duplicating edges.

#[cfg(test)]
mod tests;

use crate::error::Result;
use crate::namespace::{self, BOBBIN_IMPORTS, BOBBIN_LANGUAGE, BOBBIN_NAME};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

// ── Import resolver trait ─────────────────────────────────────

/// A candidate match produced by parsing an import specifier.
#[derive(Debug, Clone)]
pub struct ImportCandidate {
    /// Symbol name to match against `bobbin:name` (e.g. `"HashMap"`).
    pub symbol_name: Option<String>,
    /// Module/path fragment to match against entity IRIs (e.g. `"collections"`).
    pub module_hint: Option<String>,
}

/// Language-specific import specifier parser.
///
/// Implementations know how to decompose a raw import string (e.g.
/// `"std::collections::HashMap"`) into candidate search criteria
/// that the reconciliation loop can match against the knowledge graph.
pub trait ImportResolver: Send + Sync {
    /// Parse an import specifier into one or more candidate interpretations.
    fn parse(&self, specifier: &str) -> Vec<ImportCandidate>;

    /// The language identifier this resolver handles (e.g. `"rust"`).
    fn language(&self) -> &str;
}

// ── Concrete resolvers ────────────────────────────────────────

/// Resolver for Rust `use` paths (e.g. `std::collections::HashMap`).
pub struct RustResolver;

impl ImportResolver for RustResolver {
    fn language(&self) -> &str {
        "rust"
    }

    fn parse(&self, specifier: &str) -> Vec<ImportCandidate> {
        // Strip common Rust path prefixes.
        let path = specifier
            .strip_prefix("crate::")
            .or_else(|| specifier.strip_prefix("self::"))
            .or_else(|| specifier.strip_prefix("super::"))
            .unwrap_or(specifier);

        let segments: Vec<&str> = path.split("::").collect();
        if segments.is_empty() {
            return vec![];
        }

        let symbol = segments.last().copied().unwrap_or_default();
        let module_hint = if segments.len() > 1 {
            Some(segments[..segments.len() - 1].join("/"))
        } else {
            None
        };

        vec![ImportCandidate {
            symbol_name: Some(symbol.to_string()),
            module_hint,
        }]
    }
}

/// Resolver for Python import paths (e.g. `os.path.join`).
pub struct PythonResolver;

impl ImportResolver for PythonResolver {
    fn language(&self) -> &str {
        "python"
    }

    fn parse(&self, specifier: &str) -> Vec<ImportCandidate> {
        let parts: Vec<&str> = specifier.split('.').collect();
        if parts.is_empty() {
            return vec![];
        }

        let symbol = parts.last().copied().unwrap_or_default();
        let module_hint = if parts.len() > 1 {
            Some(parts[..parts.len() - 1].join("/"))
        } else {
            None
        };

        // Python imports might be a module name OR a symbol name.
        // Produce both interpretations.
        let mut candidates = vec![ImportCandidate {
            symbol_name: Some(symbol.to_string()),
            module_hint: module_hint.clone(),
        }];

        // Also try matching the whole thing as a module path.
        if parts.len() > 1 {
            candidates.push(ImportCandidate {
                symbol_name: None,
                module_hint: Some(parts.join("/")),
            });
        }

        candidates
    }
}

/// Resolver for Go import paths (e.g. `github.com/user/repo/pkg`).
pub struct GoResolver;

impl ImportResolver for GoResolver {
    fn language(&self) -> &str {
        "go"
    }

    fn parse(&self, specifier: &str) -> Vec<ImportCandidate> {
        // Go imports are full module paths. The last segment is typically
        // the package name used in code.
        let parts: Vec<&str> = specifier.split('/').collect();
        let pkg_name = parts.last().copied().unwrap_or_default();

        vec![
            // Try matching as a module path.
            ImportCandidate {
                symbol_name: None,
                module_hint: Some(specifier.to_string()),
            },
            // Also try the package name as a symbol.
            ImportCandidate {
                symbol_name: Some(pkg_name.to_string()),
                module_hint: None,
            },
        ]
    }
}

/// Return the default set of resolvers for all supported languages.
pub fn default_resolvers() -> Vec<Box<dyn ImportResolver>> {
    vec![
        Box::new(RustResolver),
        Box::new(PythonResolver),
        Box::new(GoResolver),
    ]
}

// ── Reconciliation result types ───────────────────────────────

/// Outcome for a single import edge.
#[derive(Debug, Clone)]
pub enum ImportResolution {
    /// Exactly one match — edge resolved.
    Resolved {
        /// IRI of the importing entity.
        from_iri: String,
        /// IRI of the resolved target entity.
        to_iri: String,
    },
    /// No match — target repo probably not yet indexed.
    Dangling {
        /// IRI of the importing entity.
        from_iri: String,
        /// The unresolved import specifier.
        specifier: String,
    },
    /// Multiple matches — ambiguous, left unresolved.
    Ambiguous {
        /// IRI of the importing entity.
        from_iri: String,
        /// The unresolved import specifier.
        specifier: String,
        /// IRIs of all candidate targets.
        candidates: Vec<String>,
    },
}

/// Summary of a reconciliation pass.
#[derive(Debug)]
pub struct ReconcileReport {
    /// Number of import edges successfully resolved.
    pub resolved: usize,
    /// Number of imports with no match (target not yet indexed).
    pub dangling: usize,
    /// Number of imports with multiple matches (ambiguous).
    pub ambiguous: usize,
    /// Per-edge resolution details.
    pub details: Vec<ImportResolution>,
}

// ── Core reconciliation ───────────────────────────────────────

/// Run the reconciliation pass over the store.
///
/// Finds all unresolved import edges (where the object is a literal string
/// rather than an entity reference), attempts to resolve each via the
/// provided resolvers, and atomically updates resolved edges.
///
/// Idempotent: already-resolved edges (`Value::Ref`) are skipped.
pub fn reconcile(
    store: &mut Store,
    resolvers: &[Box<dyn ImportResolver>],
    timestamp: &str,
) -> Result<ReconcileReport> {
    let Some(imports_id) = store.lookup(BOBBIN_IMPORTS)? else {
        return Ok(ReconcileReport {
            resolved: 0,
            dangling: 0,
            ambiguous: 0,
            details: vec![],
        });
    };

    let Some(name_id) = store.lookup(BOBBIN_NAME)? else {
        return Ok(ReconcileReport {
            resolved: 0,
            dangling: 0,
            ambiguous: 0,
            details: vec![],
        });
    };

    // Collect unresolved import edges (Str values on the imports predicate).
    let all_facts = store.current_facts()?;
    let unresolved: Vec<_> = all_facts
        .iter()
        .filter(|f| f.attribute == imports_id && matches!(f.value, Value::Str(_)))
        .collect();

    if unresolved.is_empty() {
        return Ok(ReconcileReport {
            resolved: 0,
            dangling: 0,
            ambiguous: 0,
            details: vec![],
        });
    }

    // Build name → entity IRI index for efficient lookups.
    let name_index = build_name_index(store, &all_facts, name_id)?;

    let mut report = ReconcileReport {
        resolved: 0,
        dangling: 0,
        ambiguous: 0,
        details: Vec::with_capacity(unresolved.len()),
    };
    let mut datums = Vec::new();

    for fact in &unresolved {
        let specifier = match &fact.value {
            Value::Str(s) => s.clone(),
            _ => continue,
        };

        let from_iri = store.resolve(fact.entity)?;

        // Determine language from the source entity's module context.
        let language = determine_language(store, &all_facts, fact.entity)?;

        // Find the matching resolver, or fall back to direct name match.
        let candidates: Vec<ImportCandidate> = match &language {
            Some(lang) => {
                let resolver = resolvers.iter().find(|r| r.language() == lang.as_str());
                match resolver {
                    Some(r) => r.parse(&specifier),
                    None => vec![ImportCandidate {
                        symbol_name: Some(specifier.clone()),
                        module_hint: None,
                    }],
                }
            }
            None => vec![ImportCandidate {
                symbol_name: Some(specifier.clone()),
                module_hint: None,
            }],
        };

        // Match candidates against the name index.
        let matches = find_matches(store, &name_index, &candidates)?;

        match matches.len() {
            0 => {
                report.dangling += 1;
                report.details.push(ImportResolution::Dangling {
                    from_iri,
                    specifier,
                });
            }
            1 => {
                let target_iri = &matches[0];
                let target_id = store.intern(target_iri)?;

                // Retract the old literal edge.
                datums.push(Datum {
                    entity: fact.entity,
                    attribute: imports_id,
                    value: Value::Str(specifier.clone()),
                    valid_from: fact.valid_from.clone(),
                    valid_to: None,
                    op: Op::Retract,
                });

                // Assert the resolved reference edge.
                datums.push(Datum {
                    entity: fact.entity,
                    attribute: imports_id,
                    value: Value::Ref(target_id),
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                });

                report.resolved += 1;
                report.details.push(ImportResolution::Resolved {
                    from_iri,
                    to_iri: target_iri.clone(),
                });
            }
            _ => {
                report.ambiguous += 1;
                report.details.push(ImportResolution::Ambiguous {
                    from_iri,
                    specifier,
                    candidates: matches,
                });
            }
        }
    }

    if !datums.is_empty() {
        store.transact(
            &datums,
            timestamp,
            Some("reconcile"),
            Some("reconciliation"),
        )?;
    }

    Ok(report)
}

// ── Internal helpers ──────────────────────────────────────────

/// Index of `bobbin:name` values to entity IRIs for fast candidate matching.
struct NameIndex {
    /// Maps lowercase name to list of (`entity_id`, `entity_iri`).
    entries: std::collections::HashMap<String, Vec<(i64, String)>>,
}

fn build_name_index(
    store: &Store,
    facts: &[crate::types::Fact],
    name_attr_id: i64,
) -> Result<NameIndex> {
    let mut entries: std::collections::HashMap<String, Vec<(i64, String)>> =
        std::collections::HashMap::new();

    for fact in facts {
        if fact.attribute == name_attr_id
            && let Value::Str(ref name) = fact.value
        {
            let iri = store.resolve(fact.entity)?;
            // Only index entities with Bobbin code IRIs.
            if iri.starts_with(namespace::BOBBIN) {
                entries
                    .entry(name.to_lowercase())
                    .or_default()
                    .push((fact.entity, iri));
            }
        }
    }

    Ok(NameIndex { entries })
}

/// Find entity IRIs matching any of the given candidates.
fn find_matches(
    _store: &Store,
    name_index: &NameIndex,
    candidates: &[ImportCandidate],
) -> Result<Vec<String>> {
    let mut results: Vec<String> = Vec::new();

    for candidate in candidates {
        if let Some(symbol) = &candidate.symbol_name {
            let key = symbol.to_lowercase();
            if let Some(entries) = name_index.entries.get(&key) {
                for (_id, iri) in entries {
                    // If there's a module hint, verify the IRI contains it.
                    if let Some(hint) = &candidate.module_hint
                        && !iri_contains_path(iri, hint)
                    {
                        continue;
                    }
                    if !results.contains(iri) {
                        results.push(iri.clone());
                    }
                }
            }
        } else if let Some(hint) = &candidate.module_hint {
            // Module-only match: search for CodeModule IRIs containing the hint.
            for entries in name_index.entries.values() {
                for (_id, iri) in entries {
                    if iri_contains_path(iri, hint) && !results.contains(iri) {
                        results.push(iri.clone());
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Check whether an entity IRI contains the given path fragment.
fn iri_contains_path(iri: &str, path_hint: &str) -> bool {
    // Strip the Bobbin prefix and check if the remaining path contains the hint.
    let Some(rest) = iri.strip_prefix(namespace::BOBBIN) else {
        return false;
    };
    // Normalize separators for comparison.
    let normalized = rest.replace("::", "/");
    let hint_normalized = path_hint.replace("::", "/");
    normalized.contains(&hint_normalized)
}

/// Determine the programming language for an entity by walking up
/// to its parent `CodeModule` via `bobbin:definedIn`.
fn determine_language(
    store: &Store,
    facts: &[crate::types::Fact],
    entity_id: i64,
) -> Result<Option<String>> {
    let defined_in_id = store.lookup(namespace::BOBBIN_DEFINED_IN)?;
    let language_id = store.lookup(BOBBIN_LANGUAGE)?;

    // First try: entity itself has a language property (it's a CodeModule).
    if let Some(lang_id) = language_id {
        for fact in facts {
            if fact.entity == entity_id
                && fact.attribute == lang_id
                && let Value::Str(ref lang) = fact.value
            {
                return Ok(Some(lang.to_lowercase()));
            }
        }
    }

    // Second try: follow definedIn to the parent module, then check its language.
    if let Some(di_id) = defined_in_id {
        for fact in facts {
            if fact.entity == entity_id
                && fact.attribute == di_id
                && let Value::Ref(module_id) = fact.value
                && let Some(lang_id) = language_id
            {
                for mfact in facts {
                    if mfact.entity == module_id
                        && mfact.attribute == lang_id
                        && let Value::Str(ref lang) = mfact.value
                    {
                        return Ok(Some(lang.to_lowercase()));
                    }
                }
            }
        }
    }

    Ok(None)
}

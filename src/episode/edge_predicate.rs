//! Edge relations: the Turtle predicate an episode edge `relation` denotes.

use super::sanitize_iri_local;
use crate::error::Result;
use crate::namespace;

/// Prefixes that `episode_to_turtle` declares, and so may appear verbatim in an
/// edge `relation`. Keep in lockstep with the `@prefix` block in
/// `episode_to_turtle` — a prefix resolved here but not declared there emits
/// Turtle that fails to parse.
pub(super) const KNOWN_PREFIXES: &[(&str, &str)] = &[
    ("rdf", namespace::RDF),
    ("rdfs", namespace::RDFS),
    ("owl", namespace::OWL),
    ("skos", namespace::SKOS),
    ("prov", namespace::PROV),
    ("quipu", namespace::QUIPU),
    ("xsd", namespace::XSD),
    ("sh", namespace::SHACL),
];

/// Resolve an edge `relation` into the Turtle predicate term to emit.
///
/// `/episode` used to force EVERY relation into `aegis:` and then sanitize it,
/// so `rdfs:subClassOf` was stored as `aegis:rdfs_subClassOf` — a predicate that
/// resembles the intended one, matches nothing, and is inert. The response was
/// HTTP 200 with a healthy `count`, so nothing signalled the loss (aegis-kuotp).
/// Measured in the live graph before the fix: `aegis:owl_sameAs` had a real
/// instance that no `owl:sameAs` query could ever reach.
///
/// The policy is: represent the caller's predicate faithfully, or refuse and say
/// which path to use. Never silently rewrite it.
///
/// - `<http://example.org/p>` — a full IRI, emitted verbatim.
/// - `rdfs:subClassOf` — a declared prefix, emitted verbatim.
/// - `foo:bar` — an undeclared prefix, REFUSED (naming `/set`).
/// - `related_to` — no prefix, lands in `aegis:` as before.
/// - `runs on` — would not round-trip through `sanitize_iri_local`, REFUSED.
pub(super) fn resolve_edge_predicate(relation: &str) -> Result<String> {
    let rel = relation.trim();
    if rel.is_empty() {
        return Err(crate::error::Error::InvalidValue(
            "edge relation is empty — every edge requires a relation.".to_string(),
        ));
    }

    // A full IRI, written in angle brackets. Emitted verbatim.
    if let Some(inner) = rel.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        if inner.contains(['<', '>', '"', ' ']) || !inner.contains(':') {
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' is not a usable IRI."
            )));
        }
        return Ok(format!("<{inner}>"));
    }

    // A prefixed name. Resolve against the prefixes this writer declares.
    if let Some((prefix, local)) = rel.split_once(':') {
        let Some((name, _)) = KNOWN_PREFIXES.iter().find(|(p, _)| *p == prefix) else {
            let known: Vec<&str> = KNOWN_PREFIXES.iter().map(|(p, _)| *p).collect();
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' uses undeclared prefix '{prefix}:'. \
                 /episode can emit these prefixes verbatim: {}. For any other \
                 vocabulary, POST the fact to /set, which takes a full predicate \
                 IRI — or write the relation as a full IRI in angle brackets, \
                 e.g. \"<http://example.org/{local}>\" (aegis-kuotp).",
                known.join(", ")
            )));
        };
        if local.is_empty() || sanitize_iri_local(local) != local {
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' has a local name that is not a valid \
                 IRI local part. Use only letters, digits, '-', '_' and '.' \
                 (aegis-kuotp)."
            )));
        }
        return Ok(format!("{name}:{local}"));
    }

    // A bare name: the aegis: domain vocabulary, as before. It must survive
    // sanitization unchanged, or we would be silently renaming it — the exact
    // defect this function exists to stop, one namespace over.
    if sanitize_iri_local(rel) != rel {
        return Err(crate::error::Error::InvalidValue(format!(
            "edge relation '{relation}' cannot be represented as-is — it would be \
             silently rewritten to '{}'. Use only letters, digits, '-', '_' and \
             '.' (e.g. '{}'), or a prefixed/full IRI for a foreign vocabulary \
             (aegis-kuotp).",
            sanitize_iri_local(rel),
            sanitize_iri_local(rel)
        )));
    }
    Ok(format!("aegis:{rel}"))
}

//! Episode node types: validation and the class IRI a `type` denotes.

use super::sanitize_iri_local;
use crate::error::Result;
use crate::namespace;

/// The one type prefix an episode node may carry: the public Quechua vocabulary
/// (aegis-kpy8ec). Any other prefix is refused by `validate_node_type`.
pub(crate) const QUECHUA_TYPE_PREFIX: &str = "quechua:";

/// The class IRI an episode node `type` denotes. A bare name is the legacy
/// domain vocabulary under `base_ns`, byte-for-byte as before; `quechua:Local`
/// is `<QUECHUA>Local`. The type namespace is independent of `base_ns`, so the
/// instance IRIs an episode mints do not move when a writer switches vocabulary.
/// Turtle emission and both vocabulary gates resolve through here, so the class
/// that is checked is the class that is written.
pub(crate) fn node_type_iri(ntype: &str, base_ns: &str) -> String {
    match ntype.strip_prefix(QUECHUA_TYPE_PREFIX) {
        Some(local) => format!("{}{}", namespace::QUECHUA, sanitize_iri_local(local)),
        None => format!("{base_ns}{}", sanitize_iri_local(ntype)),
    }
}

/// Validate a node `type`, refusing anything that would be silently rewritten.
///
/// `type` is a STRING, and a comma-separated one used to mint a single junk class:
/// `"Feature, Concept"` became `aegis:Feature__Concept` — one class, not two —
/// behind HTTP 200 with a healthy `count`. The node was in the store, correctly
/// described and edged, and **absent from `?s a Feature`**, the query anyone
/// actually runs (aegis-vngta).
///
/// It catches careful people specifically: `/search` renders a multi-typed node as
/// `type: Bead, Issue`, so the documented way to discover an existing node's typing
/// hands back a string that looks like valid input. Searching first — the rule that
/// exists to prevent duplicate nodes — is what fed the mistake.
///
/// REFUSE rather than split, deliberately. Splitting would be a lenient parser
/// guessing intent, and it would fork semantics from the crew-side guard in
/// `graph-extract` (aegis-vngta, muldoon), which already refuses this input and
/// documents `/search` output as display-only. Two layers must not disagree about
/// whether the same request is legal.
pub(super) fn validate_node_type(node_name: &str, ntype: &str) -> Result<()> {
    let t = ntype.trim();
    if let Some(local) = t.strip_prefix(QUECHUA_TYPE_PREFIX) {
        if local.is_empty() || sanitize_iri_local(local) != local {
            return Err(crate::error::Error::InvalidValue(format!(
                "node '{node_name}' has type '{ntype}': a '{QUECHUA_TYPE_PREFIX}' type needs a \
                 local name of only letters, digits, '-', '_' and '.' (aegis-kpy8ec)."
            )));
        }
        return Ok(());
    }
    if t.contains(',') {
        let split: Vec<&str> = t
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        return Err(crate::error::Error::InvalidValue(format!(
            "node '{node_name}' has a comma-separated type '{ntype}'. `type` is a \
             single class, and this would mint ONE junk class \
             'aegis:{}' that no `?s a <type>` query can reach. For multiple types, \
             send ONE ENTRY PER TYPE repeating the same node name — e.g. {} — which \
             resolves to one entity carrying both types. Note that `/search` renders \
             types as '{}' for DISPLAY only; that format is not valid input \
             (aegis-vngta).",
            sanitize_iri_local(t),
            split
                .iter()
                .map(|s| format!("{{\"name\":\"{node_name}\",\"type\":\"{s}\"}}"))
                .collect::<Vec<_>>()
                .join(", "),
            split.join(", "),
        )));
    }
    if sanitize_iri_local(t) != t {
        return Err(crate::error::Error::InvalidValue(format!(
            "node '{node_name}' has type '{ntype}', which cannot be represented as-is \
             — it would be silently rewritten to '{}'. Use only letters, digits, '-', \
             '_' and '.' (aegis-vngta).",
            sanitize_iri_local(t)
        )));
    }
    Ok(())
}

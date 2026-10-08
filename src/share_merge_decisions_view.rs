//! What a person needs to read a decisions file (aegis-yavo9c item 6):
//! human labels, the kind of conflict and its rule in words, and a strict
//! check that a file edited elsewhere carries no field `apply` would drop.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::Term;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::share_merge::{DecisionRecord, Graph};

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Why a slot could not auto-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Ours and theirs together hold more values than `sh:maxCount`.
    MaxCountExceeded,
    /// On a `sh:maxCount 1` slot one side deleted the value the other replaced.
    DeleteReplace,
}

/// The kind of `record`'s conflict, as `merge_graphs` decided it.
#[must_use]
pub fn kind(record: &DecisionRecord) -> ConflictKind {
    let one_side_deleted = record.ours.is_empty() != record.theirs.is_empty();
    if record.max_count == 1 && !record.base.is_empty() && one_side_deleted {
        ConflictKind::DeleteReplace
    } else {
        ConflictKind::MaxCountExceeded
    }
}

/// The rule that `record` broke, in words, naming the predicate by label.
#[must_use]
pub fn rule(record: &DecisionRecord, labels: &BTreeMap<String, String>) -> String {
    let predicate = labels
        .get(&record.predicate)
        .cloned()
        .unwrap_or_else(|| record.predicate.clone());
    match kind(record) {
        ConflictKind::DeleteReplace => format!(
            "sh:maxCount 1 on {predicate}: one side deleted the value the other side replaced"
        ),
        ConflictKind::MaxCountExceeded => {
            let together: BTreeSet<&String> =
                record.ours.iter().chain(record.theirs.iter()).collect();
            format!(
                "sh:maxCount {} on {predicate}: ours and theirs together hold {} values",
                record.max_count,
                together.len()
            )
        }
    }
}

/// Every IRI the rows mention, as plain IRI text.
fn iris(records: &[&DecisionRecord]) -> BTreeSet<String> {
    let strip = |t: &str| {
        t.strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .map(str::to_owned)
    };
    let mut out = BTreeSet::new();
    for r in records {
        out.insert(r.predicate.clone());
        out.extend(strip(&r.subject));
        for v in r.base.iter().chain(&r.ours).chain(&r.theirs) {
            out.extend(strip(v));
        }
    }
    out
}

/// One `rdfs:label` per IRI the rows mention, from any of the graphs:
/// untagged first, then `@en`, then the lexically first. IRIs without a
/// label are omitted.
#[must_use]
pub fn labels(records: &[&DecisionRecord], graphs: &[&Graph]) -> BTreeMap<String, String> {
    let wanted = iris(records);
    let mut found: BTreeMap<String, Vec<(u8, String)>> = BTreeMap::new();
    for graph in graphs {
        for t in *graph {
            if t.predicate.as_str() != RDFS_LABEL {
                continue;
            }
            let oxrdf::NamedOrBlankNode::NamedNode(s) = &t.subject else {
                continue;
            };
            if !wanted.contains(s.as_str()) {
                continue;
            }
            if let Term::Literal(l) = &t.object {
                let rank = match l.language() {
                    None => 0,
                    Some("en") => 1,
                    Some(_) => 2,
                };
                found
                    .entry(s.as_str().to_owned())
                    .or_default()
                    .push((rank, l.value().to_owned()));
            }
        }
    }
    found
        .into_iter()
        .filter_map(|(iri, mut c)| {
            c.sort();
            c.into_iter().next().map(|(_, label)| (iri, label))
        })
        .collect()
}

const FILE_FIELDS: &[&str] = &[
    "schema",
    "incoming_share",
    "base_share",
    "local_graph_hash",
    "incoming_graph_hash",
    "labels",
    "rows",
];
const ROW_FIELDS: &[&str] = &[
    "id",
    "subject",
    "predicate",
    "max_count",
    "base",
    "ours",
    "theirs",
    "provenance",
    "proposal",
    "decision",
    "kind",
    "rule",
    "decided_by",
    "decided_at",
];

/// Refuse a field the schema does not define. A tool that adds one would
/// otherwise have it silently dropped by `apply` while the file's hash still
/// covers it, so the record would claim what it never kept.
///
/// # Errors
/// [`Error::InvalidValue`] naming the first unknown field.
pub fn check_fields(bytes: &[u8]) -> Result<()> {
    let value: JsonValue = serde_json::from_slice(bytes)
        .map_err(|e| Error::InvalidValue(format!("decisions file is not JSON: {e}")))?;
    let unknown = |where_: &str, object: &serde_json::Map<String, JsonValue>, allowed: &[&str]| {
        object
            .keys()
            .find(|k| !allowed.contains(&k.as_str()))
            .map(|k| {
                Error::InvalidValue(format!(
                    "unknown field '{k}' {where_}: merge-decisions/v1 does not define it, and apply would drop it. Nothing was written"
                ))
            })
    };
    let Some(file) = value.as_object() else {
        return Err(Error::InvalidValue(
            "decisions file is not an object".into(),
        ));
    };
    if let Some(e) = unknown("in the file", file, FILE_FIELDS) {
        return Err(e);
    }
    for row in file
        .get("rows")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let Some(object) = row.as_object() else {
            return Err(Error::InvalidValue("a row is not an object".into()));
        };
        let id = object.get("id").and_then(JsonValue::as_str).unwrap_or("?");
        if let Some(e) = unknown(&format!("in row {id}"), object, ROW_FIELDS) {
            return Err(e);
        }
    }
    Ok(())
}

//! Last-write-wins node descriptions with lossless episode provenance.

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};
use std::collections::{HashMap, HashSet};

use super::{Episode, node_iri};

pub(super) fn current_content_hash(
    store: &Store,
    ep_iri: &str,
    base_ns: &str,
    graph: Option<&str>,
) -> Result<Option<String>> {
    let pattern = in_graph(&format!("<{ep_iri}> <{base_ns}contentHash> ?h"), graph);
    let query = format!("SELECT ?h WHERE {{ {pattern} }} LIMIT 1");
    let result = crate::sparql::query(store, &query)?;
    Ok(result.rows().first().and_then(|row| match row.get("h") {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }))
}

/// Scope a pattern to the episode's named graph; `None` is ROOT, as before.
///
/// An episode written into a named graph keeps its activity (contentHash,
/// generatedAtTime, provenance) in THAT graph. An unscoped lookup reads ROOT
/// only, so it never found the hash: every byte-identical re-post was a full
/// write reported `created`, and each left another `generatedAtTime` behind
/// (aegis-z1i5on; 160,247 values on 25,213 activities in one plane).
fn in_graph(pattern: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("GRAPH <{g}> {{ {pattern} }}"),
        None => pattern.to_string(),
    }
}

/// Is a content-hash match really "it is already there"? (aegis-7oswq4)
///
/// The hash lives on the episode ACTIVITY node and survives the retraction of
/// the entities that episode generated, so a match alone says "this content was
/// ingested once", not "this content is in the store". A re-post after a
/// cleanup therefore short-circuited to `unchanged` with nothing in the graph —
/// and `unchanged` is the signal the crew rulebook documents as "it was already
/// there", the gate for labelling a source bead ingested. Measured 2026-09-12:
/// 200 retracted entities, re-post returned `outcome: unchanged, count: 0,
/// tx_id: 0`, and a control-gated read-back found 0.
///
/// So a hash match must ALSO find the generated entities still present. The
/// test is strict — at least as many distinct entities as the episode declares,
/// not merely one — because a PARTIAL retraction is not "already there" either.
/// Re-writing is idempotent, so falling through to a real write is the safe
/// branch whenever presence is in doubt.
///
/// The presence query is object-bound, so `idx_vaet (v, a, e, …)` serves it,
/// and it runs only once the hash has already matched.
pub(super) fn is_unchanged(
    store: &Store,
    ep_iri: &str,
    base_ns: &str,
    graph: Option<&str>,
    episode: &Episode,
    existing_hash: &Option<String>,
    new_hash: &str,
) -> Result<bool> {
    if existing_hash.as_deref() != Some(new_hash) {
        return Ok(false);
    }
    let expected: HashSet<String> = episode
        .nodes
        .iter()
        .map(|n| node_iri(&n.name, base_ns))
        .collect();
    if expected.is_empty() {
        return Ok(true);
    }
    let pattern = in_graph(
        &format!("?s <{}wasGeneratedBy> <{ep_iri}>", namespace::PROV),
        graph,
    );
    let query = format!("SELECT ?s WHERE {{ {pattern} }}");
    Ok(crate::sparql::query(store, &query)?.rows().len() >= expected.len())
}

/// Parse and commit an ordinary episode after reconciling description revisions.
pub(super) fn ingest_reconciled(
    store: &mut Store,
    episode: &Episode,
    turtle: &str,
    timestamp: &str,
    base_ns: &str,
    actor: Option<&str>,
    graph: i64,
) -> Result<(i64, usize)> {
    let mut datums = crate::rdf::parse_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        timestamp,
    )?;
    reconcile_node_descriptions(store, episode, base_ns, graph, &mut datums)?;
    keep_existing_node_labels(store, graph, &mut datums)?;
    let count = datums.len();
    let source = format!("episode:{}", episode.name);
    let tx_id = store.transact_to_graph(&datums, timestamp, actor, Some(&source), graph)?;
    Ok((tx_id, count))
}

/// A node that already has a label keeps it (aegis-2mx55r).
///
/// `/episode` names a node and asserts that name as its `rdfs:label`. Naming an
/// existing node by its IRI slug (`bead_reference_pattern` for the node first
/// written as `bead reference pattern`) reaches the same IRI, so the slug became
/// a SECOND label. A policy node with two labels made the share producer refuse
/// "conflicting policy definitions". Drop the new label when the entity already
/// carries a different active one in this graph; a fresh node, or one reused by
/// its existing label, is unaffected.
pub(super) fn keep_existing_node_labels(
    store: &mut Store,
    graph: i64,
    datums: &mut Vec<Datum>,
) -> Result<()> {
    let label = store.intern(&format!("{}label", namespace::RDFS))?;
    let mut labelled: Vec<i64> = datums
        .iter()
        .filter(|d| d.attribute == label && d.op == Op::Assert)
        .map(|d| d.entity)
        .collect();
    labelled.sort_unstable();
    labelled.dedup();

    let mut existing: HashMap<i64, Vec<Value>> = HashMap::new();
    for entity in labelled {
        let current: Vec<Value> = store
            .entity_history_in_graph(entity, graph)?
            .into_iter()
            .filter(|f| f.attribute == label && f.op == Op::Assert && f.valid_to.is_none())
            .map(|f| f.value)
            .collect();
        if !current.is_empty() {
            existing.insert(entity, current);
        }
    }
    datums.retain(|d| {
        d.attribute != label
            || d.op != Op::Assert
            || existing
                .get(&d.entity)
                .is_none_or(|current| current.contains(&d.value))
    });
    Ok(())
}

/// Reconcile explicitly revised node descriptions into the pending episode tx.
///
/// A current entity carries one `rdfs:comment`. When a later episode supplies a
/// different description, move every superseded text to the episode whose
/// `prov:wasGeneratedBy` assertion shared its original tx, then retract it from
/// the entity. Nothing is silently discarded; unattributable history refuses
/// the whole write before the transaction opens.
pub(super) fn reconcile_node_descriptions(
    store: &mut Store,
    episode: &Episode,
    base_ns: &str,
    graph: i64,
    datums: &mut Vec<Datum>,
) -> Result<()> {
    let comment_iri = format!("{}comment", namespace::RDFS);
    let comment = store.intern(&comment_iri)?;
    let generated_by = store.intern(&format!("{}wasGeneratedBy", namespace::PROV))?;
    let mut reconciled = HashSet::new();

    for node in &episode.nodes {
        let Some(new_text) = node.description.as_deref() else {
            continue;
        };
        let Some(entity) = store.lookup(&node_iri(&node.name, base_ns))? else {
            continue;
        };
        if !reconciled.insert((entity, new_text)) {
            continue;
        }
        let history = store.entity_history_in_graph(entity, graph)?;
        let superseded: Vec<_> = history
            .iter()
            .filter(|fact| {
                fact.attribute == comment
                    && fact.op == Op::Assert
                    && fact.valid_to.is_none()
                    && matches!(&fact.value, Value::Str(old) if old != new_text)
            })
            .collect();

        for old in superseded {
            let episode_entity = history
                .iter()
                .find_map(|fact| {
                    if fact.tx == old.tx && fact.attribute == generated_by && fact.op == Op::Assert
                    {
                        match fact.value {
                            Value::Ref(id) => Some(id),
                            _ => None,
                        }
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    Error::InvalidValue(format!(
                        "cannot revise description for '{}': the current comment from tx {} has \
                         no same-transaction prov:wasGeneratedBy attribution; refusing to discard \
                         provenance",
                        node.name, old.tx
                    ))
                })?;

            datums.push(Datum {
                entity,
                attribute: comment,
                value: old.value.clone(),
                valid_from: old.valid_from.clone(),
                valid_to: None,
                op: Op::Retract,
            });
            datums.push(Datum {
                entity: episode_entity,
                attribute: comment,
                value: old.value.clone(),
                valid_from: old.valid_from.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
    }
    Ok(())
}

//! Last-write-wins node descriptions with lossless episode provenance.

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::{Episode, node_iri};

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

    for node in &episode.nodes {
        let Some(new_text) = node.description.as_deref() else {
            continue;
        };
        let Some(entity) = store.lookup(&node_iri(&node.name, base_ns))? else {
            continue;
        };
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

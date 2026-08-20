//! Shared read helpers over the golden-path vocabulary.

use crate::error::{Error, Result};
use crate::namespace::RDF_TYPE;
use crate::store::Store;
use crate::types::Value;

use super::PathVocab;
use super::grammar::StepSig;

/// A trajectory step, ordered.
#[derive(Debug, Clone)]
pub(crate) struct StepRef {
    pub id: i64,
    pub iri: String,
    pub order: Option<i64>,
}

/// The step entities of a trajectory, sorted by `stepOrder` (unordered steps
/// last, then by IRI so the result is deterministic).
pub(crate) fn steps_of(store: &Store, vocab: &PathVocab, traj_id: i64) -> Result<Vec<StepRef>> {
    let Some(step_of) = store.lookup(&vocab.step_of)? else {
        return Ok(Vec::new());
    };
    let mut steps = Vec::new();
    for fact in store.current_facts()? {
        if fact.attribute == step_of && fact.value == Value::Ref(traj_id) {
            let iri = store.resolve(fact.entity)?;
            let order = int_value(store, fact.entity, &vocab.step_order)?;
            steps.push(StepRef {
                id: fact.entity,
                iri,
                order,
            });
        }
    }
    steps.sort_by(|a, b| match (a.order, b.order) {
        (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.iri.cmp(&b.iri)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.iri.cmp(&b.iri),
    });
    Ok(steps)
}

/// First string value of `attr_iri` on `entity`, if any.
pub(crate) fn str_value(store: &Store, entity: i64, attr_iri: &str) -> Result<Option<String>> {
    let Some(attr) = store.lookup(attr_iri)? else {
        return Ok(None);
    };
    for fact in store.entity_facts(entity)? {
        if fact.attribute == attr
            && let Value::Str(s) = &fact.value
        {
            return Ok(Some(s.clone()));
        }
    }
    Ok(None)
}

/// First integer value of `attr_iri` on `entity`, if any. A string that
/// parses as an integer counts — episode writes carry numbers as strings.
pub(crate) fn int_value(store: &Store, entity: i64, attr_iri: &str) -> Result<Option<i64>> {
    let Some(attr) = store.lookup(attr_iri)? else {
        return Ok(None);
    };
    for fact in store.entity_facts(entity)? {
        if fact.attribute == attr {
            match &fact.value {
                Value::Int(i) => return Ok(Some(*i)),
                Value::Str(s) => {
                    if let Ok(i) = s.parse::<i64>() {
                        return Ok(Some(i));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(None)
}

/// All entity ids referenced by `entity` via `attr_iri`.
pub(crate) fn ref_values(store: &Store, entity: i64, attr_iri: &str) -> Result<Vec<i64>> {
    let Some(attr) = store.lookup(attr_iri)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for fact in store.entity_facts(entity)? {
        if fact.attribute == attr
            && let Value::Ref(id) = fact.value
        {
            out.push(id);
        }
    }
    Ok(out)
}

/// Whether `entity` carries any value for `attr_iri`.
pub(crate) fn has_value(store: &Store, entity: i64, attr_iri: &str) -> Result<bool> {
    let Some(attr) = store.lookup(attr_iri)? else {
        return Ok(false);
    };
    Ok(store
        .entity_facts(entity)?
        .iter()
        .any(|f| f.attribute == attr))
}

/// The v1 signature of a step, or `None` when the step records no
/// `actionKind` — unevaluable, per the grammar: missing data, not misconduct.
pub(crate) fn step_sig(store: &Store, vocab: &PathVocab, step: i64) -> Result<Option<StepSig>> {
    let Some(action_kind) = str_value(store, step, &vocab.action_kind)? else {
        return Ok(None);
    };
    let target_class = target_class(store, vocab, step)?;
    Ok(Some(StepSig {
        action_kind,
        target_class,
    }))
}

/// The `targetClass` half of a v1 signature (see
/// `docs/design/conformance-grammar.md`): `"none"` without a target,
/// `"literal"` for a literal target, an IRI target's lexicographically
/// smallest `rdf:type` IRI, `"untyped"` for an IRI target with no type.
fn target_class(store: &Store, vocab: &PathVocab, step: i64) -> Result<String> {
    let Some(attr) = store.lookup(&vocab.action_target)? else {
        return Ok("none".to_string());
    };
    for fact in store.entity_facts(step)? {
        if fact.attribute != attr {
            continue;
        }
        match &fact.value {
            Value::Ref(target) => {
                let mut types: Vec<String> = ref_values(store, *target, RDF_TYPE)?
                    .into_iter()
                    .map(|t| store.resolve(t))
                    .collect::<Result<_>>()?;
                types.sort();
                return Ok(types
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "untyped".to_string()));
            }
            _ => return Ok("literal".to_string()),
        }
    }
    Ok("none".to_string())
}

/// Resolve a trajectory IRI to its entity id, erroring in the impact style.
pub(crate) fn require_entity(store: &Store, iri: &str) -> Result<i64> {
    store
        .lookup(iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {iri}")))
}

/// The falsifier-gated verifications produced by a trajectory's steps:
/// `(verification id, verification IRI)` for every `verifiedBy` target that
/// carries a `falsifier`. An unfalsifiable check never appears here.
pub(crate) fn admissible_verifications(
    store: &Store,
    vocab: &PathVocab,
    steps: &[StepRef],
) -> Result<Vec<(i64, String)>> {
    let mut out = Vec::new();
    for step in steps {
        for v in ref_values(store, step.id, &vocab.verified_by)? {
            if has_value(store, v, &vocab.falsifier)? && !out.iter().any(|(id, _)| *id == v) {
                out.push((v, store.resolve(v)?));
            }
        }
    }
    Ok(out)
}

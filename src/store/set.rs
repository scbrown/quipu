//! Atomic single-predicate replacement.

use super::{Datum, Store, ops::retraction_datums};
use crate::{
    error::{Error, Result},
    types::{Fact, Op, Value},
};

pub(super) fn set_triple(
    store: &mut Store,
    entity: i64,
    predicate: i64,
    value: Value,
    timestamp: &str,
    actor: Option<&str>,
    explicit_str: bool,
) -> Result<(i64, usize, usize)> {
    let current: Vec<Fact> = store
        .entity_facts(entity)?
        .into_iter()
        .filter(|f| f.attribute == predicate)
        .collect();

    if let Value::Str(s) = &value {
        let holds_ref = current.iter().any(|f| matches!(f.value, Value::Ref(_)));
        let holds_str = current.iter().any(|f| matches!(f.value, Value::Str(_)));
        let looks_like_iri = s.contains("://") && !s.chars().any(char::is_whitespace);
        if !explicit_str && ((holds_ref && !holds_str) || (current.is_empty() && looks_like_iri)) {
            let pred_iri = store.resolve(predicate)?;
            return Err(Error::InvalidValue(format!(
                "set refused: object \"{s}\" is a string literal, but <{pred_iri}> \
                 takes an IRI reference. Pass the object as {{\"iri\": \"{s}\"}} to \
                 set an edge, or as {{\"str\": \"{s}\"}} to state that a literal is \
                 intended — a bare IRI-shaped string here is almost always a mis-shaped \
                 edge that no graph traversal can follow."
            )));
        }
    }

    let already_present = current.iter().any(|f| f.value == value);
    let to_retract: Vec<Fact> = current.into_iter().filter(|f| f.value != value).collect();
    if to_retract.is_empty() && already_present {
        return Ok((0, 0, 0));
    }

    let mut datums = retraction_datums(&to_retract);
    let retracted = datums.len();
    let asserted = usize::from(!already_present);
    if !already_present {
        datums.push(Datum {
            entity,
            attribute: predicate,
            value,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    let tx_id = store.transact(&datums, timestamp, actor, Some("set"))?;
    Ok((tx_id, retracted, asserted))
}

//! Atomic single-predicate replacement.

use super::{Datum, Store, ops::retraction_datums};
use crate::{
    error::{Error, Result},
    types::{Fact, Op, Value},
};

// aegis-byn4fn: the eighth argument is the transaction source. It is the
// handle by which these facts can ever be retracted, so it belongs on the
// write path itself; bundling the parameters into a struct to satisfy the
// lint would hide it from every call site, which is how it came to be
// missing in the first place.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_triple(
    store: &mut Store,
    entity: i64,
    predicate: i64,
    value: Value,
    timestamp: &str,
    actor: Option<&str>,
    explicit_str: bool,
    source: Option<&str>,
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
    // aegis-byn4fn: the source was the CONSTANT "set" — one key shared by every
    // correction the fleet has ever made (26,008 transactions when measured), so
    // a correction named no author and the retraction handle meant "every /set
    // ever run". The caller's key wins; the fallback is per-actor.
    let tag = crate::store::source_tag::resolve("set", actor, source);
    let tx_id = store.transact(&datums, timestamp, actor, Some(&tag))?;
    Ok((tx_id, retracted, asserted))
}

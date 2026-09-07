//! Triple-level retraction, split out of `ops.rs`.
//!
//! MOVED VERBATIM (aegis-byn4fn): the function needed one more parameter — a
//! transaction source — and `ops.rs` sat exactly at its grandfathered
//! file-size ceiling, so growing it by even one code line fails the build.
//! The same split `set.rs` already is. The body below is the block that was
//! in `ops.rs`, de-indented into an `impl Store` block; the tests prove
//! the move rather than the argument doing so.

use super::{Store, ops::retraction_datums};
use crate::{
    error::{Error, Result},
    types::{Fact, Value},
};

impl Store {
    /// Retract current facts for an entity, narrowed by predicate and/or value.
    ///
    /// With all three of entity, predicate and value supplied this is
    /// TRIPLE-LEVEL retraction: exactly one `(e, a, v)` statement is closed
    /// (aegis-arup ask 1). Episode granularity was previously the finest handle
    /// available, so removing two stray edges meant retracting the whole episode
    /// — 33 statements for a 2-statement target, a 16x blast radius, and the
    /// rebuild that follows is what put node identity at risk in the first
    /// place. Precision here removes the reason to reach for the blunt tool.
    ///
    /// Returns `(tx_id, count)`; `(0, 0)` when nothing matched.
    // aegis-byn4fn: the eighth argument is the transaction source. It is the
    // handle by which these facts can ever be retracted, so it belongs on the
    // write path itself; bundling the parameters into a struct to satisfy the
    // lint would hide it from every call site, which is how it came to be
    // missing in the first place.
    #[allow(clippy::too_many_arguments)]
    pub fn retract_triples(
        &mut self,
        entity: i64,
        predicate: Option<i64>,
        value: Option<&Value>,
        timestamp: &str,
        actor: Option<&str>,
        allow_orphan: bool,
        source: Option<&str>,
    ) -> Result<(i64, usize)> {
        let facts: Vec<Fact> = store
            .entity_facts(entity)?
            .into_iter()
            .filter(|f| predicate.is_none_or(|p| f.attribute == p))
            .filter(|f| value.is_none_or(|v| &f.value == v))
            .collect();

        if facts.is_empty() {
            // A value was given but matched nothing. Distinguish an idempotent
            // no-op (the triple is genuinely absent) from the SILENT FOOTGUN
            // that froze re-parenting: a bare-string object for an IRI-valued
            // predicate. The write API turns a bare `"kprobe-b"` into
            // `Value::Str`, which can NEVER equal the stored `Value::Ref(kprobe-b)`,
            // so the edge survives and the call reports success with
            // `retracted: 0` — a refusal shaped exactly like a success. `0` here
            // means BOTH "nothing was there" and "you addressed it wrongly", which
            // are opposite facts (one a clean no-op, one your bug) the caller
            // cannot tell apart.
            //
            // Error on the two cases that are unambiguously the shape mistake, and
            // ONLY those, so genuine idempotent retraction (a correctly-shaped
            // value that is simply absent) still falls through to `Ok((0, 0))`:
            //   1. the predicate on this entity holds ONLY `Ref` objects — a `Str`
            //      could never have matched, so it is always a caller mistake;
            //   2. the predicate has NO current fact on this entity to compare
            //      against, but the bare string itself PARSES as an IRI (has a
            //      `scheme://`) — almost certainly a mis-shaped edge retract, e.g.
            //      `rdf:type` whose value was already retracted once. A value that
            //      is a plain literal (no scheme) stays a quiet no-op.
            // A predicate that stores string literals (holds a `Str`) is left
            // alone: a bare string is the RIGHT shape there, so absence is a real
            // idempotent no-op, not a mistake.
            if let (Some(Value::Str(s)), Some(p)) = (value, predicate) {
                let on_pred: Vec<Value> = store
                    .entity_facts(entity)?
                    .into_iter()
                    .filter(|f| f.attribute == p)
                    .map(|f| f.value)
                    .collect();
                let holds_ref = on_pred.iter().any(|v| matches!(v, Value::Ref(_)));
                let holds_str = on_pred.iter().any(|v| matches!(v, Value::Str(_)));
                // A bare token with a `scheme://` and no whitespace is an IRI a
                // caller forgot to wrap, not a literal.
                let looks_like_iri = s.contains("://") && !s.chars().any(char::is_whitespace);
                let ref_only = holds_ref && !holds_str;
                let absent_but_iri_shaped = on_pred.is_empty() && looks_like_iri;
                if ref_only || absent_but_iri_shaped {
                    let pred_iri = self.resolve(p)?;
                    return Err(Error::InvalidValue(format!(
                        "retract matched nothing: object \"{s}\" is a string literal, but \
                         <{pred_iri}> takes an IRI reference. Pass the object as \
                         {{\"iri\": \"{s}\"}} to retract the edge — a bare string is matched \
                         as a literal and can never equal a stored reference, so it silently \
                         retracts nothing."
                    )));
                }
            }
            return Ok((0, 0));
        }

        // Refuse to strip an entity's LAST rdf:type while it keeps other facts —
        // that is a HALF-GHOST: label, comments and edges survive, so the node
        // stays visible to label scans and disappears from every `?s a ?t`
        // query. /episode/retract has had an orphan policy since aegis-arup;
        // /retract is the sharper instrument and had none, so the blunt tool was
        // guarded and the precise one was not. A real node (goldblum-repo) was
        // left typeless this way while the call reported success (aegis-a0ne).
        //
        // Only fires when the entity SURVIVES: retracting an entity whole
        // (predicate = None) removes identity and references together, which
        // leaves no ghost and stays allowed.
        if !allow_orphan {
            let type_id = self.intern(crate::namespace::RDF_TYPE)?;
            let all = self.entity_facts(entity)?;
            let types_before = all.iter().filter(|f| f.attribute == type_id).count();
            let types_going = facts.iter().filter(|f| f.attribute == type_id).count();
            let survivors = all.len() - facts.len();
            if types_before > 0 && types_going == types_before && survivors > 0 {
                return Err(Error::InvalidValue(format!(
                    "refusing to retract entity {entity}'s last rdf:type while {survivors} other \
                     fact(s) survive — this would leave a typeless node that label scans still \
                     find and type queries cannot. Retract the whole entity, assert the \
                     replacement type first, or pass allow_orphan to override deliberately."
                )));
            }
        }

        let datums = retraction_datums(&facts);
        let count = datums.len();
        // aegis-byn4fn: was the CONSTANT "retract" (36,409 transactions when
        // measured) — the most common source in the store, and attributable to
        // nobody. Caller's key wins; the fallback names the actor.
        let tag = super::source_tag::resolve("retract", actor, source);
        let tx_id = self.transact(&datums, timestamp, actor, Some(&tag))?;
        Ok((tx_id, count))
    }

    /// Retract all current facts for an entity (or just those matching a predicate).
    /// Returns `(tx_id, count)`.
    pub fn retract_entity(
        &mut self,
        entity: i64,
        predicate: Option<i64>,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<(i64, usize)> {
        self.retract_triples(entity, predicate, None, timestamp, actor, false, None)
    }
}

//! Agent-transition signature verification at the write gate (quipu-8cc).
//!
//! Shuttle signs every workflow transition with the performing agent's ed25519
//! key over the canonical message
//!
//! ```text
//! shuttle-transition-v1|{run_iri}|{step}|{from}|{to}|{at}|{agent}
//! ```
//!
//! and exports it as an ordinary `aegis:signature` fact on the
//! `aegis:TransitionEvent` — re-derivable from the exported facts alone, so
//! any consumer can re-check it from the graph (`shuttle verify`, the
//! `shuttle-unverified-transitions` stored-query pattern). Consumer-side
//! checking makes a forgery *detectable*; this module makes it *refusable*: a
//! write that lands a `TransitionEvent` whose signature is missing, whose
//! signer is unregistered, or whose signature does not verify is rejected
//! before it commits, the same posture the escalation router takes for
//! decisions (quipu-5s5's `decision-v1` scheme — the sibling pattern, and the
//! same `aegis:VerifierRegistration` root of trust).
//!
//! ## The message format is shuttle's, verbatim
//!
//! [`transition_message`] mirrors `shuttle/signing.py::transition_message`
//! field for field, and the field *derivations* mirror `shuttle verify`: the
//! run is the `aegis:inRun` IRI as written, the step is the last `/`-segment
//! of `aegis:atStep`, the agent is the last `:`-segment of
//! `aegis:performedBy`, the timestamp is `prov:endedAtTime`'s lexical form.
//! Drift here would refuse every genuine transition, so the round-trip test
//! signs with the real scheme and lands through the real gate.
//!
//! ## Where the keys come from
//!
//! `aegis:VerifierRegistration` facts — human-authored, `aegis:verifier`
//! naming the agent, `aegis:publicKey` its hex ed25519 key. Shuttle's
//! convention puts them in a `dataKind=identity` named graph that is never
//! frozen; quipu cannot know which graph the operator chose, so the registry
//! read spans EVERY graph in the store. That is a visibility decision, not a
//! trust one: protecting the registry from unauthorized writes is the
//! authority gate's job (`enforce_authority`), exactly as it is for the
//! decision registry. Any registered key for the agent may verify — an agent
//! mid key rotation legitimately has two registrations.
//!
//! ## Definition-time, opt-in
//!
//! Runs inside `stage_and_guard` against the pending post-state, so a
//! transition and its signature land in one transaction or not at all.
//! Gated by `[quipu.governance] verify_transitions`, default **false** —
//! opt-in, never switched on beneath a running deployment, the same posture
//! as `validate_placement`. Events already in the graph are not re-validated:
//! turning the flag on cannot retroactively break a store, only refuse the
//! next unverifiable transition.

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, PROV, RDF_TYPE};
use crate::store::{Datum, Store};
use crate::types::Value;

/// The canonical byte string an agent signs for one transition. Byte-for-byte
/// `shuttle/signing.py::transition_message` — deterministic field order, so
/// the gate re-derives the exact message the agent signed from the staged
/// facts alone.
#[must_use]
pub fn transition_message(
    run_iri: &str,
    step: &str,
    from_state: &str,
    to_state: &str,
    at: &str,
    agent: &str,
) -> Vec<u8> {
    format!("shuttle-transition-v1|{run_iri}|{step}|{from_state}|{to_state}|{at}|{agent}")
        .into_bytes()
}

/// One staged `aegis:TransitionEvent`, as read back from the pending
/// post-state. Every field is a Vec of DISTINCT values: the canonical message
/// has exactly one slot per field, so a multi-valued field cannot be signed
/// over and is refused as ambiguous rather than resolved by row order.
#[derive(Debug, Default, Clone)]
struct Transition {
    in_run: Vec<String>,
    at_step: Vec<String>,
    from_state: Vec<String>,
    to_state: Vec<String>,
    ended_at: Vec<String>,
    performed_by: Vec<String>,
    signature: Vec<String>,
}

impl Transition {
    /// The `(field name, values)` table the checks below walk. Field names are
    /// the wire vocabulary, so a refusal names what the author actually wrote.
    fn fields(&self) -> [(&'static str, &Vec<String>); 7] {
        [
            ("aegis:inRun", &self.in_run),
            ("aegis:atStep", &self.at_step),
            ("aegis:fromState", &self.from_state),
            ("aegis:toState", &self.to_state),
            ("prov:endedAtTime", &self.ended_at),
            ("aegis:performedBy", &self.performed_by),
            ("aegis:signature", &self.signature),
        ]
    }

    /// Check this event against the signature rules, returning the reason it
    /// must be refused. `Ok(None)` means the signature verifies. `keys` maps
    /// an agent name to its registered public keys — a closure so the rule
    /// stays a pure function the tests can drive without a store.
    fn violation(
        &self,
        iri: &str,
        keys: &dyn Fn(&str) -> Result<Vec<String>>,
    ) -> Result<Option<String>> {
        // Unsigned FIRST, and distinctly: it is the headline failure this gate
        // exists for, and "you forgot to sign" is a different remedy from "a
        // field is missing".
        if self.signature.is_empty() {
            return Ok(Some(format!(
                "transition '{iri}' carries no aegis:signature. Every \
                 TransitionEvent must be signed by its performing agent over \
                 the canonical shuttle-transition-v1 message; an unsigned \
                 transition is a claim anyone with write access could have \
                 made. Sign the transition (shuttle does this on `advance`) \
                 and land the signature in the same write."
            )));
        }
        let missing: Vec<&str> = self
            .fields()
            .iter()
            .filter(|(_, vs)| vs.is_empty())
            .map(|(f, _)| *f)
            .collect();
        if !missing.is_empty() {
            return Ok(Some(format!(
                "transition '{iri}' is missing {} — without every canonical \
                 field the signed shuttle-transition-v1 message cannot be \
                 re-derived, so the signature cannot be checked, and an \
                 uncheckable signature is refused rather than waved through. \
                 Land the full TransitionEvent in one write.",
                missing.join(", ")
            )));
        }
        for (field, values) in self.fields() {
            if values.len() > 1 {
                let mut sorted = values.clone();
                sorted.sort();
                return Ok(Some(format!(
                    "transition '{iri}' has {} distinct values for {field} \
                     ({}). The canonical message has one slot per field, so \
                     nothing can decide which value was signed over. Retract \
                     the stale value in the same transaction that asserts the \
                     new one.",
                    sorted.len(),
                    sorted.join(", ")
                )));
            }
            // Shuttle's signer refuses a '|' inside any field because the
            // encoding would be ambiguous; the verifier must too, or a forged
            // field boundary could shift content between slots.
            if let Some(v) = values.first()
                && v.contains('|')
            {
                return Ok(Some(format!(
                    "transition '{iri}' has a '|' inside {field} (\"{v}\"), \
                     which makes the canonical shuttle-transition-v1 encoding \
                     ambiguous. Shuttle's signer refuses such fields; so does \
                     this gate."
                )));
            }
        }

        // Field derivations, mirroring `shuttle verify` exactly: agent is the
        // last ':'-segment of the performedBy IRI, step the last '/'-segment
        // of the atStep IRI, run the inRun IRI verbatim.
        let run = &self.in_run[0];
        let step = last_segment(&self.at_step[0], '/');
        let agent = last_segment(&self.performed_by[0], ':');
        let registered = keys(agent)?;
        if registered.is_empty() {
            return Ok(Some(format!(
                "transition '{iri}' was performed by '{agent}', who has no \
                 aegis:VerifierRegistration in this store. A signature only \
                 attests once its key is registered by a human — have one \
                 register agent '{agent}''s public key (shuttle prints the \
                 registration: `shuttle register {agent} <workflow>`), then \
                 retry."
            )));
        }
        let message = transition_message(
            run,
            step,
            &self.from_state[0],
            &self.to_state[0],
            &self.ended_at[0],
            agent,
        );
        // Any registered key may verify (rotation); none verifying means the
        // signature is wrong for THIS content — a forged signature, a
        // tampered field, or a key the registry does not hold.
        if !registered
            .iter()
            .any(|key| crate::signing::verify_hex(key, &message, &self.signature[0]))
        {
            return Ok(Some(format!(
                "transition '{iri}''s signature does not verify under any of \
                 agent '{agent}''s {} registered key(s). Either the payload \
                 was altered after signing, the signature was forged, or the \
                 signing key is not the registered one. The canonical message \
                 checked was shuttle-transition-v1 over (run, step, from, to, \
                 at, agent) = ('{run}', '{step}', '{from}', '{to}', '{at}', \
                 '{agent}').",
                registered.len(),
                from = self.from_state[0],
                to = self.to_state[0],
                at = self.ended_at[0],
            )));
        }
        Ok(None)
    }
}

/// The last `sep`-delimited segment — `shuttle verify`'s `rsplit(sep, 1)[-1]`.
fn last_segment(value: &str, sep: char) -> &str {
    value.rsplit(sep).next().unwrap_or(value)
}

/// Verify every `aegis:TransitionEvent` this write defines or amends.
/// `Err(PolicyDenied)` on the first unverifiable one; the caller rolls the
/// savepoint back, so a refused transition leaves nothing behind.
///
/// Reads the *pending* post-state — a transition and its signature staged in
/// one transaction verify together, and a write that strips the signature
/// from an existing event fails.
pub fn verify_write(store: &Store, datums: &[Datum], graph: i64) -> Result<()> {
    let events = super::placement::touched_of_type(store, datums, graph, "TransitionEvent")?;
    for iri in &events {
        let transition = read_transition(store, iri, graph)?;
        let keys = |agent: &str| registered_keys(store, agent);
        if let Some(reason) = transition.violation(iri, &keys)? {
            return Err(Error::PolicyDenied(reason));
        }
    }
    Ok(())
}

/// Read one event's canonical fields from the pending post-state, in the
/// write's graph or ROOT — the same visibility `touched_of_type` used to find
/// it.
fn read_transition(store: &Store, iri: &str, graph: i64) -> Result<Transition> {
    let Some(entity) = store.lookup(iri)? else {
        return Ok(Transition::default());
    };
    let attrs: Vec<(String, i64)> = [
        format!("{DEFAULT_BASE_NS}inRun"),
        format!("{DEFAULT_BASE_NS}atStep"),
        format!("{DEFAULT_BASE_NS}fromState"),
        format!("{DEFAULT_BASE_NS}toState"),
        format!("{PROV}endedAtTime"),
        format!("{DEFAULT_BASE_NS}performedBy"),
        format!("{DEFAULT_BASE_NS}signature"),
    ]
    .into_iter()
    .filter_map(|name| match store.lookup(&name) {
        Ok(Some(id)) => Some(Ok((name, id))),
        Ok(None) => None,
        Err(e) => Some(Err(e)),
    })
    .collect::<Result<_>>()?;

    let mut stmt = store.prepare(
        "SELECT a, v FROM facts \
         WHERE e = ?1 AND op = 1 AND valid_to IS NULL AND (g = ?2 OR g = 0)",
    )?;
    let rows: Vec<(i64, Vec<u8>)> = stmt
        .query_map(rusqlite::params![entity, graph], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;

    let mut out = Transition::default();
    for (attr, v_bytes) in rows {
        let Some((name, _)) = attrs.iter().find(|(_, id)| *id == attr) else {
            continue;
        };
        let Some(value) = lexical_of(store, &Value::from_bytes(&v_bytes)?) else {
            continue;
        };
        let slot = match name.rsplit('/').next().unwrap_or("") {
            "inRun" => &mut out.in_run,
            "atStep" => &mut out.at_step,
            "fromState" => &mut out.from_state,
            "toState" => &mut out.to_state,
            "performedBy" => &mut out.performed_by,
            "signature" => &mut out.signature,
            // `{PROV}endedAtTime` also ends in "endedAtTime" after the '/'.
            "prov#endedAtTime" | "endedAtTime" => &mut out.ended_at,
            _ => continue,
        };
        if !slot.contains(&value) {
            slot.push(value);
        }
    }
    Ok(out)
}

/// Every registered public key for `agent`: `aegis:VerifierRegistration`
/// facts with `aegis:verifier` naming the agent, across EVERY graph — the
/// identity graph is a named graph of the operator's choosing (see the module
/// doc for why this is visibility, not trust).
fn registered_keys(store: &Store, agent: &str) -> Result<Vec<String>> {
    let (Some(rdf_type), Some(class), Some(verifier), Some(public_key)) = (
        store.lookup(RDF_TYPE)?,
        store.lookup(&format!("{DEFAULT_BASE_NS}VerifierRegistration"))?,
        store.lookup(&format!("{DEFAULT_BASE_NS}verifier"))?,
        store.lookup(&format!("{DEFAULT_BASE_NS}publicKey"))?,
    ) else {
        // A term never interned means no registration can exist yet.
        return Ok(Vec::new());
    };
    let mut stmt = store.prepare(
        "SELECT pk.v FROM facts t \
         JOIN facts vr ON vr.e = t.e AND vr.a = ?3 AND vr.v = ?4 \
              AND vr.op = 1 AND vr.valid_to IS NULL \
         JOIN facts pk ON pk.e = t.e AND pk.a = ?5 \
              AND pk.op = 1 AND pk.valid_to IS NULL \
         WHERE t.a = ?1 AND t.v = ?2 AND t.op = 1 AND t.valid_to IS NULL",
    )?;
    let class_bytes = Value::Ref(class).to_bytes();
    let agent_bytes = Value::Str(agent.to_string()).to_bytes();
    let raw: Vec<Vec<u8>> = stmt
        .query_map(
            rusqlite::params![rdf_type, class_bytes, verifier, agent_bytes, public_key],
            |row| row.get(0),
        )?
        .collect::<std::result::Result<_, _>>()?;
    let mut keys = Vec::new();
    for bytes in raw {
        if let Some(key) = lexical_of(store, &Value::from_bytes(&bytes)?)
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// The lexical form of a stored value: strings as-is, typed literals by their
/// lexical form, refs resolved to their IRI — whichever shape the writer's
/// route (Turtle ingest vs direct transact) produced.
fn lexical_of(store: &Store, v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Typed { lexical, .. } => Some(lexical.clone()),
        Value::Ref(id) => store.resolve(*id).ok(),
        Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;

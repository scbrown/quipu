//! R2: reach an alignment assertion WITHOUT knowing it exists.
//!
//! `/set` is the primitive that retracts one, and "use `/set` on the predicate"
//! is a fix for somebody who already knows there is something to undo. That is
//! the half that does not exist on its own, because **a `quipu:distinctFrom` is
//! invisible by construction**: the system's response to one is to stop
//! proposing the pair, so nothing ever shows it to you again. An operator who
//! wonders "why does this pair never come up?" has no thread to pull.
//!
//! So discovery is the requirement, not the retraction call. Given two IRIs and
//! no prior knowledge, [`about_pair`] answers: is anything asserted here, by
//! whom, on what evidence, when — and what command undoes it.
//!
//! ## Why it reads the graph rather than the mapping set
//!
//! A mapping set is a file someone has to still have. The alignment graph is
//! what the store actually holds, and it is what `verify` walks. Reading the
//! set would answer "what did we intend"; reading the graph answers "what is
//! true of this store right now", which is the question an operator asking
//! about a pair is actually asking.

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::sssom::OWL_SAME_AS;
use super::verify::QUIPU_DISTINCT_FROM;

const ALIGN_NS: &str = "https://quipu.dev/ontology/align/";

/// What can be done about an assertion today.
///
/// An enum rather than a `String` so the caller cannot accidentally print a
/// command that does not exist: the "no safe command" case has to be handled,
/// not formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retraction {
    /// No graph-scoped retraction exists in quipu yet, so this assertion cannot
    /// be undone in isolation.
    NotGraphScoped {
        /// The nearest real invocation.
        closest: String,
        /// Exactly what that would ALSO destroy. Stated so an operator can
        /// decide, rather than discovering it afterwards.
        blast_radius: String,
    },
}

/// One alignment assertion, with everything needed to judge and undo it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    /// The graph the assertion lives in.
    pub graph: String,
    /// Subject IRI.
    pub subject: String,
    /// `owl:sameAs` or `quipu:distinctFrom`.
    pub predicate: String,
    /// Object IRI.
    pub object: String,
    /// `align:assertedBy` — who decided it. Empty when the row was unauthored.
    pub asserted_by: Option<String>,
    /// `align:assertedOn` — when.
    pub asserted_on: Option<String>,
    /// `align:justification` — the SSSOM justification CURIE.
    pub justification: Option<String>,
}

impl Assertion {
    /// How to retract this assertion — or why you cannot yet.
    ///
    /// **There is no graph-scoped retraction in quipu today.** `cmd_retract`
    /// takes a bare entity IRI plus an optional `--predicate`; `tool_retract`
    /// adds an optional `value`; and `Store::retract_triples` takes no graph at
    /// all. So a retraction cannot be confined to the alignment graph, and the
    /// closest real invocation would retract the predicate on that entity
    /// EVERYWHERE it appears.
    ///
    /// This method therefore returns a [`Retraction::NotGraphScoped`] rather
    /// than a command string. Rendering a command was the first implementation
    /// and it was wrong in the way that matters most here: the quoted triple
    /// begins with `<`, so the CLI accepts it AS the entity IRI and exits 1
    /// with `entity not found: <the whole triple>` — telling a reader who came
    /// because this tool said an assertion exists that nothing does.
    ///
    /// A discovery feature that hands you an action contradicting its own
    /// finding is worse than one that says plainly it has no action to offer.
    #[must_use]
    pub fn retraction(&self) -> Retraction {
        Retraction::NotGraphScoped {
            closest: format!(
                "quipu retract {} --predicate {}",
                self.subject, self.predicate
            ),
            blast_radius: format!(
                "retracts <{}> <{}> in EVERY graph and for EVERY object, not just \
                 <{}> in {}",
                self.subject, self.predicate, self.object, self.graph
            ),
        }
    }

    /// Is this an assertion that two things are DIFFERENT?
    ///
    /// Worth asking separately: this is the invisible kind. A wrong
    /// `owl:sameAs` merges two entities and the next reader sees something
    /// wrong; a wrong `distinctFrom` suppresses the pair and nobody is shown
    /// anything at all.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.predicate == QUIPU_DISTINCT_FROM
    }
}

/// What this store asserts about the pair `(a, b)`, in either direction.
///
/// Order-independent: an operator asking about `(b, a)` is asking the same
/// question, and a judgement recorded one way round must be found the other.
///
/// # Errors
///
/// Propagates store errors.
pub fn about_pair(store: &Store, a: &str, b: &str) -> Result<Vec<Assertion>> {
    let mut out = Vec::new();
    let (Some(a_id), Some(b_id)) = (store.lookup(a)?, store.lookup(b)?) else {
        // An IRI this store has never interned cannot carry an assertion. Not
        // an error: "nothing is asserted here" is a real and useful answer.
        return Ok(out);
    };
    for pred in [OWL_SAME_AS, QUIPU_DISTINCT_FROM] {
        let Some(p_id) = store.lookup(pred)? else {
            continue;
        };
        for (s_id, o_id, s_iri, o_iri) in [(a_id, b_id, a, b), (b_id, a_id, b, a)] {
            let mut stmt = store.prepare(
                "SELECT g FROM facts \
                 WHERE e = ?1 AND a = ?2 AND v = ?3 AND op = 1 AND valid_to IS NULL",
            )?;
            let want = Value::Ref(o_id).to_bytes();
            let graphs: Vec<i64> = stmt
                .query_map(rusqlite::params![s_id, p_id, want], |r| r.get(0))?
                .filter_map(std::result::Result::ok)
                .collect();
            drop(stmt);
            for g in graphs {
                out.push(Assertion {
                    graph: store.resolve(g).unwrap_or_else(|_| g.to_string()),
                    subject: s_iri.to_string(),
                    predicate: pred.to_string(),
                    object: o_iri.to_string(),
                    asserted_by: provenance(store, s_id, "assertedBy", g)?,
                    asserted_on: provenance(store, s_id, "assertedOn", g)?,
                    justification: provenance(store, s_id, "justification", g)?,
                });
            }
        }
    }
    Ok(out)
}

/// One `align:<suffix>` string value on `entity` in `graph`.
fn provenance(store: &Store, entity: i64, suffix: &str, graph: i64) -> Result<Option<String>> {
    let Some(attr) = store.lookup(&format!("{ALIGN_NS}{suffix}"))? else {
        return Ok(None);
    };
    let mut stmt = store.prepare(
        "SELECT v FROM facts \
         WHERE e = ?1 AND a = ?2 AND g = ?3 AND op = 1 AND valid_to IS NULL LIMIT 1",
    )?;
    let raw: Option<Vec<u8>> = stmt
        .query_map(rusqlite::params![entity, attr, graph], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .filter_map(std::result::Result::ok)
        .next();
    drop(stmt);
    Ok(match raw.map(|b| Value::from_bytes(&b)) {
        Some(Ok(Value::Str(s))) if !s.is_empty() => Some(s),
        _ => None,
    })
}

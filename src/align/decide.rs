//! `align decide`: the operator's judgement, applied to a proposed set.
//!
//! `propose` decides nothing and `apply` writes only what was decided, so this
//! is the step in between — and it is the only one where a human's intent
//! enters the pipeline. That makes its failure modes different in kind from
//! the rest of alignment: every mistake here is a mistake about what somebody
//! MEANT, and none of them can be detected downstream.
//!
//! ## Two kinds of "I cannot apply this", with deliberately different answers
//!
//! * A decision naming a pair the set does not contain is **reported and
//!   counted** ([`DecideReport::unmatched`]). It has an innocent cause — a
//!   decisions file written against an older proposal — and the operator can
//!   see exactly what did not land.
//! * A decision that **contradicts** something is **refused**. There is no
//!   correct interpretation of "accept this pair" and "decline this pair" in
//!   one batch, and no correct interpretation of re-deciding a row that
//!   already carries a judgement. Picking one would be inventing intent.
//!
//! The line is: ambiguity in the DATA gets reported, contradiction in the
//! OPERATOR'S OWN INPUT gets refused. Reporting a contradiction would leave a
//! recorded judgement that nobody chose; refusing an innocent staleness would
//! make the common case unusable.
//!
//! ## Changing your mind is a RETRACTION, not a second decision
//!
//! Re-deciding an already-decided row is refused rather than overwritten,
//! because silently replacing a recorded judgement is the invisible-mutation
//! shape this module exists to avoid: the old decision leaves no trace, and
//! the audit trail says the operator always thought the new thing. Retraction
//! is a separate operation with its own provenance (R2).

use std::collections::BTreeMap;

use crate::error::{Error, Result};

use super::sssom::{MappingSet, Review};

/// What the operator said about one candidate pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// These are the same thing. Authors the row, so it derives an
    /// `owl:sameAs`.
    Accept,
    /// These are definitely NOT the same thing. Authors the row AND negates it,
    /// so it derives a `quipu:distinctFrom` — an assertion in its own right.
    Negate,
    /// Seen, set aside, no claim either way. Suppresses re-proposal and derives
    /// nothing. Deliberately NOT an assertion: see [`Review`].
    Decline,
}

impl Decision {
    /// How this decision reads in a decisions file.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Negate => "negate",
            Self::Decline => "decline",
        }
    }

    /// Parse one.
    ///
    /// # Errors
    /// The value is not one of the three. Refused rather than defaulted:
    /// reading an unrecognised decision as "decline" would silently suppress a
    /// pair the operator may have meant to accept.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "accept" => Ok(Self::Accept),
            "negate" => Ok(Self::Negate),
            "decline" => Ok(Self::Decline),
            other => Err(Error::InvalidValue(format!(
                "unknown decision {other:?}; expected \"accept\", \"negate\" or \"decline\""
            ))),
        }
    }
}

/// One line of the operator's decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRow {
    /// One IRI of the pair. Order does not matter — matched on `pair_key`.
    pub subject_id: String,
    /// The other IRI of the pair.
    pub object_id: String,
    /// What the operator said.
    pub decision: Decision,
}

impl DecisionRow {
    fn key(&self) -> (String, String) {
        if self.subject_id <= self.object_id {
            (self.subject_id.clone(), self.object_id.clone())
        } else {
            (self.object_id.clone(), self.subject_id.clone())
        }
    }
}

/// The decided set, and what could not be applied to it.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct DecideReport {
    /// The set with the operator's judgements recorded.
    pub set: MappingSet,
    /// How many decisions landed on a row.
    pub applied: usize,
    /// Decisions naming a pair this set does not contain, in input order.
    /// Reported rather than dropped: a decision that silently does nothing is
    /// indistinguishable from one that was applied.
    pub unmatched: Vec<(String, String)>,
}

impl DecideReport {
    /// Both numbers in one sentence, so neither can be reported without the
    /// other.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} decision(s) applied; {} named no pair in this set",
            self.applied,
            self.unmatched.len()
        )
    }
}

/// Record the operator's decisions on a proposed set.
///
/// # Errors
///
/// * `reviewer` is empty — an unattributed decision is not a decision, and R1's
///   provenance is derived from it.
/// * two decisions in `decisions` name the same pair with DIFFERENT verdicts.
/// * a decision names a row that already carries a judgement (that is a
///   retraction, not a decision).
pub fn decide(set: &MappingSet, decisions: &[DecisionRow], reviewer: &str) -> Result<DecideReport> {
    if reviewer.trim().is_empty() {
        return Err(Error::InvalidValue(
            "a decision needs a reviewer: the derived triple carries it as provenance, \
             and an unattributed judgement cannot be audited or retracted by its author"
                .to_string(),
        ));
    }

    // Collapse the input first, so a contradiction is refused before anything
    // is written — the same reason `apply` checks its version guard before
    // staging triples rather than unwinding them afterwards.
    let mut wanted: BTreeMap<(String, String), Decision> = BTreeMap::new();
    for row in decisions {
        let key = row.key();
        match wanted.get(&key) {
            Some(prior) if *prior != row.decision => {
                return Err(Error::InvalidValue(format!(
                    "conflicting decisions for {} / {}: {:?} and {:?}. \
                     Refused rather than resolved — choosing one would record a \
                     judgement the operator did not make",
                    key.0,
                    key.1,
                    prior.as_str(),
                    row.decision.as_str()
                )));
            }
            _ => {
                wanted.insert(key, row.decision);
            }
        }
    }

    let mut out = set.clone();
    let mut applied = 0usize;
    let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();

    for mapping in &mut out.mappings {
        let key = mapping.pair_key();
        let Some(decision) = wanted.get(&key).copied() else {
            continue;
        };
        if mapping.is_reviewed() {
            return Err(Error::InvalidValue(format!(
                "{} / {} already carries a judgement; changing it is a retraction, \
                 not a decision. Overwriting here would leave no trace of the \
                 first judgement and the audit trail would read as though the \
                 operator always thought the second",
                key.0, key.1
            )));
        }
        match decision {
            Decision::Accept => {
                mapping.author_id = Some(reviewer.to_string());
                mapping.quipu_reviewed_by = Some(reviewer.to_string());
            }
            Decision::Negate => {
                mapping.author_id = Some(reviewer.to_string());
                mapping.quipu_reviewed_by = Some(reviewer.to_string());
                mapping.predicate_modifier_not = Some(true);
            }
            Decision::Decline => {
                // NOT authored. An authored row is one an SSSOM consumer may
                // read as curated truth, and declining asserts nothing.
                mapping.quipu_review = Some(Review::Declined);
                mapping.quipu_reviewed_by = Some(reviewer.to_string());
            }
        }
        seen.insert(key, ());
        applied += 1;
    }

    let unmatched = decisions
        .iter()
        .map(DecisionRow::key)
        .filter(|k| !seen.contains_key(k))
        .fold(Vec::new(), |mut acc, k| {
            if !acc.contains(&k) {
                acc.push(k);
            }
            acc
        });

    out.sort();
    Ok(DecideReport {
        set: out,
        applied,
        unmatched,
    })
}

//! A purpose-specific interpretation of already admitted sealed Decisions.
//! No caller-supplied authorization, signer registration or judgment writer.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Object, Request};

pub(super) const MARKER: &str = "http://quipu.local/rule#judgmentClaim";
pub(super) const PROFILE: &str = include_str!("../../shapes/rule-answers.ttl");

/// Stable identity excludes witness limits and the expected policy: a record
/// with the wrong policy must be detected, rather than silently filtered away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub subject: String,
    pub positive_predicate: String,
    pub negative_predicate: String,
    pub object: Object,
    pub graph: String,
}

impl From<&Request> for Claim {
    fn from(request: &Request) -> Self {
        Self {
            subject: request.subject.clone(),
            positive_predicate: request.positive_predicate.clone(),
            negative_predicate: request.negative_predicate.clone(),
            object: request.object.clone(),
            graph: crate::schema::ROOT_GRAPH_IRI.into(),
        }
    }
}

impl Claim {
    /// SHA256 of this struct's fixed-field-order UTF-8 JSON representation.
    pub fn digest(&self) -> crate::Result<String> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| crate::Error::Serialization(error.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

/// An immutable predecessor reference, retained when resolving a deferral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Predecessor {
    pub verdict: String,
    pub decision_digest: String,
}

/// The signed JSON carrier in a Decision's rdf:value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub protocol: String,
    pub claim: Claim,
    pub authority: String,
    pub reason: String,
    pub disposition: Kind,
    pub predecessor: Option<Predecessor>,
}

/// Judgment is orthogonal to support; neither value grants action authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Deferred,
    Resolved,
}

/// A verified admitted judgment with immutable Decision and verdict identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub decision: String,
    pub decision_digest: String,
    pub verdicts: Vec<String>,
    pub record: Record,
}

/// Separate from the four-valued support algebra and from transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum Judgment {
    NoneRecorded,
    Deferred {
        current: Box<Entry>,
    },
    Resolved {
        current: Box<Entry>,
        earlier: Box<Entry>,
    },
}

pub(super) fn fold(mut entries: Vec<Entry>) -> crate::Result<Judgment> {
    use crate::Error;
    entries.sort_by(|left, right| left.decision.cmp(&right.decision));
    match entries.as_slice() {
        [] => Ok(Judgment::NoneRecorded),
        [current]
            if current.record.disposition == Kind::Deferred
                && current.record.predecessor.is_none() =>
        {
            Ok(Judgment::Deferred {
                current: Box::new(current.clone()),
            })
        }
        [one, two] => {
            let (earlier, current) = if one.record.disposition == Kind::Deferred {
                (one, two)
            } else {
                (two, one)
            };
            let predecessor = current.record.predecessor.as_ref();
            if earlier.record.disposition != Kind::Deferred
                || earlier.record.predecessor.is_some()
                || current.record.disposition != Kind::Resolved
                || !predecessor.is_some_and(|prior| {
                    earlier.verdicts.contains(&prior.verdict)
                        && prior.decision_digest == earlier.decision_digest
                })
                || earlier.record.claim != current.record.claim
            {
                return Err(Error::InvalidValue(
                    "judgment lifecycle is not a retained authorized resolution".into(),
                ));
            }
            Ok(Judgment::Resolved {
                current: Box::new(current.clone()),
                earlier: Box::new(earlier.clone()),
            })
        }
        _ => Err(Error::InvalidValue(
            "competing or unsupported judgment lifecycle heads".into(),
        )),
    }
}

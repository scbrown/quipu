//! Explicit positive/negative evidence, independent of judgment or action authority.
//!
//! This module does not change SPARQL truth or Datalog negation-as-failure.
//! A complete empty evidence pair is `Neither`; a failed read has no truth value.

use serde::{Deserialize, Serialize};

mod evidence;
pub mod judgment;
mod judgment_read;
mod ledger;

/// The four possibilities for two independent explicit support bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Truth {
    Supported,
    Refuted,
    Both,
    Neither,
}

impl Truth {
    /// Only call after both scoped support reads have completed.
    pub const fn from_support(positive: bool, negative: bool) -> Self {
        match (positive, negative) {
            (true, false) => Self::Supported,
            (false, true) => Self::Refuted,
            (true, true) => Self::Both,
            (false, false) => Self::Neither,
        }
    }
}

/// Deliberately narrow v1 term support; unsupported RDF terms must not become absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Object {
    Iri(String),
    Str(String),
}

/// A grounded claim and an explicitly selected judgment policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub subject: String,
    pub positive_predicate: String,
    pub negative_predicate: String,
    pub object: Object,
    pub judgment_policy: String,
    #[serde(default = "default_limit")]
    pub witness_limit: usize,
}

const fn default_limit() -> usize {
    64
}

impl Request {
    /// Reject ambiguous inputs before any evidence read.
    pub fn validate(&self) -> crate::Result<()> {
        let mut iris = vec![
            self.subject.as_str(),
            self.positive_predicate.as_str(),
            self.negative_predicate.as_str(),
            self.judgment_policy.as_str(),
        ];
        if let Object::Iri(iri) = &self.object {
            iris.push(iri);
        }
        for iri in iris {
            oxrdf::NamedNode::new(iri).map_err(|_| {
                crate::Error::InvalidValue("rule-query requires absolute IRIs".into())
            })?;
        }
        if self.positive_predicate == self.negative_predicate {
            return Err(crate::Error::InvalidValue(
                "rule-query requires distinct support predicates".into(),
            ));
        }
        if !(1..=256).contains(&self.witness_limit) {
            return Err(crate::Error::InvalidValue(
                "rule-query witness_limit must be in 1..=256".into(),
            ));
        }
        Ok(())
    }
}

/// One physical assertion supporting the exact grounded claim.
/// All current owners are retained up to the request cap; a cap is not absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub transaction: i64,
    pub valid_from: String,
    pub source: Option<String>,
}

/// Complete support reads or an explicit reason completeness could not be established.
/// This intermediate type grants no judgment or action authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Evidence {
    Complete {
        truth: Truth,
        positive_supported: bool,
        negative_supported: bool,
        positive: Vec<Witness>,
        negative: Vec<Witness>,
        store_id: String,
        transaction: i64,
        graph: String,
        scope: String,
    },
    Unknown {
        reason: String,
    },
}

/// Read exact current ROOT evidence in one snapshot, without interning terms.
/// Judgment completeness must be established separately in the same snapshot
/// by the eventual adapter; this foundation is not a complete answer envelope.
pub fn read_evidence(store: &crate::Store, request: &Request) -> crate::Result<Evidence> {
    request.validate()?;
    if !store.attachments.is_empty() {
        return Ok(Evidence::Unknown {
            reason: "attached store scope is unsupported".into(),
        });
    }
    let snapshot = store.conn.unchecked_transaction()?;
    let result = evidence::read(store, request);
    // Rolling back a read transaction makes the absence of writes explicit.
    snapshot.rollback()?;
    Ok(match result {
        Ok(evidence) => evidence,
        Err(error) => Evidence::Unknown {
            reason: format!("support read incomplete: {error}"),
        },
    })
}

/// Exact claim and expected policy bound into a retained complete response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimContext {
    pub claim: judgment::Claim,
    pub claim_digest: String,
    pub judgment_policy: String,
}

/// Typed rule answer; no boolean conversion or implicit action grant exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Complete {
        protocol: String,
        claim_context: Box<ClaimContext>,
        evidence: Evidence,
        judgment: judgment::Judgment,
        judgment_scope: String,
        action_authority: ActionAuthority,
    },
    Unknown {
        protocol: String,
        truth: Option<Truth>,
        reason: String,
        action_authority: ActionAuthority,
    },
}

/// The sole wire value: evidence or adjudication is not an action capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionAuthority {
    NotGranted,
}

/// Query both support and admitted judgment in one current ROOT read snapshot.
pub fn query(store: &crate::Store, request: &Request) -> crate::Result<Answer> {
    request.validate()?;
    let unknown = |reason: String| Answer::Unknown {
        protocol: "quipu.rule-answer/v1".into(),
        truth: None,
        reason,
        action_authority: ActionAuthority::NotGranted,
    };
    if !store.attachments.is_empty() {
        return Ok(unknown("attached store scope is unsupported".into()));
    }
    let snapshot = store.conn.unchecked_transaction()?;
    let result = (|| -> crate::Result<Answer> {
        // Honor the same configured consumer quality floor as /query. A
        // refused floor cannot turn into missing evidence or no judgment.
        store.check_label_floor(&[crate::schema::ROOT_GRAPH])?;
        let evidence = evidence::read(store, request)?;
        let judgment = judgment_read::read(store, request)?;
        let claim = judgment::Claim::from(request);
        let claim_digest = claim.digest()?;
        Ok(Answer::Complete {
            protocol: "quipu.rule-answer/v1".into(),
            claim_context: Box::new(ClaimContext {
                claim,
                claim_digest,
                judgment_policy: request.judgment_policy.clone(),
            }),
            evidence,
            judgment,
            judgment_scope: "current_root_records_with_retained_marker_history".into(),
            action_authority: ActionAuthority::NotGranted,
        })
    })();
    snapshot.rollback()?;
    Ok(result.unwrap_or_else(|error| unknown(error.to_string())))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod judgment_tests;
#[cfg(test)]
mod tests;

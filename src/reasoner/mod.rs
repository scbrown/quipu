//! Datalog-style rule engine over the EAVT fact log.
//!
//! Phase 2 of the reasoner rollout (see `docs/design/reasoner.md`). This
//! module derives high-level relations (e.g. `affects`, `dependsOn`) from
//! raw EAVT facts and writes them back through `Store::transact()` with a
//! distinct `source = "reasoner:<rule-id>"` provenance tag.
//!
//! # Scope of Phase 2
//!
//! - **Rule DSL:** Turtle with `rule:head` / `rule:body` string literals.
//!   A small hand-rolled body parser turns `installedIn(?pkg, ?c)` into a
//!   positional atom.
//! - **Stratification:** rulesets are checked at load time. Negation cycles
//!   are rejected with an error naming the offending predicates. Positive
//!   recursion is allowed and handled inside a single stratum.
//! - **Evaluation:** full re-derivation per call. Not yet incremental — the
//!   reactive [`TransactObserver`] path lands in Phase 3.
//! - **Body shapes:** 1-atom (projection) and 2-atom (single shared join
//!   variable) rules are supported. 3+ atoms and negation-as-failure are
//!   parsed and stratified but rejected at eval time with a clear error.
//!   Later phases will extend compilation, not the DSL.
//!
//! The datafrog crate is pulled in to do the work inside each stratum. At
//! the aegis homelab's 50K-fact target, datafrog's semi-naive evaluation is
//! orders of magnitude faster than needed, but using it now keeps the door
//! open for larger workloads without redesign.

pub mod ast;
pub mod evaluate;
pub mod parse;
pub mod stratify;

#[cfg(test)]
mod parse_tests;

pub use ast::{Atom, BodyAtom, Rule, Term};
pub use evaluate::{EvalReport, evaluate};
pub use parse::{RuleSet, parse_rules};

/// Namespace for the reasoner rule vocabulary.
///
/// Stored under this prefix in Turtle so rules can live alongside SHACL
/// shapes in the `shapes` table without colliding with shape vocabulary.
pub const RULE_NS: &str = "http://quipu.local/rule#";

/// `rule:Rule` — class that identifies a rule resource in Turtle.
pub const RULE_TYPE: &str = "http://quipu.local/rule#Rule";

/// `rule:id` — stable identifier for a rule, used in provenance tags.
pub const RULE_ID: &str = "http://quipu.local/rule#id";

/// `rule:head` — string literal holding the head atom source.
pub const RULE_HEAD: &str = "http://quipu.local/rule#head";

/// `rule:body` — string literal holding the comma-separated body atoms.
pub const RULE_BODY: &str = "http://quipu.local/rule#body";

/// `rule:prefix` — per-rule IRI prefix for unqualified predicate names.
/// Falls back to the ruleset default when absent.
pub const RULE_PREFIX: &str = "http://quipu.local/rule#prefix";

/// `rule:defaultPrefix` — subject-level default prefix for a ruleset. Only
/// honoured when attached to a `rule:RuleSet` resource.
pub const RULE_DEFAULT_PREFIX: &str = "http://quipu.local/rule#defaultPrefix";

/// `rule:RuleSet` — optional top-level container for a rules document.
pub const RULE_SET_TYPE: &str = "http://quipu.local/rule#RuleSet";

/// Errors produced while loading, stratifying, or evaluating a ruleset.
#[derive(Debug, thiserror::Error)]
pub enum ReasonerError {
    /// An RDF parse error in the Turtle source of the rules.
    #[error("rule Turtle parse error: {0}")]
    Turtle(String),

    /// A rule was declared but is missing a required property.
    #[error("rule {id:?} is missing required property {property}")]
    MissingProperty {
        /// Rule id (or IRI if id wasn't set).
        id: String,
        /// Property that was missing (human-readable short name).
        property: &'static str,
    },

    /// A rule head or body failed to parse.
    #[error("rule {id:?} {location}: {message}")]
    BadSyntax {
        /// Rule id or IRI.
        id: String,
        /// Which part of the rule failed (head/body).
        location: &'static str,
        /// Human-readable description of the problem.
        message: String,
    },

    /// A rule head contains a variable that does not appear in the body
    /// (un-ranged variable — would bind freely, rejected).
    #[error("rule {id:?} head variable ?{variable} is not bound in the body")]
    UnboundHeadVariable {
        /// Rule id.
        id: String,
        /// Variable name without the leading `?`.
        variable: String,
    },

    /// The ruleset could not be stratified because negation sits inside a
    /// recursive cycle.
    #[error("rule set is not stratifiable: negation cycle through {predicates:?}")]
    UnstratifiableCycle {
        /// Predicate IRIs forming the non-stratifiable cycle.
        predicates: Vec<String>,
    },

    /// A rule body shape is not yet supported by the evaluator.
    #[error("rule {id:?} uses unsupported feature: {feature}")]
    Unsupported {
        /// Rule id.
        id: String,
        /// Human-readable description of the unsupported feature.
        feature: String,
    },

    /// A store error surfaced while reading or writing facts.
    #[error("store error: {0}")]
    Store(#[from] crate::Error),
}

/// Result alias for reasoner operations.
pub type Result<T> = std::result::Result<T, ReasonerError>;

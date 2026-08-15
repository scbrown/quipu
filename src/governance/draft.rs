//! Drafting scaffold — from an exemplar to placement-valid advisory Turtle.
//!
//! Step 1 of `docs/design/policy-by-example.md`: the distance between "never do
//! this again" and a valid `aegis:Policy` is where the intent dies, because the
//! author must hand-compile a Selector, a claim, a constraint class, a
//! verification point, an effect and a hosting layer from a vocabulary they do
//! not carry in their head. This module closes that distance from the tooling
//! side: the caller fills a [`DraftIntent`] — the exemplar reference plus the
//! intent metadata — and gets back complete Turtle for a policy that
//!
//! - carries `aegis:exemplar`, the record that motivated it, so later refusals
//!   under it can cite their example;
//! - is **born advisory**: `aegis:effect "warn"`, hard-coded rather than a
//!   parameter (see [`ADVISORY_EFFECT`]);
//! - pre-fills what can be derived (boundary, default class/point/layer) so the
//!   human edits a filled-in form, not an empty vocabulary.
//!
//! ## What this deliberately does NOT do
//!
//! It does not bypass the definition-time placement check. The emitted Turtle
//! is *aimed at* passing `validate_placement`, and the round-trip test proves
//! the defaults do — but the check still runs at ingest and still refuses a
//! malformed result (an intent that overrides class/point into a Table-3
//! violation is refused THERE, with placement's own explanation, not
//! second-guessed here). Two validators for one rule would eventually disagree.
//!
//! It also does not draft similarity-tier predicates. Extracting a Selector and
//! tiered predicate candidates from a verdict-spool exemplar is the yupana half
//! of the design (sequencing step 2); until that lands, the scaffold drafts
//! claim-carrying policies whose evidence is a SPARQL ASK the store itself can
//! evaluate — which is also exactly the form the backtest can replay.

use crate::error::{Error, Result};
use crate::namespace::DEFAULT_BASE_NS;

/// The one effect a drafted policy can be born with.
///
/// A constant, not a field on [`DraftIntent`], because "easy to express" must
/// not become "easy to deploy a bad hard rule" (design §4): the ease is
/// front-loaded into drafting and backtesting, while enforcement keeps its
/// evidence bar — the existing advisory→enforcing promotion gates over
/// recorded traffic (`replay.rs`). A parameter here would be a lever for
/// skipping that bar at the exact moment the author is most confident and
/// least informed.
pub const ADVISORY_EFFECT: &str = "warn";

/// The filled-in form a human edits. Everything derivable is defaulted;
/// everything that names intent is required.
#[derive(Debug, Clone)]
pub struct DraftIntent {
    /// Local name for the policy IRI (`aegis:policy_<name>`). Sanitised to
    /// `[A-Za-z0-9_-]` — the IRI must survive being inlined into refusal
    /// messages and SPARQL.
    pub name: String,
    /// The human's intent sentence, kept VERBATIM as `rdfs:label`. The
    /// drafting tool's suggestions are scaffolding; what the human accepts is
    /// what the human authored, and the label is where that authorship lives.
    pub label: String,
    /// The record that motivated this rule — a Verdict, `DecisionRequest` or
    /// edit-record IRI. Required HERE (this scaffold exists to draft from an
    /// example) even though `aegis:exemplar` is optional on Policy generally.
    pub exemplar: String,
    /// The target entity-type IRI (`aegis:targets`).
    pub target_type_iri: String,
    /// The compliant condition: a SPARQL ASK with `$target`, same contract as
    /// the write gate's claims.
    pub claim: String,
    /// `aegis:constraintClass` — `None` defaults to `"soft"`. The class says
    /// what kind of bound this will be ONCE PROMOTED; the born-warn effect
    /// keeps it advisory until then, so declaring `"hard"` now is a statement
    /// of intent, not of enforcement.
    pub class: Option<String>,
    /// `aegis:verificationPoint` — `None` derives the class's natural
    /// advisory point (soft→PAA, hard→PAG).
    pub point: Option<String>,
    /// `aegis:hostedAtLayer` — `None` defaults to `"tool"`: the store's own
    /// write path is what evaluates a drafted claim, and quipu is the tool.
    pub layer: Option<String>,
    /// `aegis:authority` on the parent Directive — the human whose intent this
    /// is, when presented. Declared provenance, never invented.
    pub authority: Option<String>,
}

impl DraftIntent {
    /// The IRI the drafted policy will occupy.
    #[must_use]
    pub fn policy_iri(&self) -> String {
        format!("{DEFAULT_BASE_NS}policy_{}", self.name)
    }
}

/// Emit complete, placement-valid Turtle for the intent, or refuse with the
/// reason and the remedy.
///
/// Refusals here are only for what the placement check CANNOT catch later —
/// an unusable name, a claim the gate could never bind, an escalation class
/// the born-advisory contract excludes. Everything placement can judge is
/// left to placement.
///
/// # Errors
/// [`Error::InvalidValue`] naming the defective field and its remedy.
pub fn draft_turtle(intent: &DraftIntent) -> Result<String> {
    if intent.name.is_empty()
        || !intent
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Error::InvalidValue(format!(
            "draft name '{}' cannot form a policy IRI: use only letters, \
             digits, '_' and '-' (it is inlined into refusal messages and \
             SPARQL, so it must need no escaping anywhere)",
            intent.name
        )));
    }
    if intent.label.trim().is_empty() {
        return Err(Error::InvalidValue(
            "draft label is empty: the label is the human's intent sentence \
             kept verbatim, and a policy without one fails the Directive shape. \
             Say, in one sentence, what should never happen again."
                .into(),
        ));
    }
    if intent.exemplar.trim().is_empty() {
        return Err(Error::InvalidValue(
            "draft exemplar is empty: this scaffold drafts a rule FROM a \
             motivating case, and a policy with nothing to cite would refuse \
             writes unexplained. Name the Verdict, DecisionRequest or edit \
             record that motivated the rule (hand-authored policies without \
             one do not need the scaffold)."
                .into(),
        ));
    }
    // The gate substitutes `$target` before running the ASK; a claim without
    // it evaluates the same for every target, so the policy would fire on all
    // targets or none — almost never what "block edits like this one" means.
    // Refused here because placement cannot see inside the claim.
    if !intent.claim.contains("$target") {
        return Err(Error::InvalidValue(format!(
            "draft claim carries no $target placeholder: the write gate binds \
             $target to the touched entity before running the ASK, and a claim \
             without it judges every target identically. Write the compliant \
             condition about $target, e.g. ASK {{ $target <p> ?o }}. Got: {}",
            intent.claim
        )));
    }

    let class = intent.class.as_deref().unwrap_or("soft");
    // Escalation is not a class a BORN-ADVISORY draft can carry: the class
    // demands a reversibility window and onTimeout (placement enforces that),
    // which only mean anything under an escalating effect — and the effect
    // here is "warn" by contract. Author escalations directly; promotion of a
    // drafted rule to an escalating effect goes through the evidence gates.
    if class == "escalation" {
        return Err(Error::InvalidValue(
            "draft class 'escalation' is refused: a drafted policy is born \
             advisory (effect \"warn\") and an escalation class without an \
             escalating effect declares bounds nothing would use. Draft it as \
             \"soft\" or \"hard\" and let the promotion gates decide when (and \
             whether) it earns an escalating effect; author a true escalation \
             policy directly if a human ruling is the point."
                .into(),
        ));
    }
    // Default the class's natural advisory point (SARC Table 3): soft judges
    // completed-action data at the PAA; a hard-intent draft sits where it
    // would enforce once promoted, the PAG. An explicit point is passed
    // through untouched — placement, not this scaffold, judges it.
    let point = intent.point.as_deref().unwrap_or(match class {
        "hard" => "PAG",
        _ => "PAA",
    });
    let layer = intent.layer.as_deref().unwrap_or("tool");

    let mut turtle = format!(
        "@prefix aegis: <{DEFAULT_BASE_NS}> .\n\
         @prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .\n\n\
         <{iri}> a aegis:Policy ;\n\
         \x20   rdfs:label {label} ;\n\
         \x20   # The motivating case. Refusals under this policy cite it.\n\
         \x20   aegis:exemplar {exemplar} ;\n\
         \x20   aegis:targets {targets} ;\n\
         \x20   aegis:claim {claim} ;\n\
         \x20   aegis:boundary \"action\" ;\n\
         \x20   # Born advisory (policy-by-example design §4): never enforcing on\n\
         \x20   # day one. Promotion to an enforcing effect goes through the\n\
         \x20   # replay evidence gates — do not edit this by hand.\n\
         \x20   aegis:effect \"{effect}\" ;\n\
         \x20   aegis:constraintClass \"{class}\" ;\n\
         \x20   aegis:verificationPoint \"{point}\" ;\n\
         \x20   aegis:hostedAtLayer \"{layer}\"",
        iri = intent.policy_iri(),
        label = turtle_literal(&intent.label),
        exemplar = turtle_literal(&intent.exemplar),
        targets = turtle_literal(&intent.target_type_iri),
        claim = turtle_literal(&intent.claim),
        effect = ADVISORY_EFFECT,
    );
    if let Some(authority) = intent.authority.as_deref() {
        turtle.push_str(&format!(
            " ;\n\x20   aegis:authority {}",
            turtle_literal(authority)
        ));
    }
    turtle.push_str(" .\n");
    Ok(turtle)
}

/// A double-quoted Turtle literal with `\` and `"` escaped — claims are SPARQL
/// and routinely carry quotes, and an unescaped one would truncate the literal
/// into different (and possibly still-parseable) Turtle.
fn turtle_literal(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;

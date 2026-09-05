//! `align verify`: does every assertion in the alignment graph trace to a row
//! that authorised it?
//!
//! ## The invariant is TOTAL, and it is now-or-never (wu, aegis-sosiaa)
//!
//! Measured 2026-09-05: `https://quipu.dev/ontology/distinctFrom` holds **zero**
//! assertions in the live store. (The 2230 in the fleet graph are
//! `aegis:distinctFrom`, a deliberately separate predicate read by the
//! graph-extract skill's own gate — `skills/graph-extract/SKILL.md` says so.)
//!
//! While alignment is the SOLE writer of the quipu predicate, "every
//! `distinctFrom` traces to an asserted mapping" is a **total** check: there is
//! no foreign corpus for a bad assertion to hide in. That is only true now.
//! The moment an import, or another feature, writes that predicate, total
//! degrades to partial **and cannot be recovered** — nothing afterwards can
//! distinguish which of the untraceable assertions were ours.
//!
//! So exclusive ownership is a PRECONDITION, not an implementation detail, and
//! [`VerifyReport::render`] says so in the failure message rather than leaving
//! a future reader to infer why the check went weak.
//!
//! ## Traced is not correct
//!
//! This check proves PROVENANCE, not truth. An assertion that traces to a row
//! an operator authored is an assertion someone took responsibility for; it is
//! not an assertion that is right. `verify` passing means the graph and the
//! mapping set agree — nothing more — and it says that in its own output,
//! because a green check whose meaning is overread is how a review step becomes
//! a rubber stamp.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::sssom::MappingSet;

/// The predicate alignment asserts non-identity with.
pub const QUIPU_DISTINCT_FROM: &str = "https://quipu.dev/ontology/distinctFrom";

/// One asserted triple found in the alignment graph.
pub type Assertion = (String, String);

/// What `verify` concluded — three states, not two.
///
/// The middle state exists because "nothing failed" and "nothing was checked"
/// are not the same answer, and a two-state verdict is forced to render the
/// second as the first (wu, aegis-sosiaa, from a runner that printed "All
/// suites passed" over one UNAVAILABLE suite and zero passes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Assertions were found, and every one of them traced.
    Ok,
    /// NOTHING WAS VERIFIED. Vacuously clean, which is not clean.
    NothingVerified,
    /// At least one assertion traced to no mapping.
    Failed,
}

/// What `verify` found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerifyReport {
    /// `owl:sameAs` assertions that trace to an accepted row.
    pub traced_same_as: usize,
    /// `quipu:distinctFrom` assertions that trace to an asserted negative row.
    pub traced_distinct_from: usize,
    /// Assertions with NO row behind them, as `predicate: subject -> object`.
    ///
    /// These are the failure. An assertion nobody authorised is either a bug in
    /// `apply` or a writer other than alignment — and the second one is what
    /// ends the total invariant.
    pub untraceable: Vec<String>,
    /// Authored rows whose knot is missing from the graph.
    ///
    /// Not a failure: a set can legitimately be decided but not yet applied.
    /// Reported so `verify` can tell "not applied" from "applied wrongly",
    /// which are different problems with different fixes.
    pub unapplied: Vec<String>,
}

impl VerifyReport {
    /// Did any assertion trace to no mapping?
    ///
    /// Named for what it MEASURES, not for a verdict, because it is a
    /// two-state projection of a three-state answer and `pub` (wu, PR #123).
    /// `is_failure()` — its previous name — invites `!is_failure() == success`,
    /// and that inference is wrong: it returns `false` for `NothingVerified`
    /// too. A caller asking "was this run clean" must reach for
    /// [`VerifyReport::verdict`] and meet all three states.
    ///
    /// This is the rendering boundary the class of bug lives at: the model was
    /// three-state and the tests were three-state while the exported accessor
    /// was two-state. Separating the counts is not the same as offering the
    /// third answer where a caller reads it.
    ///
    /// An untraceable assertion FAILS rather than warns. A warning here would
    /// be read past exactly once and then forever, and the thing being warned
    /// about is an assertion that suppresses a pair everywhere while nobody is
    /// ever shown it again.
    #[must_use]
    pub fn has_untraceable(&self) -> bool {
        !self.untraceable.is_empty()
    }

    /// Process exit status: `0` clean, `2` nothing verified, `1` failed.
    ///
    /// Exists so the ergonomic path a caller reaches for PRESERVES all three
    /// states. Removing the misleading accessor is only half the fix — without
    /// a correct convenience, the first CLI author writes their own two-state
    /// collapse and it looks right to a reviewer.
    ///
    /// `2` for "nothing verified" matches the fleet's existing UNAVAILABLE
    /// tier, so the distinction survives all the way out to a shell rather
    /// than dying at the process boundary.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self.verdict() {
            Verdict::Ok => 0,
            Verdict::Failed => 1,
            Verdict::NothingVerified => 2,
        }
    }

    /// How many assertions this run actually checked.
    #[must_use]
    pub fn traced(&self) -> usize {
        self.traced_same_as + self.traced_distinct_from
    }

    /// The three-state verdict.
    ///
    /// `NothingVerified` when the set authorised rows and NONE of them were
    /// found in the graph. Every assertion traced, because there were none —
    /// and reporting that as OK is the vacuous pass: a green verdict over an
    /// empty check. It is not an error (the likely cause is simply that
    /// `apply` has not run), but it must not be silent, because "verified" and
    /// "had nothing to verify" lead to opposite next actions.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        if self.has_untraceable() {
            Verdict::Failed
        } else if self.traced() == 0 && !self.unapplied.is_empty() {
            Verdict::NothingVerified
        } else {
            Verdict::Ok
        }
    }

    /// A human-readable report that states what it does and does not prove.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "traced: {} owl:sameAs, {} quipu:distinctFrom",
            self.traced_same_as, self.traced_distinct_from
        );
        if !self.unapplied.is_empty() {
            let _ = writeln!(
                out,
                "{} authored row(s) not yet applied (not an error — run `align apply`)",
                self.unapplied.len()
            );
            for row in &self.unapplied {
                let _ = writeln!(out, "  pending  {row}");
            }
        }
        if self.verdict() == Verdict::NothingVerified {
            let _ = writeln!(
                out,
                "NOTHING VERIFIED — the mapping set authorises {} assertion(s) and the alignment \n\
                 graph contains none of them, so this run checked nothing.\n\
                 \n\
                 Every assertion traced, because there were no assertions. That is a vacuous pass\n\
                 and it is reported separately for that reason: \"verified\" and \"had nothing to\n\
                 verify\" lead to opposite next actions. The likely cause is that `align apply` has\n\
                 not run yet.",
                self.unapplied.len()
            );
        } else if self.untraceable.is_empty() {
            out.push_str(
                "OK — every assertion in the alignment graph traces to a row that authorised it.\n\
                 \n\
                 This proves PROVENANCE, not correctness. A traced assertion is one somebody took\n\
                 responsibility for; whether it is true is a separate question this check does not\n\
                 ask. It also assumes alignment is the SOLE writer of\n",
            );
            let _ = writeln!(out, "  {QUIPU_DISTINCT_FROM}");
            out.push_str(
                "which was true when this check was written (that predicate held zero assertions).\n\
                 If another feature starts writing it, this check silently weakens from total to\n\
                 partial and cannot be restored.\n",
            );
        } else {
            let _ = writeln!(
                out,
                "FAILED — {} assertion(s) in the alignment graph trace to no mapping:",
                self.untraceable.len()
            );
            for a in &self.untraceable {
                let _ = writeln!(out, "  untraceable  {a}");
            }
            out.push_str(
                "\nEach is either a bug in `align apply`, or a writer other than alignment.\n\
                 THE SECOND ONE MATTERS BEYOND THIS RUN: this check is total only while alignment\n\
                 is the sole writer of\n",
            );
            let _ = writeln!(out, "  {QUIPU_DISTINCT_FROM}");
            out.push_str(
                "Once something else writes that predicate, an untraceable assertion stops being\n\
                 evidence of a bug and this check degrades to partial permanently — nothing\n\
                 afterwards can tell which assertions were alignment's. Establish which writer\n\
                 produced these before relaxing the check.\n",
            );
        }
        out
    }
}

/// Compare what the alignment graph asserts against what the mapping set
/// authorised.
///
/// Pure: the caller collects the assertions, so the comparison is testable
/// without a store and the store query has one job.
#[must_use]
pub fn verify(
    set: &MappingSet,
    found_same_as: &[Assertion],
    found_distinct_from: &[Assertion],
) -> VerifyReport {
    let authorised_same_as: BTreeSet<(String, String)> = set
        .mappings
        .iter()
        .filter(|m| m.derives_knot())
        .map(super::sssom::Mapping::pair_key)
        .collect();
    let authorised_distinct: BTreeSet<(String, String)> = set
        .mappings
        .iter()
        .filter(|m| m.derives_distinct_from())
        .map(super::sssom::Mapping::pair_key)
        .collect();

    let key = |(s, o): &Assertion| {
        if s <= o {
            (s.clone(), o.clone())
        } else {
            (o.clone(), s.clone())
        }
    };

    let mut report = VerifyReport::default();
    let mut seen_same: BTreeSet<(String, String)> = BTreeSet::new();
    let mut seen_distinct: BTreeSet<(String, String)> = BTreeSet::new();

    for a in found_same_as {
        let k = key(a);
        if authorised_same_as.contains(&k) {
            report.traced_same_as += 1;
            seen_same.insert(k);
        } else {
            report
                .untraceable
                .push(format!("owl:sameAs: {} -> {}", a.0, a.1));
        }
    }
    for a in found_distinct_from {
        let k = key(a);
        if authorised_distinct.contains(&k) {
            report.traced_distinct_from += 1;
            seen_distinct.insert(k);
        } else {
            report
                .untraceable
                .push(format!("quipu:distinctFrom: {} -> {}", a.0, a.1));
        }
    }

    for k in authorised_same_as.difference(&seen_same) {
        report
            .unapplied
            .push(format!("owl:sameAs: {} -> {}", k.0, k.1));
    }
    for k in authorised_distinct.difference(&seen_distinct) {
        report
            .unapplied
            .push(format!("quipu:distinctFrom: {} -> {}", k.0, k.1));
    }

    report.untraceable.sort();
    report.unapplied.sort();
    report
}

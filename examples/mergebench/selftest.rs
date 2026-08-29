//! `--selftest`: the harness proving its own instruments before anyone quotes a
//! number out of it.
//!
//! Every check here exists because the corresponding number is otherwise
//! unfalsifiable from the outside. A `0` in the SHACL column means "no
//! corruption admitted" only if the validator can be shown to report a
//! violation when one is present; a determinism claim means nothing without a
//! second run to compare against; and the stable-serialisation arm was
//! silently NOT stable until check 4 was written, which is why it is a
//! permanent check rather than a one-off measurement.

use std::collections::{BTreeMap, BTreeSet};

use crate::generate::{self, Params};
use crate::model::{self, Graph, Triple, iri, lit, rdf_type};
use crate::shapes::{self, NS};
use crate::strategies;

struct Report {
    passed: usize,
    failed: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        if ok {
            self.passed += 1;
            println!("  PASS  {name}: {detail}");
        } else {
            self.failed += 1;
            println!("  FAIL  {name}: {detail}");
        }
    }
}

/// Run every self-check. Exits non-zero if any fails.
pub fn run(params: Params) {
    let mut r = Report { passed: 0, failed: 0 };
    println!("mergebench --selftest\n");

    validator_controls(&mut r);
    determinism(&mut r, params);
    oracle_is_recomputable(&mut r, params);
    stable_serialisation_is_stable(&mut r, params);
    line_merge_availability(&mut r);
    conflicts_are_held_at_base(&mut r, params);

    println!("\n{} passed, {} failed", r.passed, r.failed);
    if r.failed > 0 {
        std::process::exit(1);
    }
}

/// BOTH directions on the SHACL validator. A checker that never fires and a
/// checker that always fires produce the same column of zeros and ones from
/// the outside; only a pair of controls separates them.
fn validator_controls(r: &mut Report) {
    let ttl = shapes::turtle();
    let Ok(validator) = quipu::Validator::from_turtle(&ttl) else {
        r.check("validator-parse", false, "benchmark shapes did not parse");
        return;
    };

    let s = format!("{NS}control");
    let mut clean = Graph::new();
    clean.insert(Triple::new(&s, rdf_type(), iri(&format!("{NS}Entity"))));
    clean.insert(Triple::new(&s, format!("{NS}status"), lit("active")));
    let clean_fb = validator.validate(model::to_canonical_nt(&clean).as_bytes());

    // Two values on a `sh:maxCount 1` predicate — exactly the corruption a
    // blind union admits.
    let mut dirty = clean.clone();
    dirty.insert(Triple::new(&s, format!("{NS}status"), lit("retired")));
    let dirty_fb = validator.validate(model::to_canonical_nt(&dirty).as_bytes());

    match (clean_fb, dirty_fb) {
        (Ok(c), Ok(d)) => {
            r.check(
                "validator-negative-control",
                c.conforms && c.violations == 0,
                &format!("legal graph: conforms={} violations={}", c.conforms, c.violations),
            );
            r.check(
                "validator-positive-control",
                !d.conforms && d.violations > 0,
                &format!(
                    "two values on sh:maxCount 1: conforms={} violations={}",
                    d.conforms, d.violations
                ),
            );
        }
        _ => r.check("validator-controls", false, "validator returned an error"),
    }
}

/// The seed is the only entropy. Two runs must be byte-identical.
fn determinism(r: &mut Report, params: Params) {
    let a = generate::scenario(params);
    let b = generate::scenario(params);
    let same = a.base == b.base && a.ours == b.ours && a.theirs == b.theirs && a.truth == b.truth;
    r.check(
        "determinism",
        same,
        &format!("two runs at seed {} produced identical scenarios", params.seed),
    );
}

/// The oracle must be a function of the three graphs and the shapes, not of
/// the generator's private intent — otherwise a reader holding the published
/// graphs cannot check a single reported number.
fn oracle_is_recomputable(r: &mut Report, params: Params) {
    let s = generate::scenario(params);
    let triple_visible: BTreeSet<_> = s
        .truth
        .iter()
        .filter(|(_, c)| **c != generate::ConflictClass::AliasMint)
        .map(|(slot, _)| slot.clone())
        .collect();
    let recomputed = generate::ground_truth(
        &s.base,
        &s.ours,
        &s.theirs,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let recomputed_slots: BTreeSet<_> = recomputed.keys().cloned().collect();
    r.check(
        "oracle-recomputable",
        recomputed_slots == triple_visible && !triple_visible.is_empty(),
        &format!(
            "{} triple-visible conflicts recovered from the three graphs alone \
             ({} alias conflicts need the mint log, as documented)",
            triple_visible.len(),
            s.truth.len() - triple_visible.len()
        ),
    );
}

/// Regression for a measured defect: `to_turtle(g, 0)` must order the triples
/// two sides SHARE identically even when the sides hold different numbers of
/// triples. It did not, because the within-subject rotation was `seed % len`;
/// the `git-turtle-stable` arm was therefore re-serialised churn wearing a
/// stable arm's name, and it reported 121 unparseable lines that were the
/// harness's fault rather than git's.
fn stable_serialisation_is_stable(r: &mut Report, params: Params) {
    let s = generate::scenario(params);
    // Per subject, the ordered predicate-object list, terminator normalised —
    // the sides hold different numbers of triples, so only the RELATIVE order
    // of what they share is comparable.
    let blocks = |g: &Graph| -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut subject: Option<String> = None;
        for raw in model::to_turtle(g, 0).lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('@') {
                continue;
            }
            let stripped = line.trim_end_matches([';', '.']).trim_end();
            if !raw.starts_with(' ') {
                subject = Some(stripped.to_string());
                continue;
            }
            if let Some(subj) = &subject {
                out.entry(subj.clone()).or_default().push(stripped.to_string());
            }
        }
        out
    };
    let (o, t) = (blocks(&s.ours), blocks(&s.theirs));

    // `a`'s shared elements must appear in `b` in the same order: each side's
    // rendering of the common content is a subsequence of the other's.
    let consistent = |a: &[String], b: &[String]| -> bool {
        let bset: BTreeSet<&String> = b.iter().collect();
        let filtered: Vec<&String> = a.iter().filter(|x| bset.contains(*x)).collect();
        let aset: BTreeSet<&String> = a.iter().collect();
        let other: Vec<&String> = b.iter().filter(|x| aset.contains(*x)).collect();
        filtered == other
    };

    let mut shared_subjects = 0usize;
    let mut bad = 0usize;
    for (subject, ours_block) in &o {
        let Some(theirs_block) = t.get(subject) else { continue };
        shared_subjects += 1;
        if !consistent(ours_block, theirs_block) {
            bad += 1;
        }
    }
    r.check(
        "stable-serialisation",
        bad == 0 && shared_subjects > 0,
        &format!(
            "{shared_subjects} subjects present on both sides, {bad} whose shared \
             predicate-object lines are ordered differently"
        ),
    );
}

/// An unavailable arm must say so. A missing `git` reported as zero conflicts
/// would make the line-merge baselines look perfect.
fn line_merge_availability(r: &mut Report) {
    let available = strategies::git_available();
    let mut g = Graph::new();
    g.insert(Triple::new(format!("{NS}x"), rdf_type(), iri(&format!("{NS}Entity"))));
    let outcome = strategies::run("git-canonical", &g, &g, &g);
    r.check(
        "line-merge-availability-is-reported",
        outcome.available == available,
        &format!("git present={available}, arm reports available={}", outcome.available),
    );
}

/// The accounting contract every arm is scored under: a slot an arm hands to a
/// human is held at its BASE value in that arm's output. Without this,
/// `human_decisions` and `shacl_violations` would double-count — an arm could
/// flag a conflict AND land both values, and score well on both columns.
fn conflicts_are_held_at_base(r: &mut Report, params: Params) {
    let s = generate::scenario(params);
    let base_slots = model::by_slot(&s.base);
    let mut violations = 0usize;
    let mut checked = 0usize;
    for arm in strategies::ARMS {
        let outcome = strategies::run(arm, &s.base, &s.ours, &s.theirs);
        if !outcome.available {
            continue;
        }
        let merged = model::by_slot(&outcome.merged);
        let empty = BTreeSet::new();
        for slot in &outcome.conflicts {
            checked += 1;
            let want = base_slots.get(slot).unwrap_or(&empty);
            let got = merged.get(slot).unwrap_or(&empty);
            if want != got {
                violations += 1;
            }
        }
    }
    r.check(
        "conflicts-held-at-base",
        violations == 0,
        &format!("{checked} conflicted slots across all arms, {violations} not held at base"),
    );
}

//! PR-review report for a qpack change (aegis-fxpbys.1, milestone M2).
//!
//! `quipu share diff --report` turns two versions of a pack into the markdown a
//! reviewer reads on a pull request, in this order:
//!
//! 1. the entity-grouped fact diff from [`crate::share_diff`];
//! 2. SHACL violations INTRODUCED by the change (see [`ShaclReview`]);
//! 3. the merge `decisions.json` sidecar, when one is supplied;
//! 4. an alias caveat that is always present, plus advisory candidate pairs;
//! 5. a one-line summary, as the report's last line, for a check-run title.
//!
//! "Introduced" is a multiset difference over violation KEYS: each side is
//! validated against ITS OWN shapes (old data under the old `shapes.ttl`, new
//! data under the new one), and a key counts as introduced as many times as it
//! occurs more often in new than in old. A violation already present in old is
//! therefore never counted, and a shapes change that makes unchanged data fail
//! IS counted, because after the change the pack no longer conforms. Such rows
//! are flagged `from_shapes_change` so a reviewer can tell the two causes apart.
use std::collections::BTreeMap;

use oxrdf::{Quad, Triple};
use serde::Serialize;

use crate::error::Result;
use crate::git_merge::Decisions;
use crate::git_merge_alias::{ALIAS_THRESHOLD, AliasProposal, best, entities, similarity};
use crate::share_diff::{PackDiff, Snapshot, compact, diff, local_name};
use crate::share_merge::Graph;

const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Both sides of a change and whatever ships beside them.
pub struct ReviewInput<'a> {
    pub old: &'a [Quad],
    pub new: &'a [Quad],
    /// `None` when the old side ships no shapes (or does not exist yet).
    pub old_shapes: Option<&'a str>,
    /// `None` when the new side ships no shapes: SHACL is then NOT CHECKED.
    pub new_shapes: Option<&'a str>,
    pub decisions: Option<&'a Decisions>,
}

/// One introduced violation, shown with labels where the data has them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Violation {
    pub focus: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub constraint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// How many more times this key occurs in new than in old.
    pub count: usize,
    /// The old data already fails this way under the NEW shapes: the shapes
    /// change, not a data edit, introduced it.
    pub from_shapes_change: bool,
}

/// The SHACL half of the review. `NotChecked` is never rendered as zero.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShaclReview {
    Checked {
        introduced: Vec<Violation>,
        introduced_count: usize,
        /// Violations present on both sides (not introduced).
        preexisting: usize,
        /// Violations in old that the change removes.
        resolved: usize,
        shapes_changed: bool,
    },
    NotChecked {
        reason: String,
        /// True when the reason is the build, not the pack: a gate that relies
        /// on this report must fail rather than pass unchecked.
        gate_must_fail: bool,
    },
}

impl ShaclReview {
    /// Introduced violations, or `None` when not checked.
    pub fn introduced(&self) -> Option<usize> {
        match self {
            Self::Checked {
                introduced_count, ..
            } => Some(*introduced_count),
            Self::NotChecked { .. } => None,
        }
    }
}

/// One functional conflict from the sidecar.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionView {
    pub key: String,
    pub subject: String,
    pub predicate: String,
    pub constraint: String,
    pub base: Vec<String>,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
    /// `base` / `ours` / `theirs`, or `None` while unresolved.
    pub resolution: Option<String>,
}

/// One alias proposal recorded in the sidecar by the merge driver.
#[derive(Debug, Clone, Serialize)]
pub struct AliasDecisionView {
    pub key: String,
    pub ours: String,
    pub theirs: String,
    pub similarity: f64,
    /// `accept` / `reject`, or `None` while unresolved.
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionsReview {
    pub conflicts: Vec<DecisionView>,
    pub aliases: Vec<AliasDecisionView>,
    pub unresolved: usize,
}

/// An entity added by the change whose label nearly matches another entity's.
#[derive(Debug, Clone, Serialize)]
pub struct AliasCandidate {
    pub added: String,
    pub other: String,
    /// True when `other` is in the old pack; false when it is also new.
    pub other_in_old: bool,
    pub similarity: f64,
}

/// The whole report, in rendering order.
#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub diff: PackDiff,
    pub shacl: ShaclReview,
    pub decisions: Option<DecisionsReview>,
    pub alias_candidates: Vec<AliasCandidate>,
    pub summary: String,
}

/// Display names for IRIs: the label from either side, else a compact form.
struct Labels(BTreeMap<String, String>);

impl Labels {
    fn new(sides: &[&[Quad]]) -> Self {
        let mut map = BTreeMap::new();
        for quads in sides {
            for q in *quads {
                if let (oxrdf::NamedOrBlankNode::NamedNode(s), oxrdf::Term::Literal(l)) =
                    (&q.subject, &q.object)
                    && q.predicate.as_str() == RDFS_LABEL
                {
                    map.entry(s.as_str().to_string())
                        .or_insert_with(|| l.value().to_string());
                }
            }
        }
        Self(map)
    }

    /// `Label (compact)` for an IRI, in `<iri>` or bare form; other text as is.
    fn entity(&self, term: &str) -> String {
        let iri = term
            .strip_prefix('<')
            .and_then(|t| t.strip_suffix('>'))
            .unwrap_or(term);
        if !looks_like_iri(iri) {
            return term.to_string();
        }
        match self.0.get(iri) {
            Some(label) => format!("{label} ({})", compact(iri)),
            None => compact(iri),
        }
    }

    /// A term value: IRIs by label or compact name, literals unchanged.
    fn value(&self, term: &str) -> String {
        let bracketed = term.starts_with('<') && term.ends_with('>');
        let iri = term.trim_start_matches('<').trim_end_matches('>');
        if bracketed || looks_like_iri(term) {
            return self.0.get(iri).cloned().unwrap_or_else(|| compact(iri));
        }
        // An N-Triples typed literal (`"30"^^<...#integer>`): compact the datatype.
        match term.rsplit_once("^^<") {
            Some((lexical, dt)) if term.starts_with('"') && dt.ends_with('>') => {
                format!("{lexical}^^{}", compact(dt.trim_end_matches('>')))
            }
            _ => term.to_string(),
        }
    }
}

fn looks_like_iri(s: &str) -> bool {
    !s.contains(char::is_whitespace)
        && !s.starts_with('"')
        && (s.contains("://") || s.starts_with("urn:"))
}

#[cfg_attr(not(feature = "shacl"), allow(dead_code))]
type Key = (String, Option<String>, String, Option<String>);

#[cfg_attr(not(feature = "shacl"), allow(dead_code))]
struct Found {
    count: usize,
    message: Option<String>,
}

/// Blank-node labels are snapshot-local (RDFC may relabel every one between
/// two versions), so they are collapsed for matching. LIMIT: two violations
/// that differ only by which blank node they are on match each other.
#[cfg_attr(not(feature = "shacl"), allow(dead_code))]
fn stable(term: &str) -> String {
    if term.starts_with("_:") {
        "_:".into()
    } else {
        term.to_string()
    }
}

/// Validate one side and tally its violations by key. `None` shapes = none.
#[cfg(feature = "shacl")]
fn tally(shapes: Option<&str>, quads: &[Quad]) -> Result<BTreeMap<Key, Found>> {
    let mut out = BTreeMap::new();
    let Some(shapes) = shapes else {
        return Ok(out);
    };
    // Named graphs are flattened: shapes target the pack's facts, not its
    // graph layout. N-Triples is a Turtle subset, which the validator reads.
    let data: String = quads
        .iter()
        .map(|q| format!("{} .\n", Triple::from(q.clone())))
        .collect();
    let feedback = crate::shacl::Validator::from_turtle(shapes)?.validate(data.as_bytes())?;
    for issue in feedback.results {
        if !issue.severity.to_ascii_lowercase().contains("violation") {
            continue;
        }
        let key = (
            stable(&issue.focus_node),
            issue.path.clone(),
            issue.component.clone(),
            issue.value.as_deref().map(stable),
        );
        let found = out.entry(key).or_insert(Found {
            count: 0,
            message: issue.message.clone(),
        });
        found.count += 1;
    }
    Ok(out)
}

#[cfg(feature = "shacl")]
fn check_shacl(input: &ReviewInput<'_>, labels: &Labels) -> Result<ShaclReview> {
    let Some(new_shapes) = input.new_shapes else {
        return Ok(ShaclReview::NotChecked {
            reason: "the new pack ships no shapes.ttl".into(),
            gate_must_fail: false,
        });
    };
    let old = tally(input.old_shapes, input.old)?;
    let new = tally(Some(new_shapes), input.new)?;
    let shapes_changed = input.old_shapes != Some(new_shapes);
    let old_under_new = if shapes_changed {
        tally(Some(new_shapes), input.old)?
    } else {
        BTreeMap::new()
    };
    let count = |m: &BTreeMap<Key, Found>, k: &Key| m.get(k).map_or(0, |f| f.count);
    let mut introduced = Vec::new();
    let mut preexisting = 0;
    for (key, found) in &new {
        let before = count(&old, key);
        preexisting += found.count.min(before);
        if found.count > before {
            introduced.push(Violation {
                focus: labels.entity(&key.0),
                path: key.1.as_deref().map(|p| labels.value(p)),
                constraint: labels.value(&key.2),
                value: key.3.as_deref().map(|v| labels.value(v)),
                message: found.message.clone(),
                count: found.count - before,
                from_shapes_change: count(&old_under_new, key) > before,
            });
        }
    }
    let resolved = old
        .iter()
        .map(|(k, f)| f.count.saturating_sub(count(&new, k)))
        .sum();
    Ok(ShaclReview::Checked {
        introduced_count: introduced.iter().map(|v| v.count).sum(),
        introduced,
        preexisting,
        resolved,
        shapes_changed,
    })
}

#[cfg(not(feature = "shacl"))]
fn check_shacl(_: &ReviewInput<'_>, _: &Labels) -> Result<ShaclReview> {
    Ok(ShaclReview::NotChecked {
        reason: "this quipu was built without the `shacl` feature".into(),
        gate_must_fail: true,
    })
}

fn review_decisions(d: &Decisions, labels: &Labels) -> DecisionsReview {
    let resolution = |key: &str| d.resolutions.get(key).cloned();
    let terms = |v: &[String]| v.iter().map(|t| labels.value(t)).collect::<Vec<_>>();
    let conflicts: Vec<DecisionView> = d
        .conflicts
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let key = format!("conflict:{i}");
            DecisionView {
                subject: labels.entity(&c.subject),
                predicate: local_name(&c.predicate),
                constraint: format!("sh:maxCount {}", c.max_count),
                base: terms(&c.base),
                ours: terms(&c.ours),
                theirs: terms(&c.theirs),
                resolution: resolution(&key),
                key,
            }
        })
        .collect();
    let aliases: Vec<AliasDecisionView> = d
        .aliases
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let key = format!("alias:{i}");
            AliasDecisionView {
                ours: labels.entity(&a.ours),
                theirs: labels.entity(&a.theirs),
                similarity: a.similarity,
                resolution: resolution(&key),
                key,
            }
        })
        .collect();
    let unresolved = conflicts.iter().filter(|c| c.resolution.is_none()).count()
        + aliases.iter().filter(|a| a.resolution.is_none()).count();
    DecisionsReview {
        conflicts,
        aliases,
        unresolved,
    }
}

/// Entities ADDED in new whose normalized label is within the merge driver's
/// alias threshold of a same-type entity in old, or of another added one.
fn alias_candidates(old: &[Quad], new: &[Quad], labels: &Labels) -> Vec<AliasCandidate> {
    let graph = |q: &[Quad]| -> Graph { q.iter().map(|q| Triple::from(q.clone())).collect() };
    let (old_e, new_e) = (entities(&graph(old)), entities(&graph(new)));
    let added: Vec<_> = new_e
        .iter()
        .filter(|(k, _)| !old_e.contains_key(*k))
        .collect();
    let mut out = Vec::new();
    for (a, ae) in &added {
        let others = old_e
            .iter()
            .chain(added.iter().copied().filter(|(b, _)| b > a));
        let candidates: Vec<AliasProposal> = others
            .filter_map(|(b, be)| {
                let score = similarity(ae, be)?;
                (score >= ALIAS_THRESHOLD).then(|| AliasProposal {
                    ours: (*a).clone(),
                    theirs: b.clone(),
                    similarity: score,
                })
            })
            .collect();
        out.extend(best(candidates).into_iter().map(|p| AliasCandidate {
            added: labels.entity(&p.ours),
            other_in_old: old_e.contains_key(&p.theirs),
            other: labels.entity(&p.theirs),
            similarity: p.similarity,
        }));
    }
    out
}

fn summary(r: &Review) -> String {
    let d = &r.diff;
    let facts = if d.entities.is_empty() {
        "no semantic changes".to_string()
    } else {
        format!(
            "{} {} changed ({} changed, {} added, {} removed facts)",
            d.entities.len(),
            if d.entities.len() == 1 {
                "entity"
            } else {
                "entities"
            },
            d.changed,
            d.added,
            d.removed
        )
    };
    let shacl = match r.shacl.introduced() {
        Some(n) => format!("SHACL {n} introduced"),
        None => "SHACL NOT CHECKED".to_string(),
    };
    let decisions = match &r.decisions {
        Some(d) => format!(
            "{} merge decisions ({} unresolved)",
            d.conflicts.len() + d.aliases.len(),
            d.unresolved
        ),
        None => "no merge decisions".to_string(),
    };
    format!(
        "qpack review: {facts}; {shacl}; {decisions}; {} alias candidates",
        r.alias_candidates.len()
    )
}

/// Build the review. Errors when a side's shapes or data cannot be validated:
/// an unevaluable gate must not read as a pass.
pub fn review(input: &ReviewInput<'_>) -> Result<Review> {
    let labels = Labels::new(&[input.new, input.old]);
    let mut r = Review {
        diff: diff(&Snapshot::new(input.old), &Snapshot::new(input.new)),
        shacl: check_shacl(input, &labels)?,
        decisions: input.decisions.map(|d| review_decisions(d, &labels)),
        alias_candidates: alias_candidates(input.old, input.new, &labels),
        summary: String::new(),
    };
    r.summary = summary(&r);
    Ok(r)
}

#[path = "share_review_render.rs"]
mod render;
pub use render::render_report_markdown;

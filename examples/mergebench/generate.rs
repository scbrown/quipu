//! The divergence generator: one base graph, two independently edited copies,
//! and the ground-truth conflict set.
//!
//! The two sides are generated INDEPENDENTLY from the same base. The ground
//! truth is then derived from `(base, ours, theirs)` and the shapes graph by
//! [`ground_truth`] — it is not a log of what the generator intended. That
//! matters: an oracle built from the generator's intent scores the generator,
//! not the merge operator, and it cannot be reproduced by a reader who only
//! has the three graphs. A reader with the three graphs and the shapes can
//! recompute this oracle exactly.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Graph, Slot, Term, Triple, by_slot, iri, lit, rdf_type};
use crate::rng::SplitMix64;
use crate::shapes::{self, NS};

/// Which divergent copy an edit belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The local copy.
    Ours,
    /// The remote copy.
    Theirs,
}

/// Why a slot needs a human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictClass {
    /// Both sides set a functional predicate to different values.
    FunctionalDivergence,
    /// One side retracted a functional value; the other re-described it.
    DeleteModify,
    /// The union of both sides' additions exceeds a declared `sh:maxCount`.
    CardinalityOverflow,
    /// Both sides minted a differently-named node for the same entity. Invisible
    /// at the triple level — reported as a MISS for every operator here,
    /// including ours.
    AliasMint,
}

impl ConflictClass {
    /// Stable name for metrics output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FunctionalDivergence => "functional-divergence",
            Self::DeleteModify => "delete-modify",
            Self::CardinalityOverflow => "cardinality-overflow",
            Self::AliasMint => "alias-mint",
        }
    }
}

/// One divergence scenario: the three graphs plus the oracle.
pub struct Scenario {
    /// The common ancestor.
    pub base: Graph,
    /// The local copy.
    pub ours: Graph,
    /// The remote copy.
    pub theirs: Graph,
    /// Slots a human must decide, with the class that makes each one a decision.
    pub truth: BTreeMap<Slot, ConflictClass>,
    /// Total edits applied across both sides — the denominator for
    /// "human decisions per 1k edits".
    pub edits: usize,
    /// Generator parameters, echoed into the metrics file.
    pub params: Params,
}

/// Generator parameters. Every one of them is reported alongside the numbers
/// they produced; none of them is implicit.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Params {
    /// Entities in the base graph.
    pub entities: usize,
    /// Edits applied to each side.
    pub edits_per_side: usize,
    /// Fraction of edits aimed at the contended entity set, in `0.0..=1.0`.
    pub overlap: f64,
    /// Seed — the only entropy in the whole run.
    pub seed: u64,
}

fn entity(i: usize) -> String {
    format!("{NS}e{i}")
}

/// Build the common ancestor: `n` typed entities with every declared predicate
/// populated, so an edit has somewhere to land in every conflict class.
fn base_graph(n: usize, rng: &mut SplitMix64) -> Graph {
    let mut g = Graph::new();
    for i in 0..n {
        let s = entity(i);
        g.insert(Triple::new(&s, rdf_type(), iri(&format!("{NS}Entity"))));
        g.insert(Triple::new(
            &s,
            format!("{NS}label"),
            lit(&format!("entity {i}")),
        ));
        g.insert(Triple::new(&s, format!("{NS}status"), lit("active")));
        g.insert(Triple::new(
            &s,
            format!("{NS}owner"),
            iri(&format!("{NS}team{}", i % 5)),
        ));
        g.insert(Triple::new(&s, format!("{NS}version"), lit("1")));
        // Two of the four allowed tags: the union of one addition per side
        // stays legal, a second addition per side overflows. The generator
        // does not choose which; the edit stream does.
        g.insert(Triple::new(
            &s,
            format!("{NS}tag"),
            lit(&format!("t{}", i % 7)),
        ));
        g.insert(Triple::new(
            &s,
            format!("{NS}tag"),
            lit(&format!("t{}", (i + 3) % 7)),
        ));
        for k in 0..=(rng.below(2) as usize) {
            g.insert(Triple::new(
                &s,
                format!("{NS}note"),
                lit(&format!("note {i}.{k}")),
            ));
        }
        if n > 1 {
            let target = (i + 1 + rng.below(3) as usize) % n;
            g.insert(Triple::new(
                &s,
                format!("{NS}relatedTo"),
                iri(&entity(target)),
            ));
        }
    }
    g
}

/// The six edit kinds from the plan's edit model, applied in place.
fn apply_edits(
    g: &mut Graph,
    aliases: &mut BTreeMap<String, String>,
    side: Side,
    p: Params,
    rng: &mut SplitMix64,
) {
    // The contended set: edits that "overlap" land here, so the overlap
    // parameter controls collision probability directly rather than through a
    // birthday effect over the whole entity range.
    let hot = (p.entities / 4).max(1);
    let tag = match side {
        Side::Ours => "o",
        Side::Theirs => "t",
    };

    for k in 0..p.edits_per_side {
        let contended = (rng.below(1000) as f64) / 1000.0 < p.overlap;
        let idx = if contended {
            rng.below(hot as u64) as usize
        } else {
            hot + rng.below((p.entities - hot).max(1) as u64) as usize
        };
        let idx = idx.min(p.entities - 1);
        let s = entity(idx);

        match rng.below(6) {
            // Re-describe: change a functional value. The conflict class the
            // paper is named after.
            0 => {
                let pred = &shapes::FUNCTIONAL[rng.below(shapes::FUNCTIONAL.len() as u64) as usize];
                let piri = pred.iri();
                g.retain_slot_removed(&s, &piri);
                g.insert(Triple::new(&s, &piri, lit(&format!("{tag}{k}"))));
            }
            // Retract: drop a functional value outright.
            1 => {
                let pred = &shapes::FUNCTIONAL[rng.below(shapes::FUNCTIONAL.len() as u64) as usize];
                g.retain_slot_removed(&s, &pred.iri());
            }
            // Assert: add a bounded-predicate value. Two of these on the same
            // entity from opposite sides is what overflows `sh:maxCount 4`.
            2 => {
                let pred = &shapes::BOUNDED[rng.below(shapes::BOUNDED.len() as u64) as usize];
                g.insert(Triple::new(&s, pred.iri(), lit(&format!("{tag}tag{k}"))));
            }
            // Assert: add a multi-valued value. Unions cleanly; the arm that
            // proves union is RIGHT for most edits.
            3 => {
                let pred = &shapes::MULTI[rng.below(shapes::MULTI.len() as u64) as usize];
                let o = if pred.name == "relatedTo" {
                    iri(&entity(rng.below(p.entities as u64) as usize))
                } else {
                    lit(&format!("{tag} note {k}"))
                };
                g.insert(Triple::new(&s, pred.iri(), o));
            }
            // Re-type: add a second `rdf:type`. Dual-typing is the convention,
            // so this is a legal union, not a conflict — included because a
            // node-overlap heuristic flags it anyway.
            4 => {
                g.insert(Triple::new(&s, rdf_type(), iri(&format!("{NS}Reviewed"))));
            }
            // Alias-mint / node-add: a new node. When both sides mint a node
            // carrying the SAME label for the same entity, that is the alias
            // class — semantically one entity, two names, and no triple-level
            // operator can see it.
            _ => {
                let alias = format!("{s}_{tag}{k}");
                g.insert(Triple::new(&alias, rdf_type(), iri(&format!("{NS}Entity"))));
                g.insert(Triple::new(
                    &alias,
                    format!("{NS}label"),
                    lit(&format!("entity {idx}")),
                ));
                g.insert(Triple::new(&alias, format!("{NS}status"), lit("active")));
                aliases.insert(alias, s.clone());
            }
        }
    }
}

/// Helper: remove every value in a `(subject, predicate)` slot.
trait SlotOps {
    fn retain_slot_removed(&mut self, s: &str, p: &str);
}

impl SlotOps for Graph {
    fn retain_slot_removed(&mut self, s: &str, p: &str) {
        self.retain(|t| !(t.s == s && t.p == p));
    }
}

/// Derive the oracle from the three graphs and the shapes graph.
///
/// Every rule here is a statement about what a HUMAN must decide, not about
/// what any operator does. Read with `BUILD_REPORT.md` §3.
#[must_use]
pub fn ground_truth(
    base: &Graph,
    ours: &Graph,
    theirs: &Graph,
    ours_aliases: &BTreeMap<String, String>,
    theirs_aliases: &BTreeMap<String, String>,
) -> BTreeMap<Slot, ConflictClass> {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let mut out: BTreeMap<Slot, ConflictClass> = BTreeMap::new();

    let mut slots: BTreeSet<Slot> = BTreeSet::new();
    slots.extend(b.keys().cloned());
    slots.extend(o.keys().cloned());
    slots.extend(t.keys().cloned());

    for slot in slots {
        let Some(bound) = shapes::max_count(&slot.1) else {
            // Unbounded: concurrent additions union, and a deletion on one side
            // is simply honoured. Never a decision.
            continue;
        };
        let empty = BTreeSet::new();
        let bv = b.get(&slot).unwrap_or(&empty);
        let ov = o.get(&slot).unwrap_or(&empty);
        let tv = t.get(&slot).unwrap_or(&empty);
        if ov == tv {
            continue;
        }

        let merged = set_three_way(bv, ov, tv);
        if merged.len() > bound {
            let class = if bound == 1 {
                ConflictClass::FunctionalDivergence
            } else {
                ConflictClass::CardinalityOverflow
            };
            out.insert(slot, class);
            continue;
        }
        // Delete/modify on a functional slot: one side removed the value the
        // other side re-described. Set algebra answers it, but the two sides
        // expressed incompatible intent about the same slot, so it is reported
        // as its own class and scored separately.
        if bound == 1 && !bv.is_empty() {
            let ours_removed = ov.is_empty();
            let theirs_removed = tv.is_empty();
            let ours_changed = !ov.is_empty() && ov != bv;
            let theirs_changed = !tv.is_empty() && tv != bv;
            if (ours_removed && theirs_changed) || (theirs_removed && ours_changed) {
                out.insert(slot, ConflictClass::DeleteModify);
            }
        }
    }

    // Alias-mint: both sides minted a node for the same underlying entity.
    let mut by_entity: BTreeMap<&String, (Vec<&String>, Vec<&String>)> = BTreeMap::new();
    for (alias, real) in ours_aliases {
        by_entity.entry(real).or_default().0.push(alias);
    }
    for (alias, real) in theirs_aliases {
        by_entity.entry(real).or_default().1.push(alias);
    }
    for (real, (o_aliases, t_aliases)) in by_entity {
        if !o_aliases.is_empty() && !t_aliases.is_empty() {
            out.insert(
                (real.clone(), format!("{NS}aliasOf")),
                ConflictClass::AliasMint,
            );
        }
    }

    out
}

/// Set-algebraic three-way merge of one slot's values: keep what neither side
/// deleted, add what either side added.
#[must_use]
pub fn set_three_way(
    base: &BTreeSet<Term>,
    ours: &BTreeSet<Term>,
    theirs: &BTreeSet<Term>,
) -> BTreeSet<Term> {
    let mut out: BTreeSet<Term> = BTreeSet::new();
    for v in base {
        if ours.contains(v) && theirs.contains(v) {
            out.insert(v.clone());
        }
    }
    for v in ours {
        if !base.contains(v) {
            out.insert(v.clone());
        }
    }
    for v in theirs {
        if !base.contains(v) {
            out.insert(v.clone());
        }
    }
    out
}

/// Generate one scenario.
#[must_use]
pub fn scenario(params: Params) -> Scenario {
    let mut rng = SplitMix64::new(params.seed);
    let base = base_graph(params.entities, &mut rng);

    // Independent streams per side: neither side's edits can be a function of
    // the other's, which is what makes the divergence a real divergence.
    let mut ours = base.clone();
    let mut ours_aliases = BTreeMap::new();
    let mut rng_o = SplitMix64::new(params.seed ^ 0x0000_0000_0000_00A1);
    apply_edits(&mut ours, &mut ours_aliases, Side::Ours, params, &mut rng_o);

    let mut theirs = base.clone();
    let mut theirs_aliases = BTreeMap::new();
    let mut rng_t = SplitMix64::new(params.seed ^ 0x0000_0000_0000_00B2);
    apply_edits(
        &mut theirs,
        &mut theirs_aliases,
        Side::Theirs,
        params,
        &mut rng_t,
    );

    let truth = ground_truth(&base, &ours, &theirs, &ours_aliases, &theirs_aliases);
    Scenario {
        base,
        ours,
        theirs,
        truth,
        edits: params.edits_per_side * 2,
        params,
    }
}

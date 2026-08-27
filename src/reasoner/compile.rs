//! Per-rule compilation onto datafrog.
//!
//! Each rule compiles to a left-deep join **pipeline** advanced one step per
//! `while iteration.changed()` tick:
//!
//! 1. The first positive atom seeds an accumulator of binding rows
//!    (`Vec<i64>`, one slot per variable in order of first occurrence).
//! 2. Every further positive atom joins into the accumulator on the tuple of
//!    its shared variables (datafrog joins on any `Ord` key, so multi-variable
//!    keys are `Vec<i64>`; no shared variables means an empty key — a cross
//!    join).
//! 3. Every negated atom applies as a stratified antijoin against the
//!    predicate's tuples from lower strata (a static relation — the
//!    stratifier guarantees a negated predicate is fully derived before this
//!    rule's stratum runs).
//! 4. The head projects out of the final accumulator.
//!
//! This replaces the fixed 1-atom/2-atom shapes (quipu-923, gap G4 of
//! `docs/design/semantic-reasoning-gaps.md`): bodies of any length,
//! multi-variable joins, repeated variables within an atom (an equality
//! selection), and stratified negation-as-failure are all expressible. NAF is
//! evaluated over the materialized state of lower strata — open-world caveats
//! are documented at the rule DSL (`docs/design/reasoner.md` Q6).
//!
//! # Still rejected
//!
//! Non-binary atoms, string constants (facts are `Value::Ref` triples), a
//! body with no positive atom, a negated atom using a variable no positive
//! atom binds (unsafe), and a head variable no positive atom binds. These
//! error with [`ReasonerError::Unsupported`] so the evaluator never sees them.
//!
//! Constants keep their established meaning: in the **head** they must
//! already be interned; in a **positive body** atom a constant is a selection,
//! and one that has never been interned makes the rule *unsatisfiable* rather
//! than an error (nothing can match a term that does not exist —
//! aegis-jgxas). In a **negated** atom an un-interned constant makes the
//! negated pattern unmatchable, so the negation is vacuously true.

use std::collections::{BTreeMap, BTreeSet};

use datafrog::{Iteration, PrefixFilter, Relation, Variable};

use super::ast::{Atom, BodyAtom, Rule, Term};
use super::{ReasonerError, Result};

/// A body-atom column pinned to a constant term id: `(column, term_id)`.
type Filter = (usize, i64);

/// A binding row: one value per variable, in first-occurrence order.
type Row = Vec<i64>;

/// Does `row` satisfy every pinned column and the intra-atom equality?
fn passes(filters: &[Filter], eq: bool, row: &[i64; 2]) -> bool {
    (!eq || row[0] == row[1]) && filters.iter().all(|&(col, id)| row[col] == id)
}

/// How one slot of a reconstructed row is sourced from a `(key, rest)` split.
#[derive(Clone, Copy)]
enum RowSrc {
    Key(usize),
    Rest(usize),
}

/// Split positions of a row into `(key, rest)` and remember how to rebuild.
struct Split {
    key_positions: Vec<usize>,
    rest_positions: Vec<usize>,
    rebuild: Vec<RowSrc>,
}

impl Split {
    fn new(row_len: usize, key_positions: Vec<usize>) -> Self {
        let rest_positions: Vec<usize> = (0..row_len)
            .filter(|p| !key_positions.contains(p))
            .collect();
        let rebuild = (0..row_len)
            .map(|p| {
                key_positions.iter().position(|&k| k == p).map_or_else(
                    || RowSrc::Rest(rest_positions.iter().position(|&r| r == p).unwrap()),
                    RowSrc::Key,
                )
            })
            .collect();
        Self {
            key_positions,
            rest_positions,
            rebuild,
        }
    }
}

/// How one atom's two columns map onto variables and constants.
struct AtomCols {
    /// Columns whose variable was already bound (join keys), with the bound
    /// variable's row position: `(row_position, column)`.
    key_cols: Vec<(usize, usize)>,
    /// Columns introducing a new variable, in appearance order.
    new_cols: Vec<usize>,
    /// Columns pinned to a constant.
    filters: Vec<Filter>,
    /// Both columns carry the same (new) variable: an equality selection.
    eq: bool,
}

/// Classify an atom's columns against the variables bound so far. New
/// variables are appended to `var_order`.
fn atom_cols(
    rule: &Rule,
    atom: &Atom,
    const_ids: &BTreeMap<String, i64>,
    var_order: &mut Vec<String>,
) -> Result<AtomCols> {
    let mut cols = AtomCols {
        key_cols: Vec::new(),
        new_cols: Vec::new(),
        filters: Vec::new(),
        eq: false,
    };
    let mut new_here: Vec<&str> = Vec::new();
    for (col, term) in atom.args.iter().enumerate() {
        match term {
            Term::Var(name) => {
                if new_here.contains(&name.as_str()) {
                    // `p(?x, ?x)` with ?x new: bind once, select equality.
                    cols.eq = true;
                } else if let Some(pos) = var_order.iter().position(|v| v == name) {
                    cols.key_cols.push((pos, col));
                } else {
                    new_here.push(name);
                    var_order.push(name.clone());
                    cols.new_cols.push(col);
                }
            }
            Term::Iri(iri) => {
                // A missing constant resolves to `i64::MIN` — never a real
                // term id, so the filter cannot match.
                cols.filters
                    .push((col, const_ids.get(iri).copied().unwrap_or(i64::MIN)));
            }
            Term::Str(_) => return Err(unsupported(rule, "string constant in body atom")),
        }
    }
    Ok(cols)
}

/// A compiled datafrog plan for a single rule.
pub(crate) struct Plan {
    head_pred: String,
    head_slots: [HeadSlot; 2],
    /// A positive body constant referenced an IRI that has never been
    /// interned, so no fact can match and the rule derives the empty set.
    unsatisfiable: bool,
    first: FirstStage,
    joins: Vec<JoinStage>,
    negations: Vec<NegStage>,
}

struct FirstStage {
    pred: String,
    filters: Vec<Filter>,
    eq: bool,
    /// Columns providing the initial row, in variable order.
    row_cols: Vec<usize>,
    acc: Variable<Row>,
}

struct JoinStage {
    pred: String,
    split: Split,
    atom_key_cols: Vec<usize>,
    atom_new_cols: Vec<usize>,
    filters: Vec<Filter>,
    eq: bool,
    acc_keyed: Variable<(Row, Row)>,
    atom_keyed: Variable<(Row, Row)>,
    out: Variable<Row>,
}

struct NegStage {
    split: Split,
    relation: Relation<Row>,
    keyed: Variable<(Row, Row)>,
    out: Variable<Row>,
}

/// Where one head slot's value comes from.
#[derive(Clone)]
enum HeadSlot {
    /// Literal IRI resolved to a term id at compile time.
    Constant(i64),
    /// Position of the head variable in the final binding row.
    Pos(usize),
}

/// Compile a rule against an iteration, allocating the pipeline's helper
/// variables. `world_tuples` provides the lower-stratum/base tuples negated
/// atoms antijoin against.
pub(crate) fn compile_rule(
    iteration: &mut Iteration,
    rule: &Rule,
    const_ids: &BTreeMap<String, i64>,
    vars: &BTreeMap<String, Variable<(i64, i64)>>,
    world_tuples: &BTreeMap<String, BTreeSet<(i64, i64)>>,
) -> Result<Plan> {
    if rule.head.args.len() != 2 {
        return Err(unsupported(rule, "non-binary head atom"));
    }
    let mut positives: Vec<&Atom> = Vec::new();
    let mut negatives: Vec<&Atom> = Vec::new();
    for b in &rule.body {
        let atom = b.atom();
        if atom.args.len() != 2 {
            return Err(unsupported(rule, "non-binary body atom"));
        }
        match b {
            BodyAtom::Positive(a) => positives.push(a),
            BodyAtom::Negative(a) => negatives.push(a),
        }
    }
    if positives.is_empty() {
        return Err(unsupported(rule, "body needs at least one positive atom"));
    }
    for atom in positives.iter().chain(negatives.iter()) {
        if !vars.contains_key(&atom.predicate) {
            return Err(ReasonerError::Unsupported {
                id: rule.id.clone(),
                feature: format!(
                    "body references predicate with no variable allocated: {:?}",
                    atom.predicate
                ),
            });
        }
    }

    // A positive-atom constant that has never been interned means nothing can
    // match; the rule derives the empty set (recorded, not raised). Negated
    // atoms are exempt: an unmatchable negated pattern is vacuously true.
    let unsatisfiable = positives.iter().any(|atom| {
        atom.args
            .iter()
            .any(|t| matches!(t, Term::Iri(iri) if !const_ids.contains_key(iri)))
    });

    // First positive atom seeds the accumulator.
    let mut var_order: Vec<String> = Vec::new();
    let first_cols = atom_cols(rule, positives[0], const_ids, &mut var_order)?;
    let first = FirstStage {
        pred: positives[0].predicate.clone(),
        filters: first_cols.filters,
        eq: first_cols.eq,
        row_cols: first_cols.new_cols,
        acc: iteration.variable::<Row>(&format!("{}::acc0", rule.id)),
    };

    // Remaining positive atoms each become a join stage.
    let mut joins: Vec<JoinStage> = Vec::new();
    for (i, atom) in positives.iter().enumerate().skip(1) {
        let prev_len = var_order.len();
        let cols = atom_cols(rule, atom, const_ids, &mut var_order)?;
        let split = Split::new(prev_len, cols.key_cols.iter().map(|&(p, _)| p).collect());
        joins.push(JoinStage {
            pred: atom.predicate.clone(),
            split,
            atom_key_cols: cols.key_cols.iter().map(|&(_, c)| c).collect(),
            atom_new_cols: cols.new_cols,
            filters: cols.filters,
            eq: cols.eq,
            acc_keyed: iteration.variable(&format!("{}::join{i}::acc", rule.id)),
            atom_keyed: iteration.variable(&format!("{}::join{i}::atom", rule.id)),
            out: iteration.variable(&format!("{}::join{i}::out", rule.id)),
        });
    }

    // Negated atoms antijoin the final accumulator against a static relation
    // of the negated predicate's tuples (lower strata are complete when this
    // stratum runs — the stratifier's guarantee).
    let mut negations: Vec<NegStage> = Vec::new();
    for (i, atom) in negatives.iter().enumerate() {
        let mut key_positions: Vec<usize> = Vec::new();
        let mut var_cols: Vec<usize> = Vec::new();
        let mut filters: Vec<Filter> = Vec::new();
        let mut eq = false;
        let mut seen: Vec<&str> = Vec::new();
        for (col, term) in atom.args.iter().enumerate() {
            match term {
                Term::Var(name) => {
                    if seen.contains(&name.as_str()) {
                        eq = true;
                    } else {
                        let Some(pos) = var_order.iter().position(|v| v == name) else {
                            return Err(unsupported(
                                rule,
                                "unsafe negation: variable not bound by a positive atom",
                            ));
                        };
                        seen.push(name);
                        key_positions.push(pos);
                        var_cols.push(col);
                    }
                }
                Term::Iri(iri) => {
                    filters.push((col, const_ids.get(iri).copied().unwrap_or(i64::MIN)));
                }
                Term::Str(_) => return Err(unsupported(rule, "string constant in body atom")),
            }
        }
        let relation: Relation<Row> = world_tuples
            .get(&atom.predicate)
            .into_iter()
            .flatten()
            .filter(|&&(c0, c1)| passes(&filters, eq, &[c0, c1]))
            .map(|&(c0, c1)| var_cols.iter().map(|&c| [c0, c1][c]).collect::<Row>())
            .collect();
        negations.push(NegStage {
            split: Split::new(var_order.len(), key_positions),
            relation,
            keyed: iteration.variable(&format!("{}::neg{i}::keyed", rule.id)),
            out: iteration.variable(&format!("{}::neg{i}::out", rule.id)),
        });
    }

    let head_slot = |term: &Term| -> Result<HeadSlot> {
        match term {
            Term::Var(name) => var_order
                .iter()
                .position(|v| v == name)
                .map(HeadSlot::Pos)
                .ok_or_else(|| {
                    unsupported(rule, "head variable not bound by a positive body atom")
                }),
            Term::Iri(iri) => const_ids
                .get(iri)
                .copied()
                .map(HeadSlot::Constant)
                .ok_or_else(|| {
                    unsupported(rule, "head references an IRI that has never been interned")
                }),
            Term::Str(_) => Err(unsupported(rule, "string constant in head atom")),
        }
    };
    let head_slots = [
        head_slot(&rule.head.args[0])?,
        head_slot(&rule.head.args[1])?,
    ];

    Ok(Plan {
        head_pred: rule.head.predicate.clone(),
        head_slots,
        unsatisfiable,
        first,
        joins,
        negations,
    })
}

impl Plan {
    /// Advance this rule by one iteration tick, writing any new tuples
    /// into the head predicate's variable.
    pub(crate) fn step(&self, vars: &BTreeMap<String, Variable<(i64, i64)>>) {
        if self.unsatisfiable {
            return;
        }
        let head = vars
            .get(&self.head_pred)
            .expect("head variable must have been allocated before compile");

        // Stage 0: seed the accumulator from the first atom's source.
        let Some(src) = vars.get(&self.first.pred) else {
            return;
        };
        let row_cols = self.first.row_cols.clone();
        let filters = self.first.filters.clone();
        let eq = self.first.eq;
        self.first.acc.from_leapjoin(
            src,
            PrefixFilter::from(move |&(c0, c1): &(i64, i64)| passes(&filters, eq, &[c0, c1])),
            move |&(c0, c1), &()| row_cols.iter().map(|&c| [c0, c1][c]).collect::<Row>(),
        );

        // Positive joins, left-deep.
        let mut current: &Variable<Row> = &self.first.acc;
        for stage in &self.joins {
            let Some(atom_src) = vars.get(&stage.pred) else {
                return;
            };
            let (kp, rp) = (
                stage.split.key_positions.clone(),
                stage.split.rest_positions.clone(),
            );
            stage.acc_keyed.from_map(current, move |row: &Row| {
                (
                    kp.iter().map(|&p| row[p]).collect::<Row>(),
                    rp.iter().map(|&p| row[p]).collect::<Row>(),
                )
            });
            let (key_cols, new_cols) = (stage.atom_key_cols.clone(), stage.atom_new_cols.clone());
            let filters = stage.filters.clone();
            let eq = stage.eq;
            stage.atom_keyed.from_leapjoin(
                atom_src,
                PrefixFilter::from(move |&(c0, c1): &(i64, i64)| passes(&filters, eq, &[c0, c1])),
                move |&(c0, c1), &()| {
                    (
                        key_cols.iter().map(|&c| [c0, c1][c]).collect::<Row>(),
                        new_cols.iter().map(|&c| [c0, c1][c]).collect::<Row>(),
                    )
                },
            );
            let rebuild: Vec<RowSrc> = stage.split.rebuild.clone();
            stage.out.from_join(
                &stage.acc_keyed,
                &stage.atom_keyed,
                move |key: &Row, rest: &Row, new: &Row| {
                    let mut row: Row = rebuild
                        .iter()
                        .map(|s| match s {
                            RowSrc::Key(i) => key[*i],
                            RowSrc::Rest(i) => rest[*i],
                        })
                        .collect();
                    row.extend_from_slice(new);
                    row
                },
            );
            current = &stage.out;
        }

        // Stratified negation: keep rows whose negated-atom key is absent
        // from the (complete) lower-stratum relation.
        for stage in &self.negations {
            let (kp, rp) = (
                stage.split.key_positions.clone(),
                stage.split.rest_positions.clone(),
            );
            stage.keyed.from_map(current, move |row: &Row| {
                (
                    kp.iter().map(|&p| row[p]).collect::<Row>(),
                    rp.iter().map(|&p| row[p]).collect::<Row>(),
                )
            });
            let rebuild: Vec<RowSrc> = stage.split.rebuild.clone();
            stage.out.from_antijoin(
                &stage.keyed,
                &stage.relation,
                move |key: &Row, rest: &Row| {
                    rebuild
                        .iter()
                        .map(|s| match s {
                            RowSrc::Key(i) => key[*i],
                            RowSrc::Rest(i) => rest[*i],
                        })
                        .collect::<Row>()
                },
            );
            current = &stage.out;
        }

        // Project the head.
        let head_slots = self.head_slots.clone();
        head.from_map(current, move |row: &Row| {
            let slot = |s: &HeadSlot| -> i64 {
                match s {
                    HeadSlot::Constant(id) => *id,
                    HeadSlot::Pos(p) => row[*p],
                }
            };
            (slot(&head_slots[0]), slot(&head_slots[1]))
        });
    }
}

fn unsupported(rule: &Rule, feature: &str) -> ReasonerError {
    ReasonerError::Unsupported {
        id: rule.id.clone(),
        feature: feature.to_string(),
    }
}

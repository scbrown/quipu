//! Per-rule compilation onto datafrog.
//!
//! Each supported rule shape is turned into a small "plan" that knows how
//! to advance a datafrog [`Iteration`] by one step. Plans are built once
//! per stratum and executed inside `while iteration.changed()`.
//!
//! # Supported shapes
//!
//! - **1-atom**: `h(?a, ?b) :- p(?x, ?y)`. The head must reference variables
//!   drawn from the body; the body must use distinct variables.
//! - **2-atom**: `h(?a, ?b) :- p(?x, ?y), q(?z, ?w)` where the two body
//!   atoms share exactly one variable. Remaining variables must also be
//!   distinct within each atom.
//!
//! Anything else (3+ atoms, negation, unary predicates, repeated variables
//! inside a body atom, two-atom bodies without a shared variable) is rejected
//! up-front with [`ReasonerError::Unsupported`] so the evaluator never sees it.
//!
//! Constants in the **head** are allowed — they compile down to fixed slot
//! values — but must be IRIs that already exist in the term dictionary.
//!
//! Constants in a **body** atom are allowed too, and compile to a *selection*:
//! the column is pinned to that IRI's term id and non-matching tuples are
//! dropped before the head is projected (aegis-jgxas). This is what makes
//! `rdf:type(?x, <GitCommit>) :- rdf:type(?x, <Commit>)` — class equivalence
//! as a live Datalog rule — expressible.
//!
//! A body constant that has never been interned makes the rule *unsatisfiable*
//! rather than an error: no fact can reference a term that does not exist, so
//! the correct derivation is the empty set. That is deliberately NOT the head's
//! behaviour, where an unknown IRI means the rule could never write anything
//! meaningful and is a genuine authoring mistake.

use std::collections::BTreeMap;

use datafrog::{Iteration, PrefixFilter, Variable};

use super::ast::{Atom, BodyAtom, Rule, Term};
use super::{ReasonerError, Result};

/// A compiled datafrog plan for a single rule.
///
/// Holds references (cheap to clone — Variables are internally `Rc`) into
/// the [`Iteration`] that owns the underlying Variables. Call [`Plan::step`]
/// once per `while changed` tick.
pub(crate) struct Plan {
    shape: Shape,
    head_pred: String,
    /// Projection from body bindings to each of the head's two slots.
    head_plan: [HeadSlot; 2],
    /// A body constant referenced an IRI that has never been interned, so
    /// no fact can match and the rule derives the empty set. Kept as a plan
    /// (rather than an error) so the rest of the stratum still runs.
    unsatisfiable: bool,
}

/// A body-atom column pinned to a constant term id: `(column, term_id)`.
type Filter = (usize, i64);

/// A variable occupying a body-atom column: `(name, column)`.
type Binding = (String, usize);

/// A body atom split into the columns that bind and the columns that filter.
type SplitAtom = (Vec<Binding>, Vec<Filter>);

/// Does `row` satisfy every pinned column?
fn passes(filters: &[Filter], row: &[i64; 2]) -> bool {
    filters.iter().all(|&(col, id)| row[col] == id)
}

enum Shape {
    OneAtom(OneAtomPlan),
    // Boxed: a two-atom plan carries two datafrog Variables and four Vecs,
    // which would otherwise make every `Shape` as large as its widest arm.
    TwoAtom(Box<TwoAtomPlan>),
}

struct OneAtomPlan {
    input: String,
    /// Variable name → column index in the body atom. Holds one entry per
    /// column that carries a *variable*; a column carrying a constant
    /// contributes a `filters` entry instead. Names are distinct (a body
    /// like `p(?x, ?x)` is rejected at compile time).
    binding: Vec<Binding>,
    /// Columns pinned to a constant.
    filters: Vec<Filter>,
}

struct TwoAtomPlan {
    left_pred: String,
    right_pred: String,
    /// Column of the shared variable in the left atom.
    left_join_col: usize,
    /// Column of the shared variable in the right atom.
    right_join_col: usize,
    /// Name of the shared join variable.
    join_var: String,
    /// Variable at the left atom's non-join column, or `None` when that
    /// column carries a constant (in which case it is in `left_filters`).
    left_nonjoin_var: Option<String>,
    /// Variable at the right atom's non-join column, or `None` as above.
    right_nonjoin_var: Option<String>,
    /// Columns of the left atom pinned to a constant.
    left_filters: Vec<Filter>,
    /// Columns of the right atom pinned to a constant.
    right_filters: Vec<Filter>,
    /// Pre-allocated keyed view of the left atom — rebuilt each tick so
    /// incremental updates to `left_pred` flow through.
    left_keyed: Variable<(i64, i64)>,
    /// Pre-allocated keyed view of the right atom.
    right_keyed: Variable<(i64, i64)>,
}

/// Where one head slot's value comes from.
#[derive(Clone)]
enum HeadSlot {
    /// Literal IRI resolved to a term id at compile time.
    Constant(i64),
    /// Name of a body variable whose binding will be slotted here.
    Var(String),
}

/// Compile a rule against an iteration, allocating any helper variables
/// needed for 2-atom joins.
pub(crate) fn compile_rule(
    iteration: &mut Iteration,
    rule: &Rule,
    const_ids: &BTreeMap<String, i64>,
    vars: &BTreeMap<String, Variable<(i64, i64)>>,
) -> Result<Plan> {
    if rule.head.args.len() != 2 {
        return Err(unsupported(rule, "non-binary head atom"));
    }
    let body_atoms = positive_body(rule)?;
    for atom in &body_atoms {
        if atom.args.len() != 2 {
            return Err(unsupported(rule, "non-binary body atom"));
        }
        for term in &atom.args {
            if matches!(term, Term::Str(_)) {
                // Facts are `Value::Ref` triples; a literal cannot match one.
                return Err(unsupported(rule, "string constant in body atom"));
            }
        }
    }

    // Resolve every body constant up-front. A miss means "no such term
    // exists, so nothing can match" — recorded, not raised.
    let mut unsatisfiable = false;
    for atom in &body_atoms {
        for term in &atom.args {
            if let Term::Iri(iri) = term
                && !const_ids.contains_key(iri)
            {
                unsatisfiable = true;
            }
        }
    }

    let shape = match body_atoms.len() {
        1 => Shape::OneAtom(plan_one_atom(rule, body_atoms[0], const_ids)?),
        2 => Shape::TwoAtom(Box::new(plan_two_atom(
            iteration,
            rule,
            body_atoms[0],
            body_atoms[1],
            const_ids,
            vars,
        )?)),
        _ => return Err(unsupported(rule, "body with more than 2 atoms")),
    };

    let head_plan = [
        head_slot(rule, &rule.head.args[0], const_ids)?,
        head_slot(rule, &rule.head.args[1], const_ids)?,
    ];

    Ok(Plan {
        shape,
        head_pred: rule.head.predicate.clone(),
        head_plan,
        unsatisfiable,
    })
}

/// Split a body atom's two columns into variable bindings and constant
/// filters. Missing constants resolve to `i64::MIN` — never a real term id,
/// so the filter cannot match; the `unsatisfiable` flag is what actually
/// short-circuits, this is only belt-and-braces.
fn split_columns(rule: &Rule, atom: &Atom, const_ids: &BTreeMap<String, i64>) -> Result<SplitAtom> {
    let mut binding: Vec<Binding> = Vec::new();
    let mut filters: Vec<Filter> = Vec::new();
    for (col, term) in atom.args.iter().enumerate() {
        match term {
            Term::Var(name) => {
                if binding.iter().any(|(n, _)| n == name) {
                    return Err(unsupported(rule, "repeated variable in a body atom"));
                }
                binding.push((name.clone(), col));
            }
            Term::Iri(iri) => {
                filters.push((col, const_ids.get(iri).copied().unwrap_or(i64::MIN)));
            }
            Term::Str(_) => return Err(unsupported(rule, "string constant in body atom")),
        }
    }
    Ok((binding, filters))
}

fn positive_body(rule: &Rule) -> Result<Vec<&Atom>> {
    let mut out = Vec::with_capacity(rule.body.len());
    for b in &rule.body {
        match b {
            BodyAtom::Positive(a) => out.push(a),
            BodyAtom::Negative(_) => return Err(unsupported(rule, "negation-as-failure")),
        }
    }
    Ok(out)
}

fn plan_one_atom(
    rule: &Rule,
    atom: &Atom,
    const_ids: &BTreeMap<String, i64>,
) -> Result<OneAtomPlan> {
    let (binding, filters) = split_columns(rule, atom, const_ids)?;
    Ok(OneAtomPlan {
        input: atom.predicate.clone(),
        binding,
        filters,
    })
}

fn plan_two_atom(
    iteration: &mut Iteration,
    rule: &Rule,
    left: &Atom,
    right: &Atom,
    const_ids: &BTreeMap<String, i64>,
    vars: &BTreeMap<String, Variable<(i64, i64)>>,
) -> Result<TwoAtomPlan> {
    let (left_binding, left_filters) = split_columns(rule, left, const_ids)?;
    let (right_binding, right_filters) = split_columns(rule, right, const_ids)?;

    // Exactly one variable must be shared between the two atoms. Columns
    // carrying constants are selections, not join keys, so they take no
    // part in this search.
    let mut matches = Vec::new();
    for (lname, li) in &left_binding {
        for (rname, ri) in &right_binding {
            if lname == rname {
                matches.push((*li, *ri, lname.clone()));
            }
        }
    }
    if matches.len() != 1 {
        return Err(unsupported(
            rule,
            "two-atom body must share exactly one variable",
        ));
    }
    let (left_join_col, right_join_col, join_var) = matches.into_iter().next().unwrap();
    let nonjoin = |binding: &[Binding], join_col: usize| -> Option<String> {
        binding
            .iter()
            .find(|(_, col)| *col != join_col)
            .map(|(name, _)| name.clone())
    };
    let left_nonjoin_var = nonjoin(&left_binding, left_join_col);
    let right_nonjoin_var = nonjoin(&right_binding, right_join_col);

    if !vars.contains_key(&left.predicate) || !vars.contains_key(&right.predicate) {
        return Err(ReasonerError::Unsupported {
            id: rule.id.clone(),
            feature: format!(
                "body references predicate with no variable allocated: \
                 left={:?} right={:?}",
                left.predicate, right.predicate
            ),
        });
    }

    let left_keyed =
        iteration.variable::<(i64, i64)>(&format!("{}::left::{}", rule.id, left.predicate));
    let right_keyed =
        iteration.variable::<(i64, i64)>(&format!("{}::right::{}", rule.id, right.predicate));

    Ok(TwoAtomPlan {
        left_pred: left.predicate.clone(),
        right_pred: right.predicate.clone(),
        left_join_col,
        right_join_col,
        join_var,
        left_nonjoin_var,
        right_nonjoin_var,
        left_filters,
        right_filters,
        left_keyed,
        right_keyed,
    })
}

fn head_slot(rule: &Rule, term: &Term, const_ids: &BTreeMap<String, i64>) -> Result<HeadSlot> {
    match term {
        Term::Var(name) => Ok(HeadSlot::Var(name.clone())),
        Term::Iri(iri) => {
            let id = const_ids.get(iri).copied().ok_or_else(|| {
                unsupported(rule, "head references an IRI that has never been interned")
            })?;
            Ok(HeadSlot::Constant(id))
        }
        Term::Str(_) => Err(unsupported(rule, "string constant in head atom")),
    }
}

impl Plan {
    /// Advance this rule by one iteration tick, writing any new tuples
    /// into the head predicate's variable.
    pub(crate) fn step(&self, vars: &BTreeMap<String, Variable<(i64, i64)>>) {
        // A body constant naming an un-interned IRI can match nothing.
        if self.unsatisfiable {
            return;
        }
        let head = vars
            .get(&self.head_pred)
            .expect("head variable must have been allocated before compile");
        match &self.shape {
            Shape::OneAtom(p) => self.step_one_atom(head, p, vars),
            Shape::TwoAtom(p) => self.step_two_atom(head, p, vars),
        }
    }

    fn step_one_atom(
        &self,
        head: &Variable<(i64, i64)>,
        plan: &OneAtomPlan,
        vars: &BTreeMap<String, Variable<(i64, i64)>>,
    ) {
        let Some(input) = vars.get(&plan.input) else {
            return;
        };
        let head_plan = self.head_plan.clone();
        let binding = plan.binding.clone();
        if plan.filters.is_empty() {
            head.from_map(input, move |&(c0, c1)| {
                resolve_head(&head_plan, &binding, &[c0, c1])
            });
            return;
        }
        // Constants in the body are a selection: drop tuples whose pinned
        // columns disagree BEFORE projecting the head. Skipping this is the
        // aegis-jgxas over-derivation — every typed entity would match.
        let filters = plan.filters.clone();
        head.from_leapjoin(
            input,
            PrefixFilter::from(move |&(c0, c1): &(i64, i64)| passes(&filters, &[c0, c1])),
            move |&(c0, c1), &()| resolve_head(&head_plan, &binding, &[c0, c1]),
        );
    }

    fn step_two_atom(
        &self,
        head: &Variable<(i64, i64)>,
        plan: &TwoAtomPlan,
        vars: &BTreeMap<String, Variable<(i64, i64)>>,
    ) {
        let Some(left_src) = vars.get(&plan.left_pred) else {
            return;
        };
        let Some(right_src) = vars.get(&plan.right_pred) else {
            return;
        };

        // Apply each atom's constant selection while keying it for the join,
        // so non-matching tuples never reach `from_join`.
        key_filtered(
            &plan.left_keyed,
            left_src,
            plan.left_join_col,
            &plan.left_filters,
        );
        key_filtered(
            &plan.right_keyed,
            right_src,
            plan.right_join_col,
            &plan.right_filters,
        );

        let head_plan = self.head_plan.clone();
        let join_var = plan.join_var.clone();
        let left_nonjoin_var = plan.left_nonjoin_var.clone();
        let right_nonjoin_var = plan.right_nonjoin_var.clone();
        head.from_join(
            &plan.left_keyed,
            &plan.right_keyed,
            move |&key, &l_val, &r_val| {
                // A stable 3-slot row: [join_key, left_nonjoin, right_nonjoin].
                // A non-join column holding a constant contributes no binding.
                let mut binding: Vec<Binding> = vec![(join_var.clone(), 0_usize)];
                if let Some(v) = &left_nonjoin_var {
                    binding.push((v.clone(), 1));
                }
                if let Some(v) = &right_nonjoin_var {
                    binding.push((v.clone(), 2));
                }
                resolve_head(&head_plan, &binding, &[key, l_val, r_val])
            },
        );
    }
}

/// Rebuild `keyed` from `src` as `(join_column, other_column)`, dropping any
/// tuple that fails the atom's constant selection.
fn key_filtered(
    keyed: &Variable<(i64, i64)>,
    src: &Variable<(i64, i64)>,
    join_col: usize,
    filters: &[Filter],
) {
    let rekey = move |c0: i64, c1: i64| if join_col == 0 { (c0, c1) } else { (c1, c0) };
    if filters.is_empty() {
        keyed.from_map(src, move |&(c0, c1)| rekey(c0, c1));
        return;
    }
    let filters = filters.to_vec();
    keyed.from_leapjoin(
        src,
        PrefixFilter::from(move |&(c0, c1): &(i64, i64)| passes(&filters, &[c0, c1])),
        move |&(c0, c1), &()| rekey(c0, c1),
    );
}

fn resolve_head(head_plan: &[HeadSlot; 2], binding: &[(String, usize)], row: &[i64]) -> (i64, i64) {
    let lookup = |slot: &HeadSlot| -> i64 {
        match slot {
            HeadSlot::Constant(id) => *id,
            HeadSlot::Var(name) => binding
                .iter()
                .find(|(b, _)| b == name)
                .and_then(|(_, idx)| row.get(*idx).copied())
                .expect("range restriction ensures every head var is bound"),
        }
    };
    (lookup(&head_plan[0]), lookup(&head_plan[1]))
}

fn unsupported(rule: &Rule, feature: &str) -> ReasonerError {
    ReasonerError::Unsupported {
        id: rule.id.clone(),
        feature: feature.to_string(),
    }
}

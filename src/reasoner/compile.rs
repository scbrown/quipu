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
//! inside a body atom, constants in body atoms, two-atom bodies without a
//! shared variable) is rejected up-front with [`ReasonerError::Unsupported`]
//! so the evaluator never sees it.
//!
//! Constants in the **head** are allowed — they compile down to fixed slot
//! values — but must be IRIs that already exist in the term dictionary.

use std::collections::BTreeMap;

use datafrog::{Iteration, Variable};

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
}

enum Shape {
    OneAtom(OneAtomPlan),
    TwoAtom(TwoAtomPlan),
}

struct OneAtomPlan {
    input: String,
    /// Variable name → column index in the body atom. Always size 2 with
    /// distinct keys (a body like `p(?x, ?x)` is rejected at compile time).
    binding: [(String, usize); 2],
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
    /// Variable name at the left atom's non-join column.
    left_nonjoin_var: String,
    /// Variable name at the right atom's non-join column.
    right_nonjoin_var: String,
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
            if !matches!(term, Term::Var(_)) {
                return Err(unsupported(rule, "constant argument in body atom"));
            }
        }
    }

    let shape = match body_atoms.len() {
        1 => Shape::OneAtom(plan_one_atom(rule, body_atoms[0])?),
        2 => Shape::TwoAtom(plan_two_atom(
            iteration,
            rule,
            body_atoms[0],
            body_atoms[1],
            vars,
        )?),
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
    })
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

fn plan_one_atom(rule: &Rule, atom: &Atom) -> Result<OneAtomPlan> {
    let v0 = var_name(rule, &atom.args[0])?;
    let v1 = var_name(rule, &atom.args[1])?;
    if v0 == v1 {
        return Err(unsupported(rule, "repeated variable in a body atom"));
    }
    Ok(OneAtomPlan {
        input: atom.predicate.clone(),
        binding: [(v0.to_string(), 0), (v1.to_string(), 1)],
    })
}

fn plan_two_atom(
    iteration: &mut Iteration,
    rule: &Rule,
    left: &Atom,
    right: &Atom,
    vars: &BTreeMap<String, Variable<(i64, i64)>>,
) -> Result<TwoAtomPlan> {
    let l0 = var_name(rule, &left.args[0])?;
    let l1 = var_name(rule, &left.args[1])?;
    let r0 = var_name(rule, &right.args[0])?;
    let r1 = var_name(rule, &right.args[1])?;
    if l0 == l1 || r0 == r1 {
        return Err(unsupported(rule, "repeated variable inside a body atom"));
    }

    // Exactly one of {l0,l1} must match exactly one of {r0,r1}.
    let mut matches = Vec::new();
    for (li, lname) in [l0, l1].iter().enumerate() {
        for (ri, rname) in [r0, r1].iter().enumerate() {
            if lname == rname {
                matches.push((li, ri, (*lname).to_string()));
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
    let left_nonjoin_var = if left_join_col == 0 { l1 } else { l0 }.to_string();
    let right_nonjoin_var = if right_join_col == 0 { r1 } else { r0 }.to_string();

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
        left_keyed,
        right_keyed,
    })
}

fn var_name<'a>(rule: &Rule, term: &'a Term) -> Result<&'a str> {
    match term {
        Term::Var(name) => Ok(name.as_str()),
        Term::Iri(_) | Term::Str(_) => Err(unsupported(rule, "constant argument in body atom")),
    }
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
        head.from_map(input, move |&(c0, c1)| {
            resolve_head(&head_plan, &binding, &[c0, c1])
        });
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

        let left_join_col = plan.left_join_col;
        plan.left_keyed.from_map(left_src, move |&(c0, c1)| {
            if left_join_col == 0 {
                (c0, c1)
            } else {
                (c1, c0)
            }
        });
        let right_join_col = plan.right_join_col;
        plan.right_keyed.from_map(right_src, move |&(c0, c1)| {
            if right_join_col == 0 {
                (c0, c1)
            } else {
                (c1, c0)
            }
        });

        let head_plan = self.head_plan.clone();
        let join_var = plan.join_var.clone();
        let left_nonjoin_var = plan.left_nonjoin_var.clone();
        let right_nonjoin_var = plan.right_nonjoin_var.clone();
        head.from_join(
            &plan.left_keyed,
            &plan.right_keyed,
            move |&key, &l_val, &r_val| {
                // A stable 3-slot row: [join_key, left_nonjoin, right_nonjoin].
                let binding = [
                    (join_var.clone(), 0_usize),
                    (left_nonjoin_var.clone(), 1),
                    (right_nonjoin_var.clone(), 2),
                ];
                resolve_head(&head_plan, &binding, &[key, l_val, r_val])
            },
        );
    }
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

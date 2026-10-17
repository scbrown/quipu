//! Streaming historical-only fixed point; never materialize a join product.
use std::collections::{BTreeMap, BTreeSet};

use super::ast::{Atom, Rule, Term};
use super::historical::{HistoricalError, HistoricalFact};
use super::{ReasonerError, RuleSet};

type Pair = (String, String);
type World = BTreeMap<String, BTreeSet<Pair>>;
type Bindings = BTreeMap<String, String>;

struct Budget {
    remaining: usize,
    output_cap: usize,
    deadline: Option<crate::time::Deadline>,
}

impl Budget {
    fn charge(&mut self) -> Result<(), HistoricalError> {
        if let Some(deadline) = self.deadline
            && deadline.passed()
        {
            return Err(deadline.request_timeout().into());
        }
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(HistoricalError::Capacity)?;
        Ok(())
    }
}

fn unsupported(rule: &Rule) -> HistoricalError {
    ReasonerError::Unsupported {
        id: rule.id.clone(),
        feature: "historical mode requires safe binary IRI rules with positive premises".into(),
    }
    .into()
}

fn validate(rule: &Rule) -> Result<(), HistoricalError> {
    let vars: BTreeSet<_> = rule.body_vars().into_iter().collect();
    if rule.body.is_empty()
        || !rule.body.iter().any(super::BodyAtom::is_positive)
        || std::iter::once(&rule.head)
            .chain(rule.body.iter().map(super::BodyAtom::atom))
            .any(|a| a.args.len() != 2 || a.args.iter().any(|t| matches!(t, Term::Str(_))))
        || rule.head.vars().any(|v| !vars.contains(v))
        || rule
            .body
            .iter()
            .filter(|a| !a.is_positive())
            .any(|a| a.atom().vars().any(|v| !vars.contains(v)))
    {
        return Err(unsupported(rule));
    }
    Ok(())
}

fn bind(atom: &Atom, pair: &Pair, bindings: &Bindings) -> Option<Bindings> {
    let mut next = bindings.clone();
    for (term, value) in atom.args.iter().zip([&pair.0, &pair.1]) {
        match term {
            Term::Iri(iri) if iri != value => return None,
            Term::Var(name) => {
                if let Some(old) = next.get(name) {
                    if old != value {
                        return None;
                    }
                } else {
                    next.insert(name.clone(), value.clone());
                }
            }
            Term::Str(_) => return None,
            Term::Iri(_) => {}
        }
    }
    Some(next)
}

fn term_value(term: &Term, bindings: &Bindings) -> String {
    match term {
        Term::Iri(iri) => iri.clone(),
        Term::Var(name) => bindings[name].clone(),
        Term::Str(_) => unreachable!("validated IRI rules"),
    }
}

// One stack frame per positive atom; all candidate scans, including failures
// and negation, spend the same call-wide budget. No Vec of joined rows exists.
fn visit(
    rule: &Rule,
    atoms: &[&Atom],
    world: &World,
    bindings: &Bindings,
    pending: &mut BTreeSet<Pair>,
    budget: &mut Budget,
) -> Result<(), HistoricalError> {
    budget.charge()?;
    if let Some((atom, rest)) = atoms.split_first() {
        if let Some(pairs) = world.get(&atom.predicate) {
            for pair in pairs {
                budget.charge()?;
                if let Some(next) = bind(atom, pair, bindings) {
                    visit(rule, rest, world, &next, pending, budget)?;
                }
            }
        }
        return Ok(());
    }
    for negative in rule.body.iter().filter(|a| !a.is_positive()) {
        let atom = negative.atom();
        if let Some(pairs) = world.get(&atom.predicate) {
            for pair in pairs {
                budget.charge()?;
                if bind(atom, pair, bindings).is_some() {
                    return Ok(());
                }
            }
        }
    }
    let pair = (
        term_value(&rule.head.args[0], bindings),
        term_value(&rule.head.args[1], bindings),
    );
    if !pending.contains(&pair) {
        // Check BEFORE insertion: no sentinel-sized derived relation is built.
        if pending.len() >= budget.output_cap.saturating_sub(1) {
            return Err(HistoricalError::Capacity);
        }
        pending.insert(pair);
    }
    Ok(())
}

pub(super) fn evaluate(
    rules: &RuleSet,
    premises: Vec<HistoricalFact>,
    output_cap: usize,
    work_cap: usize,
    deadline: Option<crate::time::Deadline>,
) -> Result<Vec<HistoricalFact>, HistoricalError> {
    // Bound AST traversal, stack depth and per-binding width before evaluation.
    if rules.rules.len() > 64 || rules.rules.iter().any(|r| r.body.len() > 64) {
        return Err(HistoricalError::Capacity);
    }
    let known: BTreeSet<_> = premises
        .iter()
        .flat_map(|f| [&f.subject, &f.predicate, &f.object])
        .collect();
    for rule in &rules.rules {
        validate(rule)?;
        if rule
            .head
            .args
            .iter()
            .any(|t| matches!(t, Term::Iri(iri) if !known.contains(iri)))
        {
            return Err(ReasonerError::Unsupported {
                id: rule.id.clone(),
                feature: "head references an IRI that has never been interned".into(),
            }
            .into());
        }
    }
    let strata = super::stratify::stratify(rules)?;
    let mut world: World = BTreeMap::new();
    for fact in premises {
        world
            .entry(fact.predicate)
            .or_default()
            .insert((fact.subject, fact.object));
    }
    let mut answer = BTreeSet::new();
    let mut budget = Budget {
        remaining: work_cap,
        output_cap,
        deadline,
    };
    for level in strata.levels {
        loop {
            let mut changed = false;
            for index in &level {
                budget.charge()?;
                let rule = &rules.rules[*index];
                let atoms: Vec<_> = rule
                    .body
                    .iter()
                    .filter(|a| a.is_positive())
                    .map(super::BodyAtom::atom)
                    .collect();
                let mut pending = BTreeSet::new();
                visit(
                    rule,
                    &atoms,
                    &world,
                    &Bindings::new(),
                    &mut pending,
                    &mut budget,
                )?;
                for (subject, object) in pending {
                    budget.charge()?;
                    let fact = HistoricalFact {
                        subject: subject.clone(),
                        predicate: rule.head.predicate.clone(),
                        object: object.clone(),
                    };
                    if !answer.contains(&fact) {
                        if answer.len() >= output_cap.saturating_sub(1) {
                            return Err(HistoricalError::Capacity);
                        }
                        answer.insert(fact);
                    }
                    changed |= world
                        .entry(rule.head.predicate.clone())
                        .or_default()
                        .insert((subject, object));
                }
            }
            if !changed {
                break;
            }
        }
    }
    budget.charge()?;
    Ok(answer.into_iter().collect())
}

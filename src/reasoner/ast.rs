//! Abstract syntax for reasoner rules.
//!
//! The AST is small on purpose: a rule is a Horn clause with a positional
//! head atom and a list of body literals. Everything else — IRI resolution,
//! stratification, compilation to datafrog joins — operates on this shape.

/// A term appearing in an atom's argument list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    /// A variable, stored without the leading `?`.
    Var(String),
    /// A constant IRI in full form.
    Iri(String),
    /// A string literal constant.
    Str(String),
}

impl Term {
    /// Return the variable name if this term is a variable.
    pub fn as_var(&self) -> Option<&str> {
        match self {
            Self::Var(name) => Some(name.as_str()),
            _ => None,
        }
    }
}

/// A positive predicate application: `pred(arg1, arg2, ...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// Full predicate IRI (after prefix expansion at parse time).
    pub predicate: String,
    /// Positional arguments.
    pub args: Vec<Term>,
}

impl Atom {
    /// Iterate the variable names used in this atom, in argument order.
    /// Duplicate variables (e.g. `p(?x, ?x)`) appear twice.
    pub fn vars(&self) -> impl Iterator<Item = &str> {
        self.args.iter().filter_map(Term::as_var)
    }
}

/// A body literal: either a positive atom or a negated one.
///
/// Negation is parsed and preserved in the AST so the stratifier can reason
/// about it, but the Phase 2 evaluator rejects negated atoms at eval time
/// with a clear error. Negation support lands with the rest of the open
/// questions (see design doc Q6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyAtom {
    /// Positive body literal.
    Positive(Atom),
    /// Negated body literal (reserved for future NAF support).
    Negative(Atom),
}

impl BodyAtom {
    /// Return the underlying atom regardless of polarity.
    pub fn atom(&self) -> &Atom {
        match self {
            Self::Positive(a) | Self::Negative(a) => a,
        }
    }

    /// True when this literal is positive.
    pub fn is_positive(&self) -> bool {
        matches!(self, Self::Positive(_))
    }
}

/// A single Horn clause rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// Stable id for provenance. Typically `R1`, `R2`, ... but any string
    /// the ruleset author chose via `rule:id`.
    pub id: String,
    /// The head that is derived.
    pub head: Atom,
    /// The body literals that must all hold.
    pub body: Vec<BodyAtom>,
}

impl Rule {
    /// Unique variables in head argument order, first appearance only.
    pub fn head_vars(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for v in self.head.vars() {
            if !seen.contains(&v) {
                seen.push(v);
            }
        }
        seen
    }

    /// Unique predicates referenced by positive body atoms.
    pub fn positive_body_predicates(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for b in &self.body {
            if let BodyAtom::Positive(a) = b
                && !seen.contains(&a.predicate.as_str())
            {
                seen.push(a.predicate.as_str());
            }
        }
        seen
    }

    /// Unique predicates referenced by negated body atoms.
    pub fn negated_body_predicates(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for b in &self.body {
            if let BodyAtom::Negative(a) = b
                && !seen.contains(&a.predicate.as_str())
            {
                seen.push(a.predicate.as_str());
            }
        }
        seen
    }

    /// Collect every variable mentioned anywhere in the body (positive
    /// literals only — a variable that appears *only* under negation is
    /// not a safe range restriction).
    pub fn body_vars(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for b in &self.body {
            if let BodyAtom::Positive(a) = b {
                for v in a.vars() {
                    if !seen.contains(&v) {
                        seen.push(v);
                    }
                }
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str) -> Term {
        Term::Var(name.to_string())
    }

    fn atom(pred: &str, args: Vec<Term>) -> Atom {
        Atom {
            predicate: pred.to_string(),
            args,
        }
    }

    #[test]
    fn head_vars_deduped_in_order() {
        let rule = Rule {
            id: "R1".into(),
            head: atom("http://ex/h", vec![var("x"), var("y"), var("x")]),
            body: vec![BodyAtom::Positive(atom(
                "http://ex/p",
                vec![var("x"), var("y")],
            ))],
        };
        assert_eq!(rule.head_vars(), vec!["x", "y"]);
    }

    #[test]
    fn positive_and_negated_predicates_are_separated() {
        let rule = Rule {
            id: "R1".into(),
            head: atom("http://ex/h", vec![var("x")]),
            body: vec![
                BodyAtom::Positive(atom("http://ex/p", vec![var("x")])),
                BodyAtom::Positive(atom("http://ex/q", vec![var("x")])),
                BodyAtom::Negative(atom("http://ex/r", vec![var("x")])),
            ],
        };
        assert_eq!(
            rule.positive_body_predicates(),
            vec!["http://ex/p", "http://ex/q"]
        );
        assert_eq!(rule.negated_body_predicates(), vec!["http://ex/r"]);
    }

    #[test]
    fn body_vars_only_count_positive_literals() {
        // `y` appears only under negation — it is not a safe range
        // restriction for the head and should not be exposed via body_vars.
        let rule = Rule {
            id: "R1".into(),
            head: atom("http://ex/h", vec![var("x")]),
            body: vec![
                BodyAtom::Positive(atom("http://ex/p", vec![var("x")])),
                BodyAtom::Negative(atom("http://ex/q", vec![var("y")])),
            ],
        };
        assert_eq!(rule.body_vars(), vec!["x"]);
    }
}

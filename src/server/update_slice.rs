//! Which stored facts a SPARQL Update can observe or change (aegis-jm1lcl).
//!
//! `/update` evaluates with Oxigraph over a scratch dataset and diffs that
//! dataset before and after. Copying the WHOLE store into the scratch dataset
//! made every write cost O(store) while holding the writer lock. This module
//! reads the parsed update and names the slice that is enough: for each
//! predicate an operation mentions, either every subject or a fixed set of
//! constant subject IRIs. The caller copies that slice, from EVERY graph, and
//! runs the unchanged evaluate-and-diff over it.
//!
//! # Why the slice gives the same result
//!
//! Evaluating a graph pattern depends only on the quads its triple patterns can
//! match: a triple pattern with a constant predicate `p` and constant subject
//! `s` matches only quads `(s, p, *, g)`, and with a variable subject only
//! quads `(*, p, *, g)`. Every other operator (join, optional, union, minus,
//! filter, bind, values, grouping, ordering, slicing, `EXISTS`) is a function
//! of its operands' solutions. So a dataset holding every quad any pattern can
//! match yields the same solutions as the full store.
//!
//! Templates are part of the slice as well. A `DELETE` quad changes the diff
//! only if it was present before, and an `INSERT` of a quad that already exists
//! must find it present so it is not re-asserted; both are quads with a
//! template's predicate and subject, which the slice holds. Every quad an
//! operation writes is therefore inside the slice, so by induction over a
//! `;`-separated sequence each later operation also sees exactly the full
//! store's state restricted to the slice, and the before/after diff (and so
//! the transacted datums) is identical.
//!
//! Object constants are deliberately ignored: they only narrow, and literal
//! lexical-form equivalences make narrowing on them easy to get wrong.
//! `USING`, `USING NAMED` and `WITH` only choose which graphs a pattern reads;
//! the slice is taken from every graph, so they need no special handling.
//!
//! # When the argument fails, and the update takes the full-copy path
//!
//! * a variable predicate, in a pattern or a template;
//! * a property path with a zero-length step (`*`, `?`) — it matches every
//!   node of the graph — or a negated property set (`!p`), which matches edges
//!   of predicates the update never names;
//! * `GRAPH <g>` / `GRAPH ?g` around a pattern that can produce a solution
//!   without matching any triple (empty group, `OPTIONAL`/`BIND`/`VALUES`/
//!   `FILTER`-only, an aggregate): its result depends on which named graphs
//!   exist, and a slice drops graphs with no touched facts;
//! * `SERVICE`, `LATERAL`, and RDF-star triple terms;
//! * `LOAD`, `CLEAR`, `CREATE` and `DROP` (and the `ADD`/`MOVE`/`COPY` forms
//!   that desugar into them or into variable-predicate patterns);
//! * an update the parser here cannot read, or any construct not listed above.

use std::collections::{BTreeMap, BTreeSet};

use spargebra::{
    GraphUpdateOperation, SparqlParser,
    algebra::{
        AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
    },
    term::{GroundTermPattern, NamedNode, NamedNodePattern, TermPattern, TriplePattern},
};

/// The subjects of one predicate an update can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Subjects {
    /// Every subject: some pattern leaves the subject open.
    All,
    /// Only these constant subject IRIs.
    Only(BTreeSet<String>),
}

/// How `/update` must build its evaluation dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Copy only these predicates (by IRI), each for the given subjects.
    Sliced(BTreeMap<String, Subjects>),
    /// Copy the whole store; the string says which construct forced it.
    Full(&'static str),
}

type Walk = Result<(), &'static str>;

/// Decide the dataset an update needs. `Plan::Full` is always safe.
pub(crate) fn plan(update: &str) -> Plan {
    let Ok(parsed) = SparqlParser::new().parse_update(update) else {
        return Plan::Full("unparsed");
    };
    let mut slice = Slice::default();
    for operation in &parsed.operations {
        if let Err(reason) = slice.operation(operation) {
            return Plan::Full(reason);
        }
    }
    Plan::Sliced(slice.touched)
}

#[derive(Default)]
struct Slice {
    touched: BTreeMap<String, Subjects>,
}

impl Slice {
    fn record(&mut self, predicate: &NamedNode, subject: Option<&NamedNode>) {
        let entry = self
            .touched
            .entry(predicate.as_str().to_owned())
            .or_insert_with(|| Subjects::Only(BTreeSet::new()));
        match (entry, subject) {
            (Subjects::Only(set), Some(subject)) => {
                set.insert(subject.as_str().to_owned());
            }
            (entry, None) => *entry = Subjects::All,
            (Subjects::All, Some(_)) => {}
        }
    }

    fn operation(&mut self, operation: &GraphUpdateOperation) -> Walk {
        match operation {
            GraphUpdateOperation::InsertData { data } => {
                for quad in data {
                    let subject = match &quad.subject {
                        spargebra::term::NamedOrBlankNode::NamedNode(node) => Some(node),
                        // A blank node here is fresh on insert; keep the whole
                        // predicate so any label collision is decided exactly as
                        // the full path decides it.
                        spargebra::term::NamedOrBlankNode::BlankNode(_) => None,
                    };
                    if !matches!(
                        quad.object,
                        spargebra::term::Term::NamedNode(_)
                            | spargebra::term::Term::BlankNode(_)
                            | spargebra::term::Term::Literal(_)
                    ) {
                        return Err("rdf-star term");
                    }
                    self.record(&quad.predicate, subject);
                }
                Ok(())
            }
            GraphUpdateOperation::DeleteData { data } => {
                for quad in data {
                    if !matches!(
                        quad.object,
                        spargebra::term::GroundTerm::NamedNode(_)
                            | spargebra::term::GroundTerm::Literal(_)
                    ) {
                        return Err("rdf-star term");
                    }
                    self.record(&quad.predicate, Some(&quad.subject));
                }
                Ok(())
            }
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                using: _,
                pattern,
            } => {
                for quad in delete {
                    let subject = ground_subject(&quad.subject)?;
                    ground_object(&quad.object)?;
                    self.record(named_predicate(&quad.predicate)?, subject);
                }
                for quad in insert {
                    let subject = term_subject(&quad.subject)?;
                    term_object(&quad.object)?;
                    self.record(named_predicate(&quad.predicate)?, subject);
                }
                self.pattern(pattern)
            }
            GraphUpdateOperation::Load { .. } => Err("load"),
            GraphUpdateOperation::Clear { .. } => Err("clear"),
            GraphUpdateOperation::Create { .. } => Err("create"),
            GraphUpdateOperation::Drop { .. } => Err("drop"),
        }
    }

    fn triple(&mut self, triple: &TriplePattern) -> Walk {
        let subject = term_subject(&triple.subject)?;
        term_object(&triple.object)?;
        self.record(named_predicate(&triple.predicate)?, subject);
        Ok(())
    }

    fn pattern(&mut self, pattern: &GraphPattern) -> Walk {
        #[allow(unreachable_patterns, clippy::match_wildcard_for_single_variants)]
        match pattern {
            GraphPattern::Bgp { patterns } => patterns.iter().try_for_each(|t| self.triple(t)),
            GraphPattern::Path {
                subject,
                path,
                object,
            } => {
                term_object(subject)?;
                term_object(object)?;
                let mut predicates = Vec::new();
                path_predicates(path, &mut predicates)?;
                // A plain predicate keeps its constant subject; any real path
                // walks through intermediate nodes, so it reads every subject.
                let subject = match path {
                    PropertyPathExpression::NamedNode(_) => term_subject(subject)?,
                    _ => None,
                };
                for predicate in predicates {
                    self.record(predicate, subject);
                }
                Ok(())
            }
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => {
                self.pattern(left)?;
                self.pattern(right)
            }
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            } => {
                self.pattern(left)?;
                self.pattern(right)?;
                expression.as_ref().map_or(Ok(()), |e| self.expression(e))
            }
            GraphPattern::Filter { expr, inner } => {
                self.expression(expr)?;
                self.pattern(inner)
            }
            GraphPattern::Graph { name: _, inner } => {
                if !requires_match(inner) {
                    return Err("graph-existence");
                }
                self.pattern(inner)
            }
            GraphPattern::Extend {
                inner, expression, ..
            } => {
                self.expression(expression)?;
                self.pattern(inner)
            }
            GraphPattern::Values { .. } => Ok(()),
            GraphPattern::OrderBy { inner, expression } => {
                for order in expression {
                    match order {
                        OrderExpression::Asc(e) | OrderExpression::Desc(e) => self.expression(e)?,
                    }
                }
                self.pattern(inner)
            }
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. } => self.pattern(inner),
            GraphPattern::Group {
                inner, aggregates, ..
            } => {
                for (_, aggregate) in aggregates {
                    match aggregate {
                        AggregateExpression::CountSolutions { .. } => {}
                        AggregateExpression::FunctionCall { expr, .. } => self.expression(expr)?,
                    }
                }
                self.pattern(inner)
            }
            GraphPattern::Service { .. } => Err("service"),
            // LATERAL (sep-0006) and anything a later spargebra adds.
            _ => Err("unsupported-pattern"),
        }
    }

    fn expression(&mut self, expression: &Expression) -> Walk {
        #[allow(unreachable_patterns, clippy::match_wildcard_for_single_variants)]
        match expression {
            Expression::NamedNode(_)
            | Expression::Literal(_)
            | Expression::Variable(_)
            | Expression::Bound(_) => Ok(()),
            Expression::Or(a, b)
            | Expression::And(a, b)
            | Expression::Equal(a, b)
            | Expression::SameTerm(a, b)
            | Expression::Greater(a, b)
            | Expression::GreaterOrEqual(a, b)
            | Expression::Less(a, b)
            | Expression::LessOrEqual(a, b)
            | Expression::Add(a, b)
            | Expression::Subtract(a, b)
            | Expression::Multiply(a, b)
            | Expression::Divide(a, b) => {
                self.expression(a)?;
                self.expression(b)
            }
            Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
                self.expression(a)
            }
            Expression::In(a, list) => {
                self.expression(a)?;
                list.iter().try_for_each(|e| self.expression(e))
            }
            Expression::If(a, b, c) => {
                self.expression(a)?;
                self.expression(b)?;
                self.expression(c)
            }
            Expression::Coalesce(list) | Expression::FunctionCall(_, list) => {
                list.iter().try_for_each(|e| self.expression(e))
            }
            Expression::Exists(pattern) => self.pattern(pattern),
            _ => Err("unsupported-expression"),
        }
    }
}

fn named_predicate(predicate: &NamedNodePattern) -> Result<&NamedNode, &'static str> {
    match predicate {
        NamedNodePattern::NamedNode(node) => Ok(node),
        NamedNodePattern::Variable(_) => Err("variable-predicate"),
    }
}

/// The constant subject IRI of a pattern, or `None` when it is open.
fn term_subject(term: &TermPattern) -> Result<Option<&NamedNode>, &'static str> {
    term_object(term)?;
    Ok(match term {
        TermPattern::NamedNode(node) => Some(node),
        _ => None,
    })
}

fn term_object(term: &TermPattern) -> Walk {
    #[allow(unreachable_patterns, clippy::match_wildcard_for_single_variants)]
    match term {
        TermPattern::NamedNode(_)
        | TermPattern::BlankNode(_)
        | TermPattern::Literal(_)
        | TermPattern::Variable(_) => Ok(()),
        _ => Err("rdf-star term"),
    }
}

fn ground_subject(term: &GroundTermPattern) -> Result<Option<&NamedNode>, &'static str> {
    ground_object(term)?;
    Ok(match term {
        GroundTermPattern::NamedNode(node) => Some(node),
        _ => None,
    })
}

fn ground_object(term: &GroundTermPattern) -> Walk {
    #[allow(unreachable_patterns, clippy::match_wildcard_for_single_variants)]
    match term {
        GroundTermPattern::NamedNode(_)
        | GroundTermPattern::Literal(_)
        | GroundTermPattern::Variable(_) => Ok(()),
        _ => Err("rdf-star term"),
    }
}

/// Every predicate a path can traverse. A path that can match without
/// traversing a named edge, or that matches unnamed predicates, is refused.
fn path_predicates<'a>(path: &'a PropertyPathExpression, out: &mut Vec<&'a NamedNode>) -> Walk {
    match path {
        PropertyPathExpression::NamedNode(node) => {
            out.push(node);
            Ok(())
        }
        PropertyPathExpression::Reverse(inner) | PropertyPathExpression::OneOrMore(inner) => {
            path_predicates(inner, out)
        }
        PropertyPathExpression::Sequence(a, b) | PropertyPathExpression::Alternative(a, b) => {
            path_predicates(a, out)?;
            path_predicates(b, out)
        }
        PropertyPathExpression::ZeroOrMore(_) | PropertyPathExpression::ZeroOrOne(_) => {
            Err("zero-length-path")
        }
        PropertyPathExpression::NegatedPropertySet(_) => Err("negated-property-set"),
    }
}

/// True when every solution of `pattern` must have matched at least one
/// triple of the ACTIVE graph. `GRAPH` around a pattern for which this is false
/// can answer from graph existence alone, which a slice does not preserve.
fn requires_match(pattern: &GraphPattern) -> bool {
    #[allow(clippy::match_same_arms)]
    match pattern {
        GraphPattern::Bgp { patterns } => !patterns.is_empty(),
        // Zero-length and negated paths are refused before this matters.
        GraphPattern::Path { .. } => true,
        GraphPattern::Join { left, right } => requires_match(left) || requires_match(right),
        GraphPattern::Union { left, right } => requires_match(left) && requires_match(right),
        GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
            requires_match(left)
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => requires_match(inner),
        // A nested GRAPH changes the active graph; an aggregate yields a row on
        // empty input; VALUES and SERVICE match nothing locally.
        _ => false,
    }
}

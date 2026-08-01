//! `VALUES` — inline relation evaluation (quipu #51).
//!
//! `VALUES` is a literal table of solutions written into the query. SPARQL
//! defines it as a multiset of solution mappings that joins with the rest of
//! the group, so it is evaluated as a leaf (like a BGP): each declared row is
//! merged with the seed bindings, and rows that conflict with the seed drop
//! out. Everything else — the join with a neighbouring BGP, in either order —
//! falls out of the ordinary `Join` arm.

use spargebra::term::{GroundTerm, Variable};

use crate::error::Result;
use crate::store::Store;
use crate::types::Value;

use super::Bindings;

/// Evaluate a `VALUES` block into rows plus the variables it declares.
///
/// Semantics that are easy to get wrong, and are covered by tests:
///
/// - `UNDEF` leaves the variable *unbound* in that row rather than binding it
///   to a sentinel — a later `OPTIONAL`/`BOUND` must still see it as unbound.
/// - An empty table (`VALUES ?x {}`) yields **zero** rows, not every row. A
///   `VALUES` that constrains nothing would be a silent no-op filter (cf. #12),
///   which is exactly the failure mode this repo treats as worse than an error.
/// - The declared variables are returned even when the table is empty, so
///   projection still knows about them.
pub fn eval_values(
    store: &Store,
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
    seed: &Bindings,
) -> Result<(Vec<Bindings>, Vec<String>)> {
    let vars: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();

    let mut rows = Vec::with_capacity(bindings.len());
    'row: for binding in bindings {
        let mut row = seed.clone();
        for (var, term) in vars.iter().zip(binding) {
            // UNDEF: no binding for this variable in this row.
            let Some(term) = term else { continue };
            let value = ground_term_to_value(store, term)?;
            // A row that disagrees with the seed (EXISTS substitution, or a
            // value already fixed by an enclosing pattern) is not a solution.
            match row.get(var) {
                Some(existing) if existing != &value => continue 'row,
                Some(_) => {}
                None => {
                    row.insert(var.clone(), value);
                }
            }
        }
        rows.push(row);
    }

    Ok((rows, vars))
}

/// Convert a `VALUES` ground term into the store's `Value` representation.
///
/// An IRI resolves to the interned `Ref` the fact log uses, so it joins with a
/// BGP binding for the same IRI. An IRI absent from the dictionary keeps its
/// lexical form (`Str`) — the same fallback the BGP subject binding uses. That
/// matters: a sentinel id would make two *different* unknown IRIs compare
/// equal, while `Str` stays distinct and still matches no fact.
fn ground_term_to_value(store: &Store, term: &GroundTerm) -> Result<Value> {
    Ok(match term {
        GroundTerm::NamedNode(n) => store
            .lookup(n.as_str())?
            .map_or_else(|| Value::Str(n.as_str().to_string()), Value::Ref),
        GroundTerm::Literal(lit) => super::filter::literal_to_value(lit),
        // Quoted triples (RDF-star) are not stored as values, so one can never
        // match. Bind the lexical form rather than erroring: the row is then
        // simply unsatisfiable, which is what the query asked for.
        #[cfg(feature = "shacl")]
        GroundTerm::Triple(t) => Value::Str(format!("{t}")),
    })
}

//! COUNT-only grouping without retaining input solutions (aegis-skj0lv PR2).
//! Unsupported algebra/aggregates keep the ordinary exact evaluator.
use super::filter::{eval_expr, eval_filter};
use super::{Bindings, GraphScope, TemporalContext};
use crate::types::Value;
use crate::{Result, Store};

thread_local! {
    static DIRECT_REFS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Scoped to an applicable COUNT, including its correlated EXISTS reads.
pub(super) fn direct_refs() -> bool {
    DIRECT_REFS.with(std::cell::Cell::get)
}
struct DirectRefs(bool);
impl DirectRefs {
    fn install() -> Self {
        Self(DIRECT_REFS.with(|slot| slot.replace(true)))
    }
}
impl Drop for DirectRefs {
    fn drop(&mut self) {
        DIRECT_REFS.with(|slot| slot.set(self.0));
    }
}
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern,
};
use spargebra::term::{NamedNodePattern, TermPattern, Variable};
use std::collections::HashMap;

type Output = (Vec<Bindings>, Vec<String>);
type Emit<'a> = &'a mut dyn FnMut(Bindings) -> Result<()>;

fn supported(pattern: &GraphPattern) -> bool {
    match pattern {
        GraphPattern::Bgp { .. } => true,
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(_),
            inner,
        }
        | GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. } => supported(inner),
        GraphPattern::Union { left, right } => supported(left) && supported(right),
        // Seeding is equivalent to compatibility joining only for positive
        // BGP/GRAPH/JOIN operands, not for BIND, OPTIONAL or local filters.
        GraphPattern::Join { left, right } => {
            supported(left) && super::bind_join::seed_safe(right) && supported(right)
        }
        _ => false,
    }
}

fn graph_context(store: &Store, iri: &str, ctx: &TemporalContext) -> Result<TemporalContext> {
    let mut ids = store.lookup_all(iri)?;
    if let Some(visible) = &ctx.named_dataset {
        ids.retain(|id| visible.contains(id));
    }
    if ids.is_empty() {
        ids.push(-1);
    }
    Ok(TemporalContext {
        graph: GraphScope::Named(ids),
        ..ctx.clone()
    })
}

fn bgp(
    store: &Store,
    patterns: &[spargebra::term::TriplePattern],
    ctx: &TemporalContext,
    seed: &Bindings,
    emit: Emit<'_>,
) -> Result<()> {
    // Hash-join planning can change first-seen group order. Preserve that
    // evaluator where it applies; the captured typed metadata BGPs use SQL.
    if patterns.len() >= 2
        && !patterns.iter().any(super::rdfs::is_rdf_type_pattern)
        && crate::store::read_model::read_model_applicable(store, ctx)
    {
        for row in super::triple::eval_bgp(store, patterns, ctx, seed)?.0 {
            emit(row)?;
        }
        return Ok(());
    }
    let mut counts = vec![0; patterns.len()];
    bgp_sql(store, patterns, ctx, seed, emit, &mut counts)
}

fn bgp_sql(
    store: &Store,
    patterns: &[spargebra::term::TriplePattern],
    ctx: &TemporalContext,
    seed: &Bindings,
    emit: Emit<'_>,
    counts: &mut [usize],
) -> Result<()> {
    if let Some((first, rest)) = patterns.split_first() {
        let (count, remaining) = counts.split_first_mut().expect("one counter per pattern");
        super::triple::visit_triple_pattern(store, first, seed, ctx, &mut |row| {
            super::pattern_util::check_eval_budget(ctx, *count, *count)?;
            *count += 1;
            bgp_sql(store, rest, ctx, &row, emit, remaining)
        })?;
        Ok(())
    } else {
        emit(seed.clone())
    }
}

fn visit(
    store: &Store,
    pattern: &GraphPattern,
    ctx: &TemporalContext,
    seed: &Bindings,
    emit: Emit<'_>,
) -> Result<()> {
    super::pattern_util::check_eval_budget(ctx, 0, 0)?;
    match pattern {
        GraphPattern::Bgp { patterns } => bgp(store, patterns, ctx, seed, emit),
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(iri),
            inner,
        } => visit(
            store,
            inner,
            &graph_context(store, iri.as_str(), ctx)?,
            seed,
            emit,
        ),
        GraphPattern::Union { left, right } => {
            visit(store, left, ctx, seed, emit)?;
            visit(store, right, ctx, seed, emit)
        }
        GraphPattern::Join { left, right } => {
            let mut joined = 0;
            visit(store, left, ctx, seed, &mut |row| {
                visit(store, right, ctx, &row, &mut |row| {
                    super::pattern_util::check_eval_budget(ctx, joined, joined)?;
                    joined += 1;
                    emit(row)
                })
            })
        }
        GraphPattern::Filter { expr, inner } => {
            let pushed = super::filter_pushdown::seed_iri_equalities(store, expr, inner, seed)?;
            let seed = pushed.as_ref().unwrap_or(seed);
            let narrowed = super::string_pushdown::narrowed(expr, inner, seed, ctx);
            visit(
                store,
                inner,
                narrowed.as_ref().unwrap_or(ctx),
                seed,
                &mut |row| {
                    if eval_filter(store, expr, &row, ctx)? {
                        emit(row)?;
                    }
                    Ok(())
                },
            )
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => visit(store, inner, ctx, seed, &mut |mut row| {
            if let Some(value) = eval_expr(store, expression, &row) {
                row.insert(variable.as_str().into(), value);
            }
            emit(row)
        }),
        _ => unreachable!("applicability checked before evaluation"),
    }
}

use super::count_state::Counter;
struct Group {
    key: Vec<Option<Value>>,
    counters: Vec<Counter>,
}
fn new_group(key: Vec<Option<Value>>, n: usize) -> Group {
    Group {
        key,
        counters: (0..n).map(|_| Counter::default()).collect(),
    }
}

/// Applicable only to COUNT(*)/COUNT(variable), optionally DISTINCT on a term.
/// Groups retain first-seen order, and an empty ungrouped input yields zero.
pub(super) fn try_evaluate(
    store: &Store,
    inner: &GraphPattern,
    variables: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<Output>> {
    let mean = aggregates.iter().any(|(_, a)| {
        matches!(
            a,
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Avg,
                ..
            }
        )
    });
    let count = aggregates.iter().any(|(_, a)| {
        matches!(
            a,
            AggregateExpression::CountSolutions { .. }
                | AggregateExpression::FunctionCall {
                    name: AggregateFunction::Count,
                    ..
                }
        )
    });
    if !count
        || ctx.row_limit.is_some()
        || !supported(inner)
        || aggregates.iter().any(|(_, agg)| !match agg {
            AggregateExpression::CountSolutions { distinct: false } => true,
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Count,
                expr,
                ..
            } => matches!(expr, Expression::Variable(_)) || (mean && stable(expr)),
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Avg,
                expr,
                ..
            } => stable(expr),
            _ => false,
        })
    {
        return Ok(None);
    }
    // Applicable aggregate statements use a bounded page cache for the whole
    // visit. Restore the reader's shared mmap policy before releasing its lease.
    if store.has_attachments() {
        evaluate(store, inner, variables, aggregates, ctx, seed)
    } else {
        super::count_mmap::scalar(store, || {
            evaluate(store, inner, variables, aggregates, ctx, seed)
        })
    }
}

fn evaluate(
    store: &Store,
    inner: &GraphPattern,
    variables: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<Output>> {
    // Validate externally supplied references once. References emitted by a
    // leaf come from this store's dictionary; an unattached store has one id
    // per IRI, so decoding and re-looking-up that IRI cannot change its binding.
    for value in seed.values() {
        if let Value::Ref(id) = value
            && *id >= 0
        {
            store.resolve(*id)?;
        }
    }
    let _direct = DirectRefs::install();
    if let Some(output) = native_count(store, inner, variables, aggregates, ctx, seed)? {
        return Ok(Some(output));
    }
    let keys: Vec<String> = variables.iter().map(|v| v.as_str().into()).collect();
    let mut groups = Vec::<Group>::new();
    let mut buckets = HashMap::<String, Vec<usize>>::new();
    if keys.is_empty() {
        groups.push(new_group(vec![], aggregates.len()));
    }
    let mut produced = 0;
    visit(store, inner, ctx, seed, &mut |row| {
        // Preserve the complexity gate; the allocation avoidance is no waiver
        // to run an unbounded exploded join or pure-Rust filter past a deadline.
        super::pattern_util::check_eval_budget(ctx, produced, produced)?;
        produced += 1;
        let key: Vec<Option<Value>> = keys.iter().map(|k| row.get(k).cloned()).collect();
        let index = if keys.is_empty() {
            0
        } else {
            let bucket = buckets.entry(format!("{key:?}")).or_default();
            if let Some(index) = bucket.iter().copied().find(|&i| groups[i].key == key) {
                index
            } else {
                let index = groups.len();
                bucket.push(index);
                groups.push(new_group(key, aggregates.len()));
                index
            }
        };
        for ((_, agg), counter) in aggregates.iter().zip(&mut groups[index].counters) {
            match agg {
                AggregateExpression::CountSolutions { .. } => counter.count += 1,
                AggregateExpression::FunctionCall { expr, distinct, .. } => {
                    if let Some(value) = eval_expr(store, expr, &row) {
                        if matches!(
                            agg,
                            AggregateExpression::FunctionCall {
                                name: AggregateFunction::Avg,
                                ..
                            }
                        ) {
                            counter.average(value, *distinct);
                        } else {
                            counter.add(value, *distinct);
                        }
                    }
                }
            }
        }
        Ok(())
    })?;
    let mut vars = keys.clone();
    vars.extend(aggregates.iter().map(|(v, _)| v.as_str().to_string()));
    let rows = groups
        .into_iter()
        .map(|group| {
            let mut row = Bindings::new();
            for (var, value) in keys.iter().zip(group.key) {
                if let Some(value) = value {
                    row.insert(var.clone(), value);
                }
            }
            for ((variable, expression), counter) in aggregates.iter().zip(group.counters) {
                if let Some(value) = counter.result(expression) {
                    row.insert(variable.as_str().into(), value);
                }
            }
            row
        })
        .collect();
    Ok(Some((rows, vars)))
}

// Only the pure arithmetic/date projection used alongside COUNT in metadata.
// Volatile functions and other expressions retain the ordinary evaluator.
fn stable(expr: &Expression) -> bool {
    match expr {
        Expression::Variable(_) | Expression::Literal(_) | Expression::NamedNode(_) => true,
        Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b)
        | Expression::Equal(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::And(a, b)
        | Expression::Or(a, b) => stable(a) && stable(b),
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => stable(a),
        Expression::If(a, b, c) => stable(a) && stable(b) && stable(c),
        Expression::FunctionCall(
            Function::StrDt
            | Function::Year
            | Function::Month
            | Function::Day
            | Function::Hours
            | Function::Minutes
            | Function::Seconds
            | Function::Floor,
            args,
        ) => args.iter().all(stable),
        _ => false,
    }
}

fn native_count(
    store: &Store,
    inner: &GraphPattern,
    variables: &[Variable],
    aggregates: &[(Variable, AggregateExpression)],
    ctx: &TemporalContext,
    seed: &Bindings,
) -> Result<Option<Output>> {
    if !variables.is_empty()
        || !seed.is_empty()
        || store.has_attachments()
        || ctx.string_narrows.is_some()
    {
        return Ok(None);
    }
    let (tp, scoped) = match inner {
        GraphPattern::Bgp { patterns } if patterns.len() == 1 => (&patterns[0], ctx.clone()),
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(iri),
            inner,
        } => {
            let GraphPattern::Bgp { patterns } = inner.as_ref() else {
                return Ok(None);
            };
            if patterns.len() != 1 {
                return Ok(None);
            }
            (&patterns[0], graph_context(store, iri.as_str(), ctx)?)
        }
        _ => return Ok(None),
    };
    if matches!(scoped.graph, GraphScope::AnyNamed { .. })
        || ((scoped.graph.is_root_default()
            || (scoped.entails_rdfs && scoped.graph.includes_root_default()))
            && super::rdfs::is_rdf_type_pattern(tp))
    {
        return Ok(None);
    }
    // Repeated variables require bind_row compatibility checks. Blank-node
    // labels are hidden join variables and deliberately keep the normal path.
    let mut names = Vec::new();
    for term in [&tp.subject, &tp.object] {
        match term {
            TermPattern::Variable(v) => names.push(v.as_str()),
            TermPattern::BlankNode(_) => return Ok(None),
            _ => {}
        }
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        names.push(v.as_str());
    }
    let unique: std::collections::HashSet<_> = names.iter().collect();
    if unique.len() != names.len() {
        return Ok(None);
    }
    for (_, aggregate) in aggregates {
        match aggregate {
            AggregateExpression::CountSolutions { distinct: false } => {}
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Count,
                expr: Expression::Variable(v),
                distinct: false,
            } if names.contains(&v.as_str()) => {}
            _ => return Ok(None),
        }
    }
    let n = super::triple::count_triple_pattern(store, tp, &scoped)? as i64;
    let vars: Vec<String> = aggregates.iter().map(|(v, _)| v.as_str().into()).collect();
    let row = vars.iter().map(|v| (v.clone(), Value::Int(n))).collect();
    Ok(Some((vec![row], vars)))
}

#[cfg(test)]
#[path = "count_stream_tests.rs"]
mod tests;

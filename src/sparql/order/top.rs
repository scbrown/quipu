//! Stable bounded top-k over an already evaluated relation.
use super::*;
use std::cmp::Ordering;

struct Entry {
    ordinal: usize,
    row: Bindings,
    keys: Vec<Key>,
    bytes: usize,
}

fn compare(a: &Entry, b: &Entry, expression: &[OrderExpression]) -> Ordering {
    for ((order, (va, ia)), (vb, ib)) in expression.iter().zip(&a.keys).zip(&b.keys) {
        let cmp = match (ia, ib) {
            (Some(a), Some(b)) => a.cmp(b),
            _ => super::super::aggregate::compare_option_values(va, vb),
        };
        let cmp = if matches!(order, OrderExpression::Asc(_)) {
            cmp
        } else {
            cmp.reverse()
        };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.ordinal.cmp(&b.ordinal)
}

fn down(heap: &mut [Entry], expression: &[OrderExpression], ctx: &TemporalContext) -> Result<()> {
    let mut parent = 0;
    while parent * 2 + 1 < heap.len() {
        check_deadline(ctx)?;
        let mut child = parent * 2 + 1;
        if child + 1 < heap.len()
            && compare(&heap[child], &heap[child + 1], expression) == Ordering::Less
        {
            child += 1;
        }
        if compare(&heap[parent], &heap[child], expression) != Ordering::Less {
            break;
        }
        heap.swap(parent, child);
        parent = child;
    }
    Ok(())
}

fn consistent_keys(
    rows: &[Bindings],
    expression: &[OrderExpression],
    ctx: &TemporalContext,
) -> Result<bool> {
    use spargebra::algebra::Expression;
    for order in expression {
        let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
        let variable = match expr {
            Expression::Literal(_) | Expression::NamedNode(_) => continue,
            Expression::Variable(variable) => variable,
            _ => return Ok(false),
        };
        let mut previous = None;
        for row in rows {
            check_deadline(ctx)?;
            let Some(value) = row.get(variable.as_str()) else {
                continue;
            };
            let class = match value {
                Value::Int(_) => 0,
                // Integer/integer comparisons are exact, while mixed floating
                // comparisons promote and can collapse distinct integers above
                // 2^53. Mixing those relations is not a consistent heap order.
                Value::Float(number) if number.is_finite() => 5,
                Value::Float(_) => return Ok(false),
                Value::Ref(_) => 1,
                Value::Bool(_) => 2,
                Value::Typed { datatype, .. }
                    if crate::namespace::is_numeric_datatype(datatype) =>
                {
                    return Ok(false);
                }
                Value::Str(_) | Value::Lang { .. } | Value::Typed { .. } => 3,
                Value::Bytes(_) => 4,
            };
            if previous.is_some_and(|before| before != class) {
                return Ok(false);
            }
            previous = Some(class);
        }
    }
    Ok(true)
}

pub(crate) fn top_rows(
    store: &Store,
    rows: Vec<Bindings>,
    expression: &[OrderExpression],
    ctx: &TemporalContext,
    limit: usize,
) -> Result<Vec<Bindings>> {
    check_deadline(ctx)?;
    let keep = limit.min(rows.len());
    // Project discovers response-provenance columns from the entire relation,
    // including rows outside LIMIT. Keep those rows until Project has run.
    if rows.iter().any(|row| {
        ["_provider", "_trust", "_freshness"]
            .iter()
            .any(|key| row.contains_key(*key))
    }) {
        return sort_rows(store, rows, expression, ctx);
    }
    if limit == 0 {
        return Ok(Vec::new());
    }
    // Heap pruning requires a consistent order. Inspect only bound variable
    // types; never evaluate an expression a second time to decide eligibility.
    if !consistent_keys(&rows, expression, ctx)? {
        return Ok(sort_rows(store, rows, expression, ctx)?
            .into_iter()
            .take(keep)
            .collect());
    }
    // Large demands use the full sorter (native spill or explicit Wasm guard),
    // rather than increasing either existing retained-key limit.
    if keep.saturating_add(1).saturating_mul(expression.len()) > MAX_KEY_ENTRIES {
        return Ok(sort_rows(store, rows, expression, ctx)?
            .into_iter()
            .take(keep)
            .collect());
    }
    let mut heap: Vec<Entry> = Vec::new();
    let mut bytes = 0;
    for (ordinal, row) in rows.into_iter().enumerate() {
        check_deadline(ctx)?;
        let mut keys = Vec::with_capacity(expression.len());
        let mut cost = (2 * std::mem::size_of::<Entry>())
            .saturating_add(expression.len().saturating_mul(std::mem::size_of::<Key>()));
        for order in expression {
            check_deadline(ctx)?;
            let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
            let value = super::super::filter::eval_expr(store, expr, &row);
            check_deadline(ctx)?;
            cost = cost
                .checked_add(payload_bytes(value.as_ref()))
                .ok_or_else(budget_error)?;
            let iri = if let Some(Value::Ref(id)) = value.as_ref() {
                let lexical = store.resolve(*id)?;
                cost = cost
                    .checked_add(lexical.len().saturating_add(128))
                    .ok_or_else(budget_error)?;
                Some(std::sync::Arc::<str>::from(lexical))
            } else {
                None
            };
            // Charge the incoming row as well as the retained heap. A huge
            // losing key is not permission for an unbounded temporary object.
            let mut projected = bytes;
            charge(&mut projected, cost)?;
            keys.push((value, iri));
        }
        let entry = Entry {
            ordinal,
            row,
            keys,
            bytes: cost,
        };
        if heap.len() < keep {
            charge(&mut bytes, cost)?;
            heap.push(entry);
            let mut child = heap.len() - 1;
            while child > 0 {
                check_deadline(ctx)?;
                let parent = (child - 1) / 2;
                if compare(&heap[parent], &heap[child], expression) != Ordering::Less {
                    break;
                }
                heap.swap(parent, child);
                child = parent;
            }
        } else if compare(&entry, &heap[0], expression) == Ordering::Less {
            bytes -= heap[0].bytes;
            charge(&mut bytes, cost)?;
            heap[0] = entry;
            down(&mut heap, expression, ctx)?;
        }
    }
    let mut output = Vec::with_capacity(heap.len());
    while let Some(last) = heap.pop() {
        check_deadline(ctx)?;
        if heap.is_empty() {
            output.push(last.row);
            break;
        }
        let first = std::mem::replace(&mut heap[0], last);
        output.push(first.row);
        down(&mut heap, expression, ctx)?;
    }
    output.reverse();
    Ok(output)
}

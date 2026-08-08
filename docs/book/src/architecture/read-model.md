# The In-Memory Read Model

Quipu stores facts in SQLite and answers SPARQL by compiling each triple pattern
into SQL. The read model is an optional in-memory index over the same facts,
built so that joins do not have to go back to SQL for every row.

It is **on by default** for multi-pattern queries. This page explains what it
is, what it deliberately cannot answer, and when you might turn it off.

## What it holds

Three permutation indexes over one graph's currently-valid asserted facts, plus
an object index:

| Index | Answers |
|---|---|
| `spo` | `<s> ?p ?o` — everything about a subject |
| `pso` | `?s <p> ?o` — everything using a predicate |
| `pos` | `?s <p> <o>` and `?s a <Type>` |
| `osp` | `?s ?p <o>` — everything pointing at an object |

Joins are hash joins: each pattern is evaluated once and joined on the variables
it shares with the rows so far, rather than re-evaluated once per accumulated
row.

Everything is keyed by term id. The `id ↔ IRI` dictionary lives on the store
itself and is shared with every other read path rather than duplicated here.

## What it deliberately cannot answer

The model is built from **currently-valid, asserted facts in one graph**. That
is a strict subset of what SPARQL can ask, and the difference is not a gap to
paper over — it is the point.

| Query | Served from |
|---|---|
| Current facts in the ROOT graph | the model |
| `valid_at` / `as_of_tx` time travel | SQL |
| `GRAPH <iri>`, `GRAPH ?g`, `FROM` | SQL |
| Overlays and tombstones | SQL |
| A store with attached databases | SQL |
| Anything, while a write holds an open transaction | SQL |
| A graph past the size budget | SQL |

A guard checks every one of these before the model is consulted, and anything
outside its scope falls through to the SQL path unchanged. **A query that time
travels is not slower because an optimization is missing — it is a different
question, asked of data the model does not hold.**

## What it costs, and what it buys

Measured on stores of synthetic episodes, SQL path → read model:

| Episodes | Point lookup | Type scan | 2-hop join |
|---:|---|---|---|
| 1,000 | 0.11 → 0.14 ms | 4.6 → 5.4 ms | 1,016 → **38 ms** |
| 4,000 | 0.10 → 0.13 ms | 18.8 → 18.5 ms | 26,233 → **225 ms** |
| 10,000 | 0.16 → 0.12 ms | 56.2 → 46.8 ms | 173,803 → **560 ms** |

**27× to 310× on joins**, which are also linear now rather than quadratic, and
no measured regression on the other shapes.

Three design choices are what make that true, and each was a measured failure
before it was a choice:

- **Only multi-pattern queries use it.** A single pattern is what SQL is already
  fast at — a bound-subject lookup is a tenth of a millisecond against an index.
  Routing those through the model made them pay to build one, which measured as
  0.12 ms → 320 ms.
- **Writes maintain the model** rather than dropping it, so a write-then-read
  loop does not pay a rebuild every time.
- **Size is bounded** at one million triples (roughly 320 MB), checked with a
  `COUNT` so an oversized store never pays a build to discover it is oversized.
  Past that ceiling queries use SQL — slower on joins, but exactly the behaviour
  they had before.

## Turning it off

```rust
store.set_read_model_enabled(false);          // SQL for everything
store.set_read_model_max_triples(200_000);    // or just lower the ceiling
```

Worth considering if your process is memory-constrained, or if your workload is
almost entirely single-pattern lookups on a large store — there the model is
built for joins that never come.

## Correctness

Both paths share one binding implementation, so a triple becomes the same
`Value` either way — including the subtleties, like a subject that resolves to a
blank node binding as a string rather than a reference.

Beyond that, a differential test runs every pattern shape through both paths and
asserts the answers match, and the entire test suite has been run with the model
forced on.

Two bugs that surfaced from doing so are worth knowing about, because they show
what this kind of index has to get right:

- The write-time policy guard runs queries **inside** an open transaction,
  against rows that are staged but not committed. A model built there and left
  resident after a *denied* write would hold facts the database had rolled back.
- Conversely, a model cached before a write is missing that write's staged rows,
  so the guard would judge a write against a store lacking the very facts that
  made it valid.

The first fix dropped the model at both points. It worked, and it forced a
rebuild after every write — which is why the fast path could not be the default
at first.

The structural fix is to **suspend** the model for the duration of a write
instead: the guard uses SQL, so the model never observes staged rows, and there
is nothing to poison on rollback. That is also what makes maintenance possible,
because the model is still there when the commit lands.

The general rule either way: the model is never consulted across a transaction
boundary it did not observe.

## See also

- `docs/design/in-memory-read-model.md` — the full design, measurements, and
  phase plan.
- [EAVT Fact Log](eavt.md) — the storage the model is built from.
- [SPARQL Engine](sparql.md) — the evaluator it plugs into.

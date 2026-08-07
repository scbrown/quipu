# The In-Memory Read Model

Quipu stores facts in SQLite and answers SPARQL by compiling each triple pattern
into SQL. The read model is an optional in-memory index over the same facts,
built so that joins do not have to go back to SQL for every row.

It is **off by default**. This page explains what it is, why it is off, and when
to turn it on.

## What it holds

Three permutation indexes over one graph's currently-valid asserted facts, plus
an object index:

| Index | Answers |
|---|---|
| `spo` | `<s> ?p ?o` — everything about a subject |
| `pso` | `?s <p> ?o` — everything using a predicate |
| `pos` | `?s <p> <o>` and `?s a <Type>` |
| `osp` | `?s ?p <o>` — everything pointing at an object |

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

A guard checks every one of these before the model is consulted, and anything
outside its scope falls through to the SQL path unchanged. **A query that time
travels is not slower because an optimization is missing — it is a different
question, asked of data the model does not hold.**

## Why it is off by default

Measured on a store of 10,000 synthetic episodes (170,150 facts):

| Query shape | SQL path | Read model |
|---|---:|---:|
| Point lookup (`<s> ?p ?o`) | 0.12 ms | **320 ms** |
| Type scan (`?s a <T>`, LIMIT 100) | 47 ms | 47 ms |
| 2-hop join | timeout | 30 s |

The join improves by 4–5×. The point lookup gets roughly 2,600× worse. Two
reasons, both structural:

- **The first query after any write rebuilds the whole model** — about 250 ms at
  this size. The model is dropped on every write rather than maintained
  incrementally, so a write-then-read loop pays that rebuild every time.
- **Joining is still quadratic.** Consulting the model makes each individual
  pattern lookup cheap, but the surrounding join still evaluates one pattern per
  row of the previous pattern's results. Moving the *leaf* to memory does not
  change the *shape* of the join.

A path that is three orders of magnitude slower on the most common query shape
is not a fast path. Both causes are tracked and the default flips once they are
fixed.

## Turning it on

```rust
let store = Store::open("my.db")?;
store.set_read_model_enabled(true);
```

Worth doing when your workload is **join-heavy and read-mostly**: many
multi-pattern queries between writes, so the rebuild is amortised. Worth
avoiding when writes and reads interleave, or when queries are dominated by
bound-subject lookups — which are already fast.

The model is built on first use and dropped on every write, so nothing needs to
be invalidated by hand.

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

Both are fixed by dropping the model on entry to a write as well as on exit. The
general rule: the model is never consulted across a transaction boundary it did
not observe.

## See also

- `docs/design/in-memory-read-model.md` — the full design, measurements, and
  phase plan.
- [EAVT Fact Log](eavt.md) — the storage the model is built from.
- [SPARQL Engine](sparql.md) — the evaluator it plugs into.

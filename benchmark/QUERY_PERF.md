# String filter query performance gate

`scripts/quipu-query-perf.py` guards the string-FILTER pushdown from
`aegis-tl2q4j`. Agents write `CONTAINS`, `REGEX`, `STRSTARTS` and `LCASE` filters
naturally. Before the pushdown, these filters timed out on the production store.
After it, most finish in milliseconds. The mixed load ratchet
(`LOAD_TEST.md`) has no string-filter class, so without this gate, a change that
undid the narrowing would pass CI.

## What it runs

The query set is the nine shapes measured on the production store for
`aegis-tl2q4j`, rewritten onto neutral `example.org` IRIs, plus a STRSTARTS shape
and the `C0` control (an exact type count with no filter).

Every class runs twice:

- **pushed**: the query as written; the scan is narrowed in SQL.
- **defeated**: `FILTER((X) || false)`. Pushdown handles only top-level
  conjuncts, so this form is never narrowed.

The gate fails a class when:

1. the two forms return different rows; the narrowing must never change an
   answer. This check runs without `LIMIT`, because two scans can return
   different first-N rows. A class that matches no rows fails too, since it
   would prove nothing.
2. defeated time / pushed time (p50) falls below `min_ratio`. Both forms run in
   the same job, so runner speed cancels out. This is the positive control on
   every PR. A build without narrowing fails here, and so does a planner that
   learns to see through `|| false`, so the gate cannot go quietly vacuous.
3. pushed p95 exceeds `max_p95_ms`, or exceeds `max_rel_c0` times the C0 median.

Every timed defeated response must succeed with valid, nonempty row bindings
before a ratio is calculated. HTTP errors, timeouts, malformed or empty
responses fail the class and leave its ratio unset. Error durations cannot
inflate the numerator. Run `just test-query-perf` for offline healthy and
failure controls, including a successful differential followed by timed 408s.

`Q7_graph_var_contains` (`GRAPH ?g` plus a filter on `?g`) is not narrowed. It is
tracked on `aegis-nmouik`, reported with its timings, and fails only if it errors
or times out. `Q1_status_pred` filters on predicate IRIs. On this fixture,
narrowing does not speed it up (ratio ~0.6), so it has ceilings and no ratio. It
is here because the first cut of `aegis-tl2q4j` took this shape from 0.58 s to a
30 s timeout.

## Fixture

`generate` writes a deterministic store shaped like production: 150,000
entities, 2.1M triples, 120 types with a long tail, 200 predicates, labels and
comments, IRI and literal objects, about 70% of entities in the default graph,
and 26 named graphs. Rare needles appear at fixed rates, so each class matches a
small, stable number of rows, which is where narrowing pays. At toy scale a full
scan is fast and the gate would prove nothing. All IRIs are under
`http://example.org/perf/`, and the self-test asserts that no internal hostname
appears.

`load` uses declared ingests (`quipu ingest` with count and sha256) for the named
graphs. `quipu ingest` refuses ROOT by design, so the default-graph file goes into
a scratch store and is copied into ROOT with `quipu graph import --as
urn:quipu:graph:root`. Load the fixture onto tmpfs (`/dev/shm`): every chunk
commits with an fsync, and on a busy disk the same load takes over 10 minutes
instead of about 3.

CI caches the generated N-Triples and manifest by generator source hash. A
changed generator invalidates the cache. Each job still loads a fresh database
with its own binaries, so a cached database cannot hide schema changes.

## Running it

```sh
cargo build --release --locked --features full --bin quipu --bin quipu-server
scripts/quipu-query-perf.py generate --out /tmp/fixture
scripts/quipu-query-perf.py load --fixture /tmp/fixture --db /dev/shm/qp.db \
  --quipu target/release/quipu
target/release/quipu-server --db /dev/shm/qp.db --bind 127.0.0.1:3030 &
scripts/quipu-query-perf.py gate --budget benchmark/query-perf-budget.json
```

`smoke --url <served>` runs only the pushed form of every class and reports
p50/p95. Use it after a deploy. The defeated forms are full scans and do not
belong on a production server.

## Budgets

`benchmark/query-perf-budget.json` records the measured numbers next to the
limits. The ratios are set at about a third of the measured ratio or less. The
ceilings are about four times the measured value relative to C0, and absolute
ceilings catch only gross regressions. Tighten only from repeated CI runs.

Positive control, measured when the gate was written: a release build with
`string_pushdown::narrowed` returning nothing failed all seven narrowing classes
on both the ratio and the C0 ceiling. A new pushdown-sensitive shape joins
`CLASSES` with a budget entry; the self-test refuses a class without one.

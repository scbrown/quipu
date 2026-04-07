# Quipu Reasoner — Incremental Datalog on the Bitemporal Fact Log

> Created: 2026-04-06
> Status: DRAFT — design, not implementation
> Related: [vision.md](./vision.md) (Killer Feature #3, Open Decision D)

## One-Line

A stratified Datalog rule engine that derives high-level relations
(`affects`, `dependsOn`, `atRisk`) from raw EAVT facts, keeps them
fresh incrementally as base facts change, and answers counterfactual
impact questions via speculative transactions — without leaving the
"SQLite energy" thesis.

## The Motivating Question

> "What services across all hosts would be affected if I remove this
> package from this container?"

This looks like one question. It is actually four capabilities stacked:

| # | Capability | Layer |
|---|------------|-------|
| 1 | Derive `affects(pkg, svc)` from raw `installedIn` / `runs` / `hostedOn` facts | **Inference** — rule engine |
| 2 | Walk the transitive closure across derived edges | **Reachability** — SPARQL property paths |
| 3 | Evaluate "if I remove X" without actually removing it | **Counterfactual** — speculative transactions |
| 4 | Keep derived facts correct when base facts change | **Cascade** — incremental view maintenance |

Quipu today has **(2)** via SPARQL property paths and RDFS subclass
inference (`src/sparql/rdfs.rs`). This document specifies **(1)**, **(3)**,
and **(4)** as a single coherent module: `src/reasoner/`.

## Scale Context

The aegis homelab — quipu's primary target workload — has roughly:

- **~200 base entities** (containers, services, hosts, networks, crew, rigs, tools)
- **~1,500-2,500 base facts** from the JSON-LD ontology
- **~10K-30K episode-derived facts per year** assuming 5-10 beads/day
- **~50K total facts** after a year of active use, derived included

This is **four orders of magnitude** below the point where incremental
evaluation is required for performance. The interesting reason to design
for incrementality is **correctness under updates**, not latency. A full
ruleset re-run over 50K facts with datafrog is expected to complete in
single-digit milliseconds; SPARQL queries over the materialized result
are unaffected.

This matters because it lets us make the **simplest** design decision at
every fork. No worker pools, no separate dataflow runtime, no LanceDB in
the reasoner path. Just a library call after each transact.

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                      Quipu Reasoner                              │
│                                                                  │
│   ┌─────────────┐      ┌──────────────┐      ┌──────────────┐  │
│   │  RuleSet    │      │   Scheduler  │      │  Datafrog    │  │
│   │  (loaded    │─────▶│   (delta-    │─────▶│  evaluator   │  │
│   │   from TTL) │      │    aware)    │      │  (per-rule)  │  │
│   └─────────────┘      └──────┬───────┘      └──────┬───────┘  │
│          ▲                    │                     │          │
│          │                    │                     │          │
│          │           ┌────────┴──────────┐          │          │
│          │           │  Delta Stream     │          │          │
│          │           │  (tx → Δfacts)    │          │          │
│          │           └────────┬──────────┘          │          │
│          │                    │                     │          │
└──────────┼────────────────────┼─────────────────────┼──────────┘
           │                    │                     │
           │                    │          ┌──────────▼──────────┐
           │                    │          │   Derived Datums    │
           │                    │          │  (source: reasoner) │
           │                    │          └──────────┬──────────┘
           │                    │                     │
┌──────────┼────────────────────┼─────────────────────┼──────────┐
│          │                    │                     │          │
│          │          ┌─────────┴─────────┐           │          │
│          │          │  transact() hook  │◀──────────┘          │
│          │          └─────────┬─────────┘                      │
│          │                    │                                │
│   ┌──────┴──────┐    ┌────────▼────────┐                       │
│   │   shapes    │    │   facts         │    EAVT bitemporal    │
│   │   (SHACL)   │    │   (E,A,V,tx,    │    fact log (SQLite)  │
│   └─────────────┘    │    valid_from,  │                       │
│                      │    valid_to)    │                       │
│                      └─────────────────┘                       │
└──────────────────────────────────────────────────────────────────┘
```

**The reasoner is a library, not a service.** It owns no state between
calls. After every successful `Store::transact()`, the reasoner receives
a `Delta`, runs the affected rules, produces new `Datum`s, and writes
them back through the same `transact()` path with
`source: "reasoner:<rule-id>"`. SQLite remains the source of truth.
You can turn the reasoner off and the database is still valid.

## Three Layers in Detail

### Layer 1 — Inference (Datalog rules)

A `Rule` is a Horn clause over quipu predicates:

```text
affects(Pkg, Svc) :- installedIn(Pkg, Container), runs(Container, Svc).
affects(Pkg, Svc) :- affects(Pkg, X), dependsOn(X, Svc).
atRisk(Svc)      :- affects(Pkg, Svc), not validated(Pkg).
```

The rule DSL is stored as Turtle in the shapes table, alongside SHACL
shapes, because it **is** part of the ontology:

```turtle
@prefix : <http://aegis.local/> .
@prefix rule: <http://quipu.local/rule#> .

:affects-direct a rule:Rule ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "installedIn(?pkg, ?c), runs(?c, ?svc)" ;
    rule:id "R1" .

:affects-transitive a rule:Rule ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "affects(?pkg, ?x), dependsOn(?x, ?svc)" ;
    rule:id "R2" .
```

At load time the rule parser:

1. **Type-checks** predicates against the SHACL shape vocabulary
2. **Assigns strata** via topological sort on the predicate dependency graph
3. **Rejects non-stratifiable rulesets** with an error naming the negation cycle
4. **Compiles** each rule into a datafrog join expression

Execution is **semi-naive** within each stratum: round N only considers
tuples where at least one body atom fires on a fact that was new in
round N-1. This is datafrog's default evaluation model and is already
battle-tested inside `rustc`'s borrow checker.

#### Why datafrog over alternatives

| Option | Verdict | Reason |
|--------|---------|--------|
| **datafrog** | ✅ Use | ~1500 LOC, semi-naive, stratified, no runtime, used in production by rustc. Integrates as a function call. |
| ascent | ⚠️ Viable | Macro-based Datalog, more ergonomic. Heavier, larger dep footprint. Consider if datafrog ergonomics become painful. |
| crepe | ⚠️ Viable | Compile-time Datalog, fast. Requires rules at build time — doesn't fit runtime-loaded rules. |
| whelk-rs | ❌ Wrong fit | OWL 2 EL reasoner, not general Datalog. Reserve for OWL class hierarchy work. |
| differential-dataflow | ❌ Wrong fit for quipu's scale | Worker pool runtime, in-memory arrangements as source of truth, no bitemporal awareness. Fights "SQLite energy." |
| hand-rolled | ❌ No | Stratification, semi-naive eval, and negation are subtle. Don't reinvent rustc's wheel. |

#### Stratification termination proof (for the record)

A ruleset is **stratifiable** if its predicates can be assigned integer
strata such that:

- Positive body atoms: `stratum(body) ≤ stratum(head)`
- Negative body atoms: `stratum(body) < stratum(head)`

Termination then follows from three observations:

1. **Within a single stratum**, all rules are positive (by construction —
   negations only reference strictly-lower strata). Pure monotone
   Datalog over a finite Herbrand universe reaches a fixed point in
   bounded steps.
2. **Strata execute in order**, lowest first. When stratum N runs,
   strata 0..N−1 are frozen.
3. **Information flows upward only** — a higher stratum never asserts
   a fact that would change a lower one.

Finite strata × each stratum terminates → the whole evaluation
terminates. Non-stratifiable programs (e.g. `p :- not q. q :- not p.`)
are rejected at load time with an error pointing to the cycle. This is
a **static check** — we never ship an unstratifiable ruleset and then
discover the non-termination at runtime.

### Layer 2 — Cascade (incremental evaluation)

After every successful `Store::transact()`, a `Delta` is emitted:

```rust
pub struct Delta {
    pub tx: i64,
    pub asserts: Vec<Datum>,   // op = 1
    pub retracts: Vec<Datum>,  // op = 0
}
```

The reasoner maintains a **predicate-to-rules index** built from the
ruleset: `HashMap<PredicateId, Vec<RuleId>>`. On each delta:

1. For each changed predicate, look up affected rules.
2. Collect the transitive closure of affected rules (rules whose head
   is a body predicate of another affected rule).
3. Partition by stratum, sort low → high.
4. For each stratum, run datafrog evaluation restricted to rules in
   the affected set, seeding with the delta.
5. New derivations become `Datum`s and are written through
   `Store::transact()` with `source: "reasoner:<rule-id>"`.
6. Retracted derivations become `Op::Retract` datums — the bitemporal
   fact log handles the closing of `valid_to` naturally.

**Retraction is the subtle part.** When a base fact is retracted, we
need to retract every derived fact whose support depended on it.
Options:

- **Re-derive and diff** — rerun the affected rules from scratch,
  compare new derivation set against current, retract the difference.
  Simple, correct, wastes work at scale but at 50K facts it's free.
- **Truth maintenance** — track each derived fact's support
  (which base facts and which rule produced it). On retraction,
  walk the support graph. Correct and efficient but requires an
  auxiliary support table in SQLite.

**Start with re-derive-and-diff.** Truth maintenance is an optimization
for when you have millions of derived facts and it actually matters.
At quipu's target scale, simpler is correcter.

The CDC hook itself is cheap. Two options:

- **SQLite `update_hook`** via rusqlite — fires per row-level write.
  Works but is chatty and sits below the transact boundary.
- **Wrap `Store::transact()` directly** — collect asserted/retracted
  datums from the datum vec, emit a single `Delta` at commit time.
  Preferred: one hook point, clean transaction boundary, no chatter.

The reasoner registers itself as a `TransactObserver` (new trait):

```rust
pub trait TransactObserver: Send + Sync {
    fn after_commit(&self, store: &Store, delta: &Delta) -> Result<()>;
}
```

This pattern already exists in spirit in the auto-embedding hook on
write. The reasoner is just another observer.

### Layer 3 — Counterfactual (speculative transactions)

"What if I remove X" is answered by a **speculative transaction**:

```rust
impl Store {
    /// Execute `f` against a forked view of the store with `hypothetical`
    /// datums applied. The fork is discarded; the underlying store is
    /// never mutated. Useful for impact analysis and "what-if" queries.
    pub fn speculate<F, R>(
        &self,
        hypothetical: &[Datum],
        f: F,
    ) -> Result<R>
    where
        F: FnOnce(&Store) -> Result<R>;
}
```

Implementation options:

1. **Copy-on-write SQLite** — open the backing file with `PRAGMA query_only`
   plus an attached in-memory overlay database that holds the hypothetical
   delta. Queries union the two. Complex but cheap.
2. **In-memory clone** — dump current facts into a fresh in-memory store,
   apply hypothetical datums, run queries, drop the clone. Simple, works
   at any scale quipu targets.
3. **Savepoint + rollback** — SQLite `SAVEPOINT`, apply hypothetical
   datums, run reasoner, run query, `ROLLBACK TO SAVEPOINT`. Uses the
   *real* store but guarantees no mutation persists. Simplest and leverages
   existing infrastructure.

**Start with savepoint + rollback.** It reuses the real transact path,
means speculative queries see exactly the same reasoner output that a
real commit would produce, and uses zero new code beyond a thin wrapper.

A `quipu impact` CLI command becomes:

```bash
# Current-state walk: what currently depends on this package?
quipu impact pkg:nginx --hops 5

# Counterfactual: what would break if I removed it?
quipu impact --remove pkg:nginx
# Under the hood:
#   1. BEGIN SAVEPOINT
#   2. Retract all facts where subject=pkg:nginx
#   3. Run reasoner to recompute affects/dependsOn
#   4. SPARQL query: affected services/hosts
#   5. ROLLBACK TO SAVEPOINT
#   6. Return results
```

The REPL can offer an interactive variant (`quipu repl` gains a
`:speculate` command that stays in speculative mode until `:commit` or
`:rollback`).

## Rule Examples for Aegis Homelab

Grounded in `/home/braino/workspace/aegis/ontology/entities/*.jsonld`:

```text
# A package affects every service in its container.
affects(Pkg, Svc) :-
    installedIn(Pkg, Container),
    runsService(Container, Svc).

# A package affects every host running its container.
affectsHost(Pkg, Host) :-
    installedIn(Pkg, Container),
    runningOn(Container, Host).

# Services on the same container share fate — one crash takes them all.
sharesFateWith(SvcA, SvcB) :-
    runsService(Container, SvcA),
    runsService(Container, SvcB),
    SvcA != SvcB.

# Transitive service dependency.
dependsOn(SvcA, SvcC) :-
    dependsOn(SvcA, SvcB),
    dependsOn(SvcB, SvcC).

# A host is critical if it runs a service that many other services depend on.
criticalHost(Host) :-
    runningOn(Container, Host),
    runsService(Container, Svc),
    count(dependsOn(_, Svc)) > 3.

# A service is "at risk" if it depends on something without validation.
atRisk(Svc) :-
    dependsOn(Svc, X),
    not validated(X).
```

With these rules loaded, the motivating question becomes:

```sparql
# "What services across all hosts would be affected if I remove nginx
#  from the proxy container?"
SELECT DISTINCT ?svc ?host WHERE {
  ?pkg aegis:name "nginx" ;
       aegis:installedIn ct:proxy .
  ?pkg aegis:affects ?svc .
  ?svc aegis:runningOn ?host .
}
```

Answered against the speculative fork that has `?pkg aegis:installedIn
ct:proxy` retracted. Done.

## Integration with Existing Quipu Subsystems

| Subsystem | Interaction |
|-----------|-------------|
| **EAVT fact log** (`src/store/`) | Sole source of truth. Reasoner reads facts, writes derived facts back via `Store::transact()`. Derived facts have a distinct `source` field in the transaction row. |
| **SHACL validation** (`src/shacl.rs`) | Derived facts are validated exactly like user-written facts. A rule that produces a fact violating a shape is a rule bug — fail loudly. |
| **RDFS subclass inference** (`src/sparql/rdfs.rs`) | Stays in SPARQL evaluation where it is. RDFS is cheap enough to evaluate at query time and doesn't benefit from materialization. |
| **SPARQL engine** (`src/sparql/`) | Unchanged. Derived predicates become ordinary triples, queryable with the normal evaluator. |
| **Episode ingestion** (`src/episode/`) | Episodes trigger deltas like any other write. The reasoner sees bead ingestion as a normal source of change. |
| **Vector backends** (`src/vector_lance.rs`, default SQLite) | Unchanged. Vectors are for semantic search, not reasoning. A derived predicate like `atRisk` can be joined with vector search results, but that's downstream composition. |
| **MCP tools** (`src/mcp/tools.rs`) | New tool: `quipu_impact(entity_iri, remove=bool)` wraps `speculate()` + impact query. |
| **REST API** (`src/http/`) | New endpoint: `POST /impact` with JSON body `{entity, remove, hops}`. |
| **Web UI** | New panel: "Impact Analysis" — select entity, toggle remove/keep, visualize affected subgraph. |

## What Goes in the Fact Log

Derived facts are ordinary facts with a distinguishing source:

```sql
-- A derived fact written by rule R1:
INSERT INTO facts (e, a, v, tx, valid_from, valid_to, op)
VALUES (
    <pkg:nginx>,         -- e
    <aegis:affects>,     -- a
    <svc:proxy>,         -- v
    42,                  -- tx (the reasoner's transact)
    '2026-04-06T...',    -- valid_from (inherited from the youngest base fact)
    NULL,                -- valid_to (current)
    1                    -- op = assert
);

-- The transaction row records provenance:
INSERT INTO transactions (id, timestamp, actor, source)
VALUES (42, '2026-04-06T...', 'reasoner', 'rule:R1');
```

This means:

- **Time travel works for derived facts** — `as_of_tx = 41` shows the world
  before the derivation.
- **Bitemporal queries compose naturally** — "what did we derive about X
  as of March 15" is a normal valid-time query.
- **Retractions are bitemporal** — when a base fact's support disappears,
  the derived fact gets `valid_to = now` set, not a hard delete.
- **Provenance is built in** — every derived fact traces to a rule via
  its transaction row, and the rule's body points at the input facts.

This is Killer Feature #7 ("Episode Provenance") extended to rules: the
chain raw observation → extracted fact → rule derivation is a graph
traversal across transactions.

## Phased Rollout

Each phase is independently shippable and testable.

### Phase 1 — Impact CLI on property paths (no new deps)

Goal: **prove the question is answerable on current data.**

- Add `quipu impact <entity> [--hops N]` command
- Implement as SPARQL property path walk: `?x ^p1/p2/p3* ?y`
- No reasoner, no new crates
- Surfaces ontology gaps — you'll find missing edges immediately
- Serves as the regression test for everything that follows
- Ships in: one PR, ~300 LOC including tests

### Phase 2 — Reasoner skeleton with datafrog

Goal: **derived facts exist in the store with provenance.**

- New crate dep: `datafrog = "2"` (2 KB, zero transitive deps)
- New module: `src/reasoner/` with `Rule`, `RuleSet`, `evaluate()`
- Rule DSL as Turtle with `rule:head` / `rule:body` properties
- Stratification check at load time with clear error messages
- `quipu reason` CLI command: runs the full ruleset, reports derived/retracted
- Full re-derivation on each call (not yet incremental)
- Aegis ruleset lives in `shapes/aegis-rules.ttl`
- Ships in: 3-4 PRs (parser, stratifier, evaluator, CLI+tests)

### Phase 3 — Reactive evaluation

Goal: **derived facts stay fresh automatically.**

- `Delta` type and `TransactObserver` trait
- Wire the reasoner as an observer
- Predicate-to-rule index built at ruleset load
- Delta-seeded evaluation (affected rules only)
- Re-derive-and-diff retraction
- Feature flag during rollout: `--features reactive-reasoner`
- Ships in: 2-3 PRs

### Phase 4 — Counterfactual queries

Goal: **"what if I remove this" works.**

- `Store::speculate()` with SAVEPOINT/ROLLBACK implementation
- `quipu impact --remove` flag wired through
- MCP tool `quipu_impact`
- REST endpoint `POST /impact`
- Ships in: 2 PRs (core + interfaces)

### Phase 5 — Optional, deferred

Not on the near path. Listed so they aren't forgotten:

- **Truth maintenance** — incremental retraction via support tracking. Only
  if phase 3's re-derive-and-diff becomes slow.
- **OWL 2 RL rules** — whelk-rs integration for class hierarchy reasoning.
  RDFS already covers the 80% case in vision.md.
- **Differential dataflow** — if a real scale wall appears (>10M derived
  facts, sub-second freshness required). Unlikely for the homelab target.
- **Explain** — "why do we believe `affects(pkg:nginx, svc:grafana)`?"
  traces the derivation chain. Cheap to build once the reasoner exists.

## Open Questions

### Q1 — Rule DSL: Turtle vs. a separate file format?

Turtle keeps rules in the same ontology layer as SHACL shapes and is
loadable via the existing shapes table. Downside: awkward to write by
hand. A small Datalog-surface DSL parsed to the same AST would be
nicer ergonomically. **Recommendation:** Turtle as the storage format,
optional `.dl` file as convenient author format, compiler from `.dl`
to Turtle at load time.

### Q2 — Where do reasoner errors surface?

If a rule fires infinitely (non-termination escaped the stratifier), or
produces a SHACL violation, or references an unknown predicate — where
does the error go? A `reasoner_errors` table? Logged to stderr? Emitted
as a validation event via the agent-friendly feedback channel?
**Recommendation:** three destinations — fatal errors block the transact
and return to the caller (same as SHACL violations today), rule-level
warnings go to a `reasoner_log` table, statistics (timings, derivation
counts) emit via tracing.

### Q3 — Should derived facts count toward SHACL `sh:minCount` / `sh:maxCount`?

If a shape says "a service must have `runsOn`" and the reasoner
derives a `runsOn` edge, does that satisfy the constraint? Probably yes
— otherwise validation becomes incoherent under reasoning. But this
means validation must run **after** reasoning, not before.
**Recommendation:** `transact()` validates after reasoner closure.

### Q4 — Does the reasoner need its own transaction boundary?

When the reasoner derives 50 facts in response to a base fact assertion,
should those 50 facts land in the **same** tx as the user's write, or in
a **child** tx that references the parent? Same-tx is atomic and simple.
Child-tx preserves "user wrote N facts, reasoner derived M facts" as a
first-class distinction for audit.
**Recommendation:** child tx with a parent reference — matches the
episode provenance model and keeps `quipu log` readable.

### Q5 — How are rules versioned?

Rules are part of the ontology, so they should evolve with it. Does
changing a rule retract all derivations made under the old rule? Does
it retroactively re-derive under the new rule? Both are surprising.
**Recommendation:** rule change → retract all derivations produced by
the old rule version (their tx is marked by `source`), then run the new
rule. Log the rule change as a `DecisionRecord` in the graph itself.

### Q6 — Negation-as-failure semantics

Stratification gives us a well-defined answer for negation over
**closed** predicates. But quipu is an open-world knowledge base —
absence of a fact doesn't mean its negation. How does the reasoner
reconcile? **Recommendation:** negation operates over the **current
materialized state**, not a closed-world assumption. Rules that use
`not validated(X)` are saying "we haven't derived or observed
validation yet" — explicitly an open-world NAF. Document this
prominently.

## What This Is Not

- **Not a replacement for SPARQL** — it produces triples for SPARQL to query.
- **Not OWL reasoning** — OWL 2 RL/EL can be added later via whelk-rs or a
  second rule loader that compiles OWL axioms to Datalog. Today's scope is
  domain rules.
- **Not a constraint solver** — it derives facts that follow from existing
  facts, not hypothetical constants or new entities. No creativity.
- **Not differential dataflow** — semi-naive evaluation is sufficient at
  target scale. DD remains a deferred escape hatch.
- **Not the SHACL validator** — SHACL checks shape conformance; the reasoner
  derives new facts. They compose but are distinct.

## Why This Design Holds Up

Three properties make this design robust to quipu's future:

1. **SQLite remains the source of truth.** The reasoner is a stateless
   function over the fact log. Turn it off and the database is still
   valid, still queryable, still inspectable with `sqlite3`. No hidden
   state in a separate runtime.

2. **Phases are independently shippable.** Phase 1 answers the motivating
   question on day one with zero new infrastructure. Phase 2 adds rules
   but without incremental updates. Phase 3 adds freshness. Phase 4 adds
   counterfactuals. Each phase is useful on its own; each builds on the
   previous; no phase requires the next.

3. **Scale-up path is open.** Datafrog handles 50K-1M facts trivially.
   If quipu ever outgrows that, the `TransactObserver` interface is
   generic enough to swap the reasoner implementation (ascent, whelk,
   even differential dataflow) without changing callers. The rule DSL
   and the fact log model are the stable contracts.

The bet is that **embedded Datalog is the right shape** for quipu's
scale and values, and that incremental view maintenance is desirable
mainly for correctness rather than performance. If either of those
turns out to be wrong, the phased design limits blast radius: we'd
discover it at phase 2 or 3 and re-scope phase 4 without having
committed to a heavyweight runtime.

## References

- [vision.md](./vision.md) — parent design. Killer Feature #3
  ("Incremental Materialization with Provenance") and Open Decision D
  ("Reactive notifications") are what this document resolves.
- `src/sparql/rdfs.rs` — existing subclass inference, the shape the
  reasoner's query-time extensions should mirror.
- `src/reconcile/mod.rs` — existing precedent for a post-transact pass
  that writes derived facts back through `Store::transact()`. The
  reasoner uses the same pattern with a different derivation engine.
- `shapes/aegis-ontology.shapes.ttl` — the SHACL shapes that the rule
  DSL will share vocabulary with.
- `ontology/aegis-context.jsonld` (in the aegis repo) — the source of
  truth for the predicate vocabulary the initial ruleset will use.
- datafrog: <https://crates.io/crates/datafrog>
- Stratified Datalog: Abiteboul, Hull, Vianu, *Foundations of Databases*,
  chapter 15 (the canonical reference for semi-naive evaluation and
  stratification termination proofs).

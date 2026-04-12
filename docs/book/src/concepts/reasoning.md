# The Reasoner

Raw facts tell you what *is*. The reasoner tells you what *follows*.

Quipu's reasoner is a stratified Datalog engine that reads the EAVT fact log,
applies rules, and writes derived facts back into the store. The derived facts
look and behave like any other fact — you query them with SPARQL, validate
them with SHACL, and time-travel through them — but their `source` tag traces
back to the rule that produced them.

## Why Derive Facts?

Consider a homelab with containers running on hosts. You've recorded:

```text
traefik  runsOn  webproxy
webproxy runsOn  koror
```

A human reads this and concludes "traefik transitively runs on koror." But
SPARQL doesn't know that unless you either:

1. **Query-time**: write a property path (`runsOn+`) every time you ask
2. **Write-time**: materialise the transitive closure once and query it directly

Option 1 works for simple cases. But as your graph grows — services depending
on packages, packages installed in containers, containers running on hosts —
the property paths get unwieldy, slow, and duplicated across queries. Option 2
is what the reasoner does: derive the facts once, keep them fresh, and let
every query benefit.

## Rules as Horn Clauses

A rule says "if these conditions hold, then this fact is true":

```text
runsOn(?svc, ?host) :- runsOn(?svc, ?container), runsOn(?container, ?host).
```

Read this as: "if service S runs on container C, and container C runs on
host H, then service S runs on host H." The part before `:-` is the **head**
(what gets derived). The parts after are the **body** (what must already be
true).

Rules in Quipu are written in Turtle files using a simple vocabulary:

```turtle
ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?container), runsOn(?container, ?host)" .
```

The head and body are string literals that the reasoner parses. Bare predicate
names like `runsOn` are expanded using a configurable prefix, so you don't
need to write full IRIs inside the rule strings.

## Stratification: Layered Evaluation

When rules depend on each other, the reasoner needs to evaluate them in the
right order. This is called **stratification**.

Consider two rules:

```text
dependsOn(?a, ?c)  :- dependsOn(?a, ?b), dependsOn(?b, ?c).
affects(?pkg, ?svc) :- installedIn(?pkg, ?c), runsService(?c, ?svc).
```

The first rule is self-recursive — it reads and writes `dependsOn`. The
second reads `installedIn` and `runsService` (which no rule produces) and
writes `affects`. These rules are independent and could run in either order.

The stratifier builds a dependency graph between predicates and groups rules
into **strata** (layers):

- **Stratum 0**: Base facts — predicates that appear only in rule bodies
  (`installedIn`, `runsService`). No rules to evaluate here.
- **Stratum 1**: Rules that only depend on base facts (`affects`).
- **Stratum 2**: Rules that depend on stratum 1 results.
- And so on.

Rules within the same stratum can be positively recursive (like transitive
`dependsOn`). The evaluator handles this with **semi-naive iteration**: it
keeps applying the rule until no new facts are derived, using only the
newly-derived facts from the previous round to avoid redundant work.

What the stratifier *won't* allow is a **negation cycle** — rule A negates
a predicate that rule B produces, and rule B negates something rule A
produces. This would make evaluation non-deterministic, so the reasoner
rejects it at load time with an error naming the offending predicates.

## The Evaluation Cycle

Each time the reasoner runs, it performs a full **re-derive-and-diff**:

1. **Stratify** the ruleset (this is cheap — just graph analysis)
2. **Load** the current world: read all relevant facts from the store
3. **Evaluate** each stratum in order, running rules to fixpoint
4. **Diff** each rule's new derivations against its old ones
5. **Write** asserts for new facts, retracts for stale ones

Step 4 is the key insight. Rather than tracking which base facts changed and
propagating deltas (truth maintenance), the reasoner re-derives everything
and compares. At the target scale of ~50K facts, this is fast enough to
complete in milliseconds and dramatically simpler to get correct.

Every derived fact is written through `Store::transact()` with a source tag
like `reasoner:depends_on_transitive`, so you can always tell which rule
produced a fact and when.

## Reactive Evaluation

Running `quipu reason` manually works, but you'd have to remember to run it
after every change. **Reactive evaluation** automates this: the reasoner
registers as a `TransactObserver` on the store and fires automatically after
every commit.

When a transaction lands, the reactive reasoner:

1. Checks which predicates changed
2. Finds the rules whose bodies reference those predicates
3. Follows the dependency chain to find transitively affected rules
4. Re-evaluates only the affected strata
5. Writes any new asserts or retracts

This means derived facts stay fresh without explicit invocation. Add a new
`runsOn` edge, and the transitive closure updates in the same transaction
boundary.

The reactive reasoner is smart enough to skip its own output — when it sees
a transaction with `source = "reasoner:..."`, it doesn't re-trigger. This
prevents infinite loops.

## Speculate: "What If?" Queries

Sometimes you want to explore hypothetical changes without committing them.
The `speculate()` API does exactly this:

```rust
let report = store.speculate(&hypothetical_datums, timestamp, |store| {
    // Inside here, the store contains the hypothetical facts.
    // Run the reasoner, query the results, whatever you need.
    evaluate(store, &ruleset, timestamp)
})?;
// Here the hypothetical facts are gone — the store is unchanged.
```

Under the hood, `speculate()` opens a SQLite savepoint, applies the
hypothetical facts, runs your closure, then rolls back. The store is never
modified. This lets you answer questions like:

- "What would change if I remove this package from this container?"
- "If koror goes down, which derived dependencies break?"
- "What's the blast radius of upgrading this library?"

## Provenance: Tracing Derived Facts

Every derived fact carries metadata that traces it back to its source:

| Field | Example |
|-------|---------|
| `source` | `reasoner:depends_on_transitive` |
| `actor` | `reasoner` |
| `valid_from` | `2026-04-04T12:00:00Z` |

You can query derived facts by their provenance:

```sparql
PREFIX ont: <http://aegis.gastown.local/ontology/>

SELECT ?entity ?attr ?value
WHERE {
  ?entity ?attr ?value .
  # Filter to only reasoner-derived facts
  FILTER(STRSTARTS(STR(?source), "reasoner:"))
}
```

Or from the Rust API, filter on the `source` field of returned facts.

## The datafrog Engine

Under the hood, evaluation is powered by [datafrog](https://crates.io/crates/datafrog),
a ~1500-line Rust crate used by the Rust compiler itself for borrow checking.
It implements semi-naive evaluation with no runtime dependencies, no services
to manage, and no allocation overhead worth measuring at homelab scale.

You never interact with datafrog directly — it's an implementation detail.
The reasoner compiles your Horn clause rules into datafrog join plans and
runs them inside `while iteration.changed()` loops.

## What's Next

- [The Rule Builder](../tutorials/rule-builder.md) — write your first rules
- [Reasoner Reference](../reference/reasoner.md) — rule syntax, CLI, API, errors
- [Impact Analysis](../recipes/impact-analysis.md) — put the reasoner to work

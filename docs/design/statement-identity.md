# Design: statement identity, edge properties, and bounded paths

> **Implementation status (2026-08-04):** ⬜ **Analysis and proposal, not built.**
> The mechanism this document generalizes already exists in one hardcoded form:
> `src/episode/mod.rs:560` reifies an edge to carry `quipu:confidence`. Nothing
> else here is implemented.

This note answers a design question raised against Quipu: *should Quipu adopt
property-graph features — key/value attributes on entities, and the traversal
ergonomics that Gremlin and openCypher give you?*

The short answer is that the node half is already shipped, the edge half is
half-shipped, and the part that is genuinely missing is not the property-graph
data model but **per-occurrence statement identity** and **bounded path
queries**. §6 explains why adopting a second data model would cost more than it
buys.

## 1. What already exists

**Node attributes are done.** `src/episode/mod.rs:516` takes an episode node's
`properties: {k: v}` map and emits one triple per key into the base namespace,
with real XSD typing — `xsd:integer`, `xsd:double`, `xsd:boolean`, or a plain
string literal. A JSON array fans out into a multi-valued predicate rather than
being dropped. That is the node half of a property graph, spelled as triples.

`examples/demo-graph/demo.ttl` uses the same thing directly in Turtle
(`ex:region "eu-west"`, `ex:engine "postgres"`) and needs nothing further.

**Edge attributes exist in exactly one flavour.** `src/episode/mod.rs:560`
reifies a statement to hang `quipu:confidence` off it, deriving the reification
IRI from `fnv1a_64(src|rel|tgt)` so that re-ingest dedups at fact level instead
of accumulating statement nodes. This is a general edge-qualifier mechanism with
a single hardcoded qualifier.

**Traversal exists, but as a function rather than a query.** `src/impact.rs`
walks outward from an entity with a hop bound and a predicate allowlist. Its own
header states the constraint that motivates half of this document:

> Property paths cannot express a depth cap, so we walk the store directly.

## 2. The gap that matters: identity per occurrence

Some relations are **events**, and an event needs identity per occurrence. RDF's
set-of-triples semantics gives it identity per *endpoint pair*, which is not the
same thing, and Quipu's storage layer enforces the set reading: an identical
re-assertion is an idempotent no-op (`src/store/ops.rs:15`; `store/events.rs:563`
asserts that a no-op re-ingest emits no event at all).

So `:orch :dispatched :worker` asserted at 14:02 and again at 14:47 is **one
fact**. The second dispatch leaves no trace. Bitemporality does not rescue this —
it records when facts were learned and when they held, not that the same fact
occurred twice.

This is not hypothetical. `src/governance/tree.rs` documents the consequence in
its own header, as a limitation of the reader:

> **Sibling dispatches collapse.** Two separate dispatches of the same worker by
> the same orchestrator produce the same chain, so they land on one node. If
> those two runs did different things, the tree shows one node that did both —
> which is attribution dilution reappearing at the reader rather than at the
> record.

Attribution dilution is the exact failure the governance stack exists to prevent
(SARC §9.5). The reconstruction is honest about losing it, and marks affected
nodes via `Forest::collapsed` rather than presenting the tree as unambiguous —
but the information is lost at the storage layer, so no reader can recover it.

## 3. Three consumers, three different asks

The three use cases pull in different directions, and it is worth keeping them
distinct rather than solving them with one feature flag.

| Use case | What it needs | Status |
|---|---|---|
| Governance / agentic dev | Per-occurrence identity for dispatch and tool-call events | Blocked by §2 |
| Homelab / infrastructure | Qualifiers on edges (`dependsOn` hard vs soft), and traversal that can filter on them | Needs §4 + §5 |
| Code graph (Hank) | Depth-bounded transitive paths with the path returned | Needs §5 |

**Governance.** Beyond §2's collapse, `src/governance/tree.rs` and
`src/governance/inheritance.rs` perform real graph work — tree reconstruction,
ancestor/descendant reachability, subtree containment for the laundering check —
in Rust, over `TraceRecord` structs parsed from JSONL, entirely outside the graph
engine. Meanwhile `src/governance/verdict_facts.rs` writes verdicts *into* the
store. Governance data is therefore split across two representations, and the
half that most wants querying is the half that is not in the graph.

Note what this costs `inheritance.rs` specifically. Its *weak* finding —
"constraint C was evaluated at a dispatch node and never appears anywhere in its
subtree" — is a subtree-reachability question. With the trace in the graph and
per-dispatch identity, it is a query rather than a hand-rolled pass.

**Homelab.** `examples/demo-graph/demo.ttl` models `dependsOn`, `runsOn`,
`ownedBy`, `persistsTo`. At least one of those wants a qualifier: a hard
dependency and a soft one are different claims, and a blast radius computed
without the distinction over-reports — which trains operators to ignore it. The
query that follows is the one nothing in Quipu can express today: *walk
`dependsOn+` from a cluster, only through hard edges, bounded at 4 hops, and
return the paths.*

**Code graph.** Hank promotes `calls` / `imports` / `definedIn` edges
(`hank/src/export.rs`). Interesting questions about a call graph are transitive
reachability with a bound. Note that `hank-spec.md` §9.6 deliberately keeps
Hank's *interactive* reachability in its own in-memory graph, so this is not a
request to serve Hank's hot path. It is about the promoted, governed graph:
§9.3's code-archaeology example (`--valid-at`) is one-hop today, and the
transitive version of that question is what needs the bound.

## 4. Proposal A — generalize the edge qualifier

Extend `episode::Edge` with an arbitrary `properties` map alongside the existing
`confidence`, reusing the reification path already in `episode/mod.rs`. Consider
accepting and emitting RDF-star syntax (`<< :a :calls :b >> :confidence 0.9`) so
that qualified statements stay ergonomically queryable rather than requiring
callers to hand-write the `rdf:subject` / `rdf:predicate` / `rdf:object` join.

**The identity fork is the real decision.** The current reification IRI is
`fnv1a_64(src|rel|tgt)` — deterministic from the endpoints, deliberately, so that
re-ingest dedups. Event-shaped edges need the opposite: identity that includes
the occurrence. These are both correct, for different relations, and the choice
cannot be inferred from the triple. Options, roughly in order of preference:

1. **Declare it in the ontology.** Mark a predicate as event-shaped in SHACL
   (`quipu:occurrenceScoped true` or similar) and let the ingest path pick the
   IRI derivation from the predicate's declaration. Keeps the decision in the
   schema, where the rest of Quipu's strictness lives, and makes it auditable.
2. **Declare it per-edge in the episode payload.** Simpler, but pushes an
   ontology decision into every writer, where it will drift.
3. **Infer from the presence of a timestamp property.** Rejected — implicit, and
   silently changes dedup behaviour when a writer adds an unrelated field.

Whichever is chosen, the default must stay dedup-by-endpoints so existing
ingests are byte-identical.

## 5. Proposal B — bounded paths as a query construct

Two additions to `src/sparql/property_path.rs`:

- **A depth bound** on path expressions, removing the reason `impact.rs` bypasses
  the SPARQL engine.
- **Path return** — the sequence of nodes and edges traversed, not just the
  endpoint pair that `eval_path_expr` yields today.

With qualified statements from Proposal A, the traversal should also be able to
filter on an edge's qualifier mid-walk; that is what makes the hard/soft
dependency case work.

Once both land, `impact()` should be reimplemented on top of the path evaluator
so there is one traversal engine rather than two. Its current predicate
allowlist and hop bound become the degenerate case of a path query, and its
behaviour — including the deliberate choice to follow only reference-valued
facts, never literals — needs preserving as tests before the swap.

## 6. Why not adopt a property-graph model

The obvious alternative is to add a property-graph store alongside the RDF one
and expose openCypher or Gremlin over it. Rejected, for two reasons.

**It is the Neptune trap.** Amazon Neptune ships both models, and they are
separate engines: data loaded as RDF is not reachable from Gremlin, so you choose
at load time and live with it. A second model in Quipu means SHACL validation,
bitemporality, provenance, and the governance write-gate each need a second
implementation or silently do not apply to half the store. For a system whose
pitch is auditability, a region of the store that the policy engine cannot
validate is worse than a missing feature.

**The ergonomic gap is narrower than the model gap.** Everything in §3 is
reachable via statement identity plus bounded paths, inside the one substrate.
What openCypher would additionally buy is syntax, and syntax is not worth a
second copy of the governance stack.

## 7. Adjacent risk — namespace governance

Worth settling alongside this work, because Proposal A widens the same path.
Today every key in an episode node's `properties` map becomes a predicate in the
base namespace via `sanitize_iri_local`, with no shape governing which keys are
admissible. Agents writing free-form properties will mint predicates
indefinitely, and nothing reports the drift.

For a project whose thesis is *start strict*, an unvalidated write path into the
ontology is the thing to fence before widening it to edges. The cheapest useful
version is a report, not a block: list predicates minted by episode ingest that
no shape mentions — the same shape as `quipu audit inventory`, which already
answers "which tool classes are ungoverned."

## 8. Suggested ordering

1. **Proposal A** (statement identity + edge properties) — unblocks governance
   and homelab, and is the smallest change since the mechanism exists.
2. **Proposal B** (bounded paths, path return) — unblocks the code graph and
   homelab blast radius; collapses two traversal engines into one.
3. **Trace ingestion into the store**, so `governance/tree.rs` becomes a query
   rather than a reconstruction. Much the largest change, depends on (1) for
   per-dispatch identity, and wants its own design note before any code.
4. **Namespace governance report** (§7) — independent of the others, and cheap.

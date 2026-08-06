# Design: Graph Labels — freshness, trust and policy as a lattice over named graphs

> **Implementation status (2026-08-06):** ⬜ **Designed, not built.** Nothing in
> this document is implemented. The substrate it builds on — named graphs, the
> `graphs` registry, `GRAPH`/`FROM`/`FROM NAMED`, graph-scoped authority — is
> built (see [named-graphs.md](named-graphs.md) for its own partial-status
> banner). Issue links live in each section; the dependency order is in §10.

**Status:** Quipu can partition the store into named graphs but cannot say
anything *about* a graph: how current its contents are, how far they should be
trusted, or what policy governs them. Every consumer has hand-rolled that
missing layer differently — NeuralAmplifier with a SHACL-enforced per-fact
`ruleTier` tag plus retrieval precedence enforced by Python call order, Hank
with a `tier`/`freshness` vocabulary that the graph it promotes into cannot
carry, and the SARC conformance work with a trust boundary that is "declared
and reported, not closed." This design makes the missing layer a store
primitive: labels on graphs, drawn from ordered value sets, composing under
one invariant.

## 1. The one invariant: composition never widens

Three modules already independently chose the same rule:

- `Authority::intersect` — delegation only narrows; an empty intersection is a
  refusal, never a fallback (`src/governance/authority.rs`).
- SHACL branch composition — a branch may only *add* constraints, never remove
  ([named-graphs.md](named-graphs.md) §7.2).
- Hank's confidence rule — "a policy is only as confident as its
  least-confident leaf" (hank, governance plane).

The invariant is **composition never widens** — deliberately *not* "everything
is a meet," because the operator flips direction by axis: freshness and trust
compose by **meet** (a union of graphs is only as fresh and as trusted as its
weakest member), while obligations compose by **join** (a union of graphs
carries the *union* of their restrictions). Same invariant, two operators.
Naming the invariant rather than the operator is what prevents the sign error.

A small `src/lattice.rs` (`Meet`/`Join` traits and the fold) gets implemented
by `Authority`, making the parallel structural instead of rhetorical.

## 2. The three axes

| Axis | Values | Order | Compose |
|---|---|---|---|
| `quipu:freshness` | `fresh` > `recomputing` > `stale` | total | meet |
| `quipu:trust` | IRIs ranked by a *declared chain* | total within a chain; cross-chain **refused** | meet |
| `quipu:policyClass` | set of obligation tokens (`pii`, `no-export`, …) | powerset ⊆ | join |

- **Freshness** reuses Hank's exact strings. When a composed label must be
  reported on a scale that only admits fresh/stale, `recomputing` collapses to
  `stale` — the conservative reading is the only one that cannot overstate,
  the same mapping Hank records on its verdict path.
- **Trust is not a hardcoded enum.** Hank's chain is
  `live|lsp|tree-sitter|committed|attested`; NeuralAmplifier's is
  `canonical|house-rule|engine-observed|aspirational`. Quipu must learn
  neither vocabulary: trust values are IRIs, and the *ordering* is data —
  `smac:canonical quipu:trustRank 40 ; quipu:inChain smac:ruleTierChain`.
  Quipu ships one default chain
  (`attested > verified > observed > asserted > untrusted`) and nothing else.
  Comparing ranks across chains is refused, never silently compared as ints.
- **Policy is the deliberately-partial axis** — Denning's information-flow
  lattice (§9). Incomparable label sets are normal; the join always exists on
  a powerset, so nothing blocks.

### 2.1 Undeclared is not a lattice value

Every graph in every existing deployment is unlabelled, so the default decides
compatibility. Neither pole is right: defaulting to ⊤ fail-opens trust (an
unlabelled graph would read as `attested`); defaulting to ⊥ drags every
existing query's label to the floor. Instead the composed label is a pair:

```text
(fold over the DECLARED labels, coverage ∈ {full, partial, none})
```

Today's stores report coverage `none` and the label as *undeclared* — matching
the stack's convention of omitting freshness rather than faking `fresh`. When
an enforcement floor (§5) is configured, partial coverage **fails the floor**:
fail-safe at enforcement, honest at reporting.

## 3. Storage: hybrid, RDF authoritative

**Source of truth: RDF facts in a reserved meta-graph** `urn:quipu:graph:meta`,
registered `committed` in `graphs`. The `quipu:` namespace
(`http://quipu.dev/ontology/`) is already the control-predicate home
(`quipu:onViolation`).

```turtle
<urn:quipu:graph:datalinks-smac>
    quipu:freshness   "fresh" ;
    quipu:trust       smac:canonical ;
    quipu:policyClass "no-export" .
```

RDF wins the source-of-truth role because labels must be:

- **Queryable** — precedence-as-`ORDER BY` (§6) falls out of plain SPARQL with
  no engine change.
- **Bitemporal** — "was this graph fresh when we made that decision?" is
  answerable by `as_of_tx`. This is not theoretical: NeuralAmplifier promotes a
  learned tactic above the doctrine it contradicts when confidence crosses a
  threshold, so precedence *changes mid-game* and its history must be
  reconstructable.
- **Governed** — a label write goes through `transact_to_graph`, so
  `enforce_graph_authority` fires **on the meta-graph**. Consequence to state
  plainly: relabelling a graph requires authority over
  `urn:quipu:graph:meta`, not over the graph being labelled. Otherwise a
  tenant relabels itself `attested`.
- **SHACL-validated** — `shapes/graph-labels.ttl`: closed `sh:in` lists,
  `minCount`/`maxCount 1`. Permissive on domain shape, strict on the label
  predicates, because the tag is the reader's only signal of trust.

**Derived cache: five additive nullable columns on `graphs`** — `fresh_rank`,
`trust_rank`, `trust_chain`, `policy`, `labels_tx` — written in the same
savepoint as the meta-graph facts. The columns denormalize the meta-graph
exactly as `facts.g` denormalizes `tx → graph`. `quipu doctor labels`
recomputes from RDF and reports drift; a `trust_chain` mismatch after a chain
redefinition makes `label_of` refuse rather than lie.

The cache exists because the fold is computed at query entry, and a nested
SPARQL evaluation there is a re-entrancy hazard on the single connection —
`query_temporal` installs the `ProgressGuard` and owns the deadline/row budget.
Pure columns were rejected because they lose history, governance and
queryability — the reasons to put labels in a knowledge graph at all.

### 3.1 Vocabulary discipline: `rdfs:range`, never `rdfs:domain`

The label predicates will attach at two levels — graph IRIs now, statement
nodes later (§7). The recorded Q-SARC-VOCAB lesson
([policy-edit-hooks.md](policy-edit-hooks.md)) applies: a domain declaration on
a predicate used at two attachment levels would silently retype whatever it
touches when the reasoner materialises it. Label predicates declare
`rdfs:range` only.

## 4. Propagation: per-dataset, computed once

**Per-solution labels are a semantic blocker, not a cost knob.**
`src/sparql/triple.rs` omits `g` from the projection precisely so that
`SELECT DISTINCT e, a, v` collapses a triple present in three graphs to one
solution — that *is* the RDF-merge semantics of `FROM`
([named-graphs.md](named-graphs.md) §4). Projecting `g` to carry a per-row
label would turn one solution into three: different results, not slower ones.

So v1 labels the **dataset**: the composed label is a property of the query's
active dataset, computed once in `apply_dataset` — one
`SELECT … FROM graphs WHERE g IN (…)` over a handful of PK-indexed rows.
O(|dataset|), not O(|rows|). For `GraphScope::AnyNamed` the fold ranges over
the restriction (or all named graphs) and is cached per query.

This is conservative by construction: a query whose dataset includes a stale
graph is labelled stale even if no returned row came from it. Conservative
cannot overstate, which is the direction every mapping in this stack already
chooses.

**The homomorphism, as a property test:**

```text
label(A ∪ B) = label(A) ⊓ label(B)
```

Graph-sets form a join-semilattice under union; labels form a lattice; `label`
is a monotone map between them. That is the entire formal content of "a
semilattice of graphs carrying lattice-valued labels," and it is directly
testable as a proptest over random graph sets. If it fails, composition has
stopped being associative and every derived answer is suspect.

### 4.1 Surfacing

`QueryResult`'s three variants stay untouched — many internal callers match on
it and want nothing more. New entry point:

```rust
pub struct LabeledResult { pub result: QueryResult, pub labels: DatasetLabels }
pub fn query_labeled(store, sparql, ctx) -> Result<LabeledResult>
```

`/query` and `quipu_query` gain a top-level `"labels"` JSON key beside the
existing `truncated` flag. Old clients ignore an extra key. **No new HTTP
route** — deliberately, to keep `http_auth.rs`'s completeness test out of the
critical path.

**Per-row labels only where they are free:** under `GRAPH ?g` the graph is
already bound per row, so `_freshness`/`_trust`/`_policy` columns can be
annotated by the exact pattern `FederatedProvider::query_all` already uses for
`_provider`. Opt-in via the request. A `?_freshness` pseudo-variable is
rejected: spargebra does not know it, and a magic variable is a lie in the
algebra.

## 5. Enforcement: an opt-in floor

`[quipu.labels] min_freshness`, `min_trust`, `deny_policy_tokens`. When the
composed label falls below the floor, the query is **refused**, and the
refusal names the graph that dragged the label down — mirroring
`authority::refusal`, which names the narrowest link in the chain. "Which
layer made this answer untrustworthy" becomes a one-line answer.

Defaults: all unset, no refusals, behaviour identical. The config keys and
their consumer land in the same change (the `unwired_warnings` guard holds us
to that).

## 6. Precedence as data

NeuralAmplifier's retrieval precedence —
`canonical > engine-observed > house-rule > doctrine > learned` — is real
policy currently enforced by the order Python calls things in. With labels it
becomes seven `quipu:trustRank` facts, and retrieval precedence is plain
SPARQL:

```sparql
SELECT ?s ?o ?g ?rank WHERE {
  GRAPH ?g { ?s smac:prefersUnit ?o }
  GRAPH <urn:quipu:graph:meta> { ?g quipu:trustRank ?rank }
} ORDER BY DESC(?rank)
```

No engine change — *if* the `?g` join across two `GRAPH` patterns works today
(it should, per [named-graphs.md](named-graphs.md) §5's "the same `?g` across a
BGP is enforced by the join"; verify before promising — if not, that is one
small issue, not a redesign).

Three boundaries to keep the claim honest:

- **The top rungs stay outside.** Engine legality (`action_space`) belongs to
  the engine; deny-policies are pre-action gates. NeuralAmplifier's
  "precedence by order, not by policy code" is a deliberate *safety* property
  of pipeline ordering — retrieval cannot widen the action space. The lattice
  decides which plane wins on *disagreement*; it does not reorder the
  pipeline.
- **Promotion moves the fact, not the rank.** A learned tactic that outranks
  its plane is by definition not a property of its plane. The recommendation
  to NeuralAmplifier: promotion = moving the tactic into a higher-ranked graph
  (`memory:promoted`), making the override a bitemporal graph move —
  auditable, time-travelable, reversible — rather than a side predicate.
- **Ties and incomparability are reported, never resolved silently.**
  Cross-chain rank comparison is refused. A dataset created *with a declared
  ordering* refuses duplicate ranks among members. Elsewhere ties are returned
  as ties, and a genuine conflict between incomparable graphs surfaces as
  `quipu:contested` carrying both values. A silent tiebreak is how a "learned
  tactic beats canonical" bug ships.

## 7. Named datasets — the semilattice half

**`parent_branch` is not touched.** It is not a taxonomy; it is the resolution
root of `compose_view`'s nearest-overlay-wins rule, bind-once so an overlay
cannot forge presence in a base it was never bound to. Making it many-parent
would break overlay resolution for a benefit that belongs elsewhere.

The overlapping structure already exists — `FROM a b c` *is* an arbitrary
graph set, and the merge semantics are already correct. What is missing is a
**name** for a set, so it can be reused, labelled, governed, and handed to
another agent:

```sql
CREATE TABLE datasets        (name TEXT PRIMARY KEY, created_at TEXT NOT NULL);
CREATE TABLE dataset_members (dataset TEXT NOT NULL, g INTEGER NOT NULL,
                              ord INTEGER, PRIMARY KEY (dataset, g));
```

mirrored into the meta-graph as `quipu:Dataset` / `quipu:includesGraph`.
Resolution is a small change inside `apply_dataset`'s resolve closure: an IRI
that names a dataset expands to its members, so `FROM <urn:na:dataset:…>` and
the `graph` query param both work. A dataset's label is the fold over its
members — the reusable, nameable lattice element; §4's homomorphism is what
makes that well-defined.

Datasets overlap freely and neither contains the other:

```text
dataset:play-thinker   = {datalinks:smac, datalinks:thinker, doctrine, memory:durable}
dataset:audit-canonical = {datalinks:smac, doctrine}
```

That is Alexander's semilattice — the city, not the tree. The branch tree
(`parent_branch`, overlay resolution) and the dataset semilattice (overlapping
membership) are **different relations over the same node set**, and conflating
them is the failure the essay names.

Two naming and semantics notes: the concept is called **dataset** (SPARQL's
own word, resolved by `apply_dataset`) and not "view" (`src/graph_view.rs`
owns that word). And the ROOT-alone default survives: a dataset is never
implicitly active; silence must not widen the dataset.

## 8. Statement-level labels — same vocabulary, second attachment point

Per-graph is the right default granularity (per-triple annotation engines
never left the lab — §9). But two real cases need finer grain: per-edge
`quipu:confidence` (already expressed once, hardcoded, via `rdf:Statement`
reification on the episode path) and a single contested fact inside an
otherwise-trusted graph.

Position: **the same label vocabulary, attachable to a graph IRI or to a
`stmt_` node — one substrate, one governance path** (the shape
[statement-identity.md](statement-identity.md) §6 endorses by rejecting a
second store). A reified statement is four ordinary facts, so statement-level
labels are already bitemporal and already governed, free. Per-graph ships
first; per-edge is gated on statement-identity's Proposal A landing, and on
the namespace report its §8 asks for (the `properties` write path into the
ontology namespace is ungoverned today, and widening it comes second to
fencing it).

The rule that must be stated now so the vocabulary does not fork later:
**downward-only override.** A statement label may mark a fact
`stale`/`contested`/lower-trust inside a fresh graph; it may never raise a
fact above its graph's label. Composition never widens, applied vertically.
Dataset folds ignore statement labels in v1 — they refine *reporting* (row
annotation under `GRAPH ?g`), not the dataset label.

Cost honesty: a qualified edge is ≥5 facts versus ~1 to label a whole graph;
every per-edge label query is a three-way reification join until an RDF-star
surface exists; and `stmt_*` nodes already carry an exclusion tax in
whole-graph scans. Per-graph-first is not a compromise — it is the granularity
the costs argue for, with per-edge as the targeted exception.

## 9. Prior art

The combination is novel; each component has a lineage worth borrowing
vocabulary from rather than re-litigating:

- **Denning, "A Lattice Model of Secure Information Flow" (CACM 1976)** and
  lattice-based access control: labels form a lattice, information flows only
  upward, composition is meet/join. The label half of this design, with fifty
  years of settled semantics — including why the structure must be a lattice
  and not a chain (incomparable labels are the normal case for policy).
- **Carroll, Bizer, Hayes & Stickler, "Named Graphs, Provenance and Trust"
  (WWW 2005):** named graphs as the unit of trust; consumers evaluate graphs
  under task-specific trust policies. Validates the granularity decision —
  trust attaches to the graph, not the triple. What that lineage never had is
  a bitemporal store underneath; time-travelable label history is Quipu's
  differentiator.
- **Annotated RDF / semiring provenance** (Udrea et al.; Zimmermann et al.;
  the SPARQL provenance-semiring line): per-solution annotation through query
  algebra is well-theorized — if ever built here it should follow the
  spm-semiring construction rather than improvising around OPTIONAL/MINUS
  non-monotonicity. And per-*triple* annotation engines stayed research
  systems, which is evidence for per-graph granularity as the default.
- **Bitemporal shape versioning has essentially no prior implementation** —
  see [shape-versioning.md](shape-versioning.md), which is where this stack is
  ahead of the literature rather than behind it.

## 10. Build order

Dependency-ordered; **[U]** unblocks modelling work, **[P]** parallel:

1. **[U]** This document.
2. **[U]** Label columns + reserved meta-graph + migration —
   `set_graph_label`/`label_of`, `quipu doctor labels`. *Consumers can start
   tagging planes the day this lands, before propagation exists.*
3. **[P]** `src/lattice.rs` meet/join algebra + the homomorphism proptest;
   `Authority` implements the trait.
4. Dataset labels on the query path — `query_labeled`, the `"labels"` key.
   Depends on 2, 3.
5. Label floors (opt-in). Depends on 4; config + consumer in the same change.
6. **[P]** Named datasets. Independent of labels until the fold.
7. Per-row labels under `GRAPH ?g` + the precedence `ORDER BY` test. Depends
   on 4, 6.
8. Statement-level labels — normative only, gated on statement-identity.

## 11. Scope boundaries (honest)

- **No provenance semiring.** Labels are per-dataset, per-row only where `?g`
  is already bound. A label does not survive a join or negation with claimed
  precision.
- **Labels are not access control.** A floor refuses a *query*; it does not
  hide rows, and nothing stops a caller who names a graph directly from
  reading it. `aegis:authorityOver` gates **writes only**; a read-side
  authority check does not exist and is not built here. Presenting trust
  labels as a confidentiality boundary would repeat the `group_id` mistake
  this stack already documents.
- **No automatic freshness.** Quipu never observes staleness; a producer
  declares it. No synthesized `fresh` tag, ever.
- **The reasoner still ranges over one graph** and writes back into it, so
  cross-graph derived-label composition never arises. Deliberate: single-graph
  derivation makes derived labels trivially correct.
- **SARC's "no trust predicate" gap: partly closed, and the split matters.**
  Closed: the *propagation* half — imported content in an
  `untrusted`-labelled graph taints every dataset that touches it, and a floor
  can refuse; mechanical and auditable. Not closed: nothing *evaluates*
  content (the label is declared by whoever wrote it); the gap's own text asks
  for "a producer that records sub-agent responses," which still does not
  exist; and per-dataset tainting cannot say *which rows* except under
  `GRAPH ?g`. The honest claim is that the boundary becomes enforceable, not
  that it becomes closed.
- **This does not change any consumer's pipeline ordering** — that is a
  separate, deliberate safety property in NeuralAmplifier and stays where it
  is.

## 12. Related

- [named-graphs.md](named-graphs.md) — the substrate: `g`, the `graphs`
  registry, dataset selection, the ROOT-alone default.
- [multi-db-composition.md](multi-db-composition.md) — labels for *attached*
  graphs; the two designs meet at `graphs.g` being a term id.
- [shape-versioning.md](shape-versioning.md) — the policy layer's time axis.
- [statement-identity.md](statement-identity.md) — the second attachment
  point (§8).
- [group-isolation.md](group-isolation.md) — why labels must not be presented
  as an isolation boundary.
- `src/governance/authority.rs` — the existing meet, to be factored into
  `src/lattice.rs`.

# A Unified Entailment Regime — Plan

> Created: 2026-08-27
> Status: ⚙ CORE IMPLEMENTED (2026-08-27, bead quipu-0b6) — placement per
> Stiwi's recorded decisions: derivations land in reserved-suffix companion
> graphs (`src/store/inferred.rs`; write guard in `transact_to_graph`),
> premises read graph ∪ companion, both engines rerouted, the companion is
> self-describing (graph-level `sourceKind` tag + `derivedAsOfTx` freshness
> note), `FROM <urn:quipu:graph:root>` composes ROOT into a union read, and
> `quipu db migrate-inferred` moves legacy-placed derivations.
> REMAINING under quipu-0b6: the promotion mechanism (§3 — authority-gated
> move + the standing auto-promote policy, which waits on camayoc's
> competency question) and lattice labeling of companions (§1, meet-of-
> premises durability). Prerequisites landed earlier with quipu-923.
> Related: [semantic-reasoning-gaps.md](./semantic-reasoning-gaps.md) (G7),
> [reasoning-engine-fixes.md](./reasoning-engine-fixes.md),
> [named-graphs.md](./named-graphs.md), [graph-labels.md](./graph-labels.md),
> [reasoner.md](./reasoner.md)

## One-Line

One answer to "what is entailed, where does it live, and how fresh is it" —
spanning the three existing engines — with the placement decision made:
derived facts are **materialized into a quarantined inferred plane**, tagged,
and promoted through an authority gate, never written as ordinary facts.

## The decision this document records

Quipu's Datalog reasoner and OWL materializer today write derived facts back
as ordinary facts distinguished only by `source`. Camayoc's ingress discipline
(rule 4: *"inference is quarantined, not banned"*; rule 5: *"facts true at
write time, judgments at read time"*) holds that an inferred fact must never
masquerade as an observed one, and bobbin already runs a quarantine plane
(`crew:inferred`, trust rank 0) for model-inferred facts.

**Decision (Stiwi, 2026-08-27): quarantined materialization.** Reasoner- and
OWL-derived facts are materialized forward — the closure exists and is
queryable — but:

1. they carry `aegis:sourceKind "inferred"`;
2. they land in a designated inferred graph, not the graph of their premises;
3. they enter trusted planes only through authority-gated promotion;
4. consumers opt in to reading them, and can always tell them apart.

Logical entailment is deterministic where LLM extraction is not, but under
this regime that difference governs *promotability* (deterministic closure is
cheap to re-verify and promote), not *placement*. Everything derived starts
quarantined.

## Regime overview

| Question shape | Regime | Freshness | Stored? |
|---|---|---|---|
| Ad-hoc traversal, judgment-shaped reads (contested pairs, liveness, current-end-of-chain) | Query-time property paths | Always current | Never |
| Closure consumers join against repeatedly (subclass membership, transitive `contains`, symmetric/inverse completion) | Quarantined materialization | Delta-driven (reactive) or on-demand; staleness reportable | Inferred plane |
| Counterfactuals | `Store::speculate` (unchanged) | N/A (speculative) | Never |

Property paths remain the sanctioned explicit route and the default answer;
materialization is reserved for closures that are joined against often enough
that per-query recomputation is the wrong cost (the bobbin hot-path rule), or
that must be visible to SHACL/policy evaluation.

## Mechanics to design and build

### 1. The inferred plane

- A per-scope named graph for derived facts, named by convention (align with
  the sibling repos' `crew:inferred`; exact IRI scheme to be settled against
  `src/store/datasets.rs` naming rules). One inferred graph per premise graph
  scope — closure computed over ROOT lands in ROOT's inferred companion;
  closure over a named graph lands in that graph's companion. This preserves
  the named-graphs ruling: entailment never crosses a graph boundary, and an
  overlay's inferred companion cannot forge reachability in the parent.
- Datasets (`src/store/datasets.rs`) are the read-side composition mechanism:
  "base + inferred" is a dataset a consumer selects explicitly. Silence never
  widens scope — a plain query sees asserted facts only, exactly as today.
- Label lattice (`src/lattice.rs`, `store/labels.rs`): the inferred graph
  carries a low-trust label. Durability follows camayoc's meet rule (*"a
  derived fact is only as durable as its least durable input"*,
  `what-belongs-in-the-graph.md` §4b): a derived fact's label is the meet of
  its premises' labels, computed at derivation time and re-checked at
  promotion.

### 2. Tagging

- Every materialized fact carries `aegis:sourceKind "inferred"` alongside the
  existing `source` provenance (`reasoner:<rule-id>` / `owl:materialize`),
  which names the *deriver* where `sourceKind` names the *epistemic class*.
  The camayoc SHACL gate closes `sourceKind` to
  `("observed" "declared" "inferred")`; quipu writes conform rather than
  extending the enum.
- `explain` ([reasoning-engine-fixes.md](./reasoning-engine-fixes.md) Phase 6)
  is the promotion audit's evidence: a fact is promotable when its derivation
  tree bottoms out in facts of acceptable trust.

### 3. Promotion

> **Implementation note (2026-08-27), found while scoping — the reason this
> section is not yet built.** Promotion-as-move interacts with
> re-derive-and-diff: a fact moved out of the companion into the premise
> graph becomes a PREMISE, and on the next evaluation the engine re-derives
> it — the companion's per-source diff no longer contains it, so it would be
> re-asserted there, recreating the two-copies-at-two-trust-levels hazard
> the move decision exists to avoid. The OWL materializer's seen-set already
> absorbs this (premises include the promoted fact); the Datalog
> `write_rule_delta` does not — it must learn to skip tuples already current
> in the premise graph. And the retraction half (premise retracted →
> promoted fact retracted) needs the promoted fact to keep a derivation
> marker, or a sweep that re-checks promoted facts against `explain`-style
> re-matching. Design these two together before building either.

- Authority-gated graph move, following camayoc's implemented pattern
  (`scripts/promote_plane.py`, `config/plane-authority.json`, fail-closed) and
  quipu's existing governance surface (`src/governance/authority.rs`,
  `placement.rs`). Promotion includes the retraction half: the fact leaves the
  inferred graph as it enters the target, and a later retraction of a premise
  retracts the promoted fact too (this is where Phase 3 source-aware truth
  maintenance is load-bearing).
- Deterministic closure may get a standing promotion policy (auto-promote
  subclass closure whose premises are all `declared`); model-inferred facts
  never do. The policy is data (a governance policy), not code.

### 4. Freshness contract

- The inferred graph carries a freshness note: which transaction of the
  premise graph its closure reflects. Consumers can compare it to the premise
  graph's head and decide staleness for themselves — reported, never faked,
  matching yupana's tier/freshness discipline (omitted rather than faked when
  unknown).
- Reactive derivation (the `TransactObserver` path) keeps the note current;
  without the observer enabled, `quipu reason` / `POST /reason` refresh it
  on demand.

### 5. Migration from today's behavior

- Existing derived facts (`source = "reasoner:*"`, `"owl:materialize"`) in
  ordinary graphs are identifiable by source and movable by a one-time
  migration: rewrite into the companion inferred graph with `sourceKind`
  added, retract from the original. Bitemporal history is preserved — the
  move is a normal retract+assert, not history rewriting.
- `evaluate.rs` write-back and `Ontology::materialize()` switch their target
  graph to the companion inferred graph. The staged write-path inference
  (`owl_domain_range_inferences()` in `transact_to_graph`) is the delicate
  case: it exists so guards and SHACL see post-inference state atomically.
  Design choice to settle during implementation: stage into the same
  transaction but target the companion graph (guards evaluate the union), or
  keep domain/range staging as-is and scope this regime to the two engines'
  bulk output. Start with the latter — smaller blast radius — and revisit.

### 6. The negative boundary (unchanged by this regime)

Quarantine does not make every judgment storable. Contested pairs, liveness,
currency remain read-time queries — camayoc's *"the judgment must not be
stored"* holds even for a low-trust plane, because these judgments decay with
time rather than with premise retraction, and no truth maintenance can keep
them honest. The regime governs *logical closure over stored facts*, nothing
else.

## What consumers see

- **Default**: unchanged. Asserted facts only, explicit paths still work.
- **Opt-in**: query the "base + inferred" dataset and get closure — yupana's
  catalogue query becomes `?s a aegis:TextRule` against that dataset; bobbin
  drops dual-typing and reads chunk supertypes from closure; SHACL tightening
  validates against the union.
- **Always distinguishable**: `sourceKind` on every derived fact, `explain`
  for its pedigree, the freshness note for its currency.

## Decisions (Stiwi, 2026-08-27 — recorded in the decision artifact)

The open questions are settled; implementation is green-lit ("build it
next"):

1. **Companion-graph naming: reserved IRI suffix.** `<premise>#inferred`
   (a premise IRI already carrying a fragment gets `-inferred` appended
   instead; ROOT's companion is the well-known
   `http://quipu.local/graph/root#inferred`). Chosen for intuitive access —
   the IRI is derivable anywhere without a registry lookup. The collision
   risk that made the registry the original lean is neutralized by making
   the suffix **reserved**: the store refuses external writes to any graph
   IRI carrying it — only the engines populate a companion.
2. **Promotion moves.** The fact leaves the inferred graph as it enters the
   target; the bitemporal log keeps the record.
3. **Standing auto-promotion, narrowly.** A governance policy (data, not
   code) may auto-promote deterministic closure whose premises are all
   declared/observed; camayoc mints the competency question before any
   policy term. Model-inferred facts never qualify.
4. **Scope: bulk engine output only.** The staged write-path domain/range
   inference keeps today's atomic behavior; revisit once the regime is
   proven.

One refinement made at implementation time, recorded rather than silent:
**the `sourceKind "inferred"` tag attaches at the graph level, not per
subject.** Asserting `aegis:sourceKind` on a derived triple's *subject*
would mistag the subject itself (an observed entity does not become
inferred because one triple about it is derived). The companion graph
carries the epistemic marker — its label, plus a meta-graph annotation —
and each fact's transaction `source` still names the deriver.

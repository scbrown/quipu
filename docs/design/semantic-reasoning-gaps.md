# Semantic Reasoning Support — Gap Inventory

> Created: 2026-08-27
> Status: INVENTORY — updated 2026-08-27: gaps G1–G6 and G8 are CLOSED by
> [reasoning-engine-fixes.md](./reasoning-engine-fixes.md) (bead quipu-923);
> G7 (the unified entailment regime) remains open, tracked by quipu-0b6.
> Consumer-side: U6's camayoc chains now have chain-aware stored queries
> (camayoc_supersession_chain_current, camayoc_transitive_blockers,
> camayoc_section_ancestry — the flagship demonstration); U5's "unbuilt
> quipu path cone" premise was stale — the command is implemented
> (src/path/cone.rs; CLI, REST /path/cone, MCP).
> Related: [reasoner.md](./reasoner.md), [named-graphs.md](./named-graphs.md),
> [vision.md](./vision.md) (Decision 4),
> [reasoning-engine-fixes.md](./reasoning-engine-fixes.md),
> [entailment-regime.md](./entailment-regime.md),
> [flagship-use-cases.md](./flagship-use-cases.md)

## One-Line

Quipu already ships three inference subsystems; this document is the measured
inventory of where they fall short, what the consumers (yupana, bobbin,
camayoc) are already paying for the shortfalls, and the house rules any fix
must clear — so the companion plans start from evidence, not aspiration.

## What exists today (the reuse baseline)

Semantic reasoning in quipu is a consolidation problem, not a greenfield one.

1. **Stratified Datalog reasoner** (`src/reasoner/`: parse → ast → stratify →
   compile → evaluate, on `datafrog`). Rules are Horn clauses stored as Turtle;
   derived facts are written back through `Store::transact_to_graph()` with
   `source = "reasoner:<rule-id>"`, making them ordinary bitemporal,
   SHACL-visible facts. Reactive incremental mode via
   `ReactiveReasoner: TransactObserver` (feature `reactive-reasoner`).
2. **OWL 2 RL materializer** (`src/owl.rs`, `owl_parse.rs`), hand-rolled on
   `oxttl`. Extracts subclass, subproperty, disjoint, inverse, functional,
   symmetric, transitive, equivalent-class/-property, domain, range axioms;
   `Ontology::materialize()` writes `source = "owl:materialize"` facts.
3. **SPARQL property paths** (`src/sparql/property_path.rs`): full operator
   coverage (`ZeroOrMore`, `OneOrMore`, `ZeroOrOne`, `Reverse`, `Sequence`,
   `Alternative`, `NegatedPropertySet`). This is the sanctioned route to
   *explicit* inference.
4. **A deliberate ruling against implicit entailment.** Query evaluation
   matches asserted triples only; implicit RDFS subclass widening was removed
   ([named-graphs.md](./named-graphs.md) §6.2–6.3: a path never crosses a
   graph boundary; an overlay must not forge reachability in its parent).
   `src/sparql/rdfs.rs` retains the advisory `withheld_types()` marker that the
   MCP layer surfaces as an inference header.
5. **A write-path inference precedent.** `Store::transact_to_graph()`
   (`src/store/ops.rs`) already stages `owl_domain_range_inferences()` before
   insert, so guards, SHACL, events, and observers see the post-inference write
   atomically.

## Store-side gaps

Each row carries the evidence that it is a lived failure, not a hypothetical.

### G1. `owl:TransitiveProperty` is inert

Parsed into `Axioms.transitive_properties`, counted by `axiom_summary()`,
reported as accepted — and never materialized. The only references to
`transitive_properties` are the declaration, the counter, and the parser.
This is exactly the dead-end shape the code comment at `src/owl.rs:197-208`
documents for the already-fixed `subPropertyOf` case: an axiom the API accepts
and silently drops.

### G2. `owl:equivalentProperty` is inert

Identical situation to G1: parsed, counted, never used in materialization.

### G3. OWL materialization is one-shot and non-fixpoint

Each axiom family runs once over base facts; a type inferred by `rdfs:range`
never feeds the subclass closure in the same pass, and nothing re-runs
materialization when facts change after ontology load. The workaround in
production was to re-express an OWL axiom as a Datalog rule so the reactive
reasoner keeps it live — recorded at
`shapes/aegis-class-subsumption.rules.ttl:20-24`: *"OWL materialization is
ONE-SHOT at ontology load and nothing re-runs it, so that merge was a
snapshot: correct once, then stale."*

### G4. Datalog compiler caps trail the DSL

The parser and stratifier accept more than the compiler will evaluate
(`src/reasoner/compile.rs:160-170`; documented at
`docs/book/src/reference/reasoner.md`):

- binary predicates only;
- max **2 body atoms**, sharing **exactly one** variable;
- no constants in body positions, no repeated variables within an atom;
- negation is parsed and stratified but **rejected at evaluation**;
- no aggregation.

These shapes error with `ReasonerError::Unsupported` — the DSL is ahead of the
engine, which reads as a bug to every rule author who hits it.

### G5. No truth maintenance — a recorded wrong fixpoint

Truth maintenance is re-derive-and-diff, and `World::load` reads
`current_facts()` without distinguishing base from derived facts. Mutually
supporting rules (the natural encoding of `owl:equivalentClass`) therefore
reach a **stable non-converging fixpoint under retraction**: each rule's
derived output keeps the other's premise alive. Recorded at
`shapes/aegis-class-subsumption.rules.ttl:29-39` and pinned by
`probe_mutual_class_equivalence_under_retraction` in
`src/reasoner/evaluate_tests.rs`. This is the single most important
correctness constraint on any new reasoning work.

### G6. Ruleset freshness: startup snapshot, no REST route

The reactive reasoner's ruleset is loaded once at server startup
(`src/server.rs:188-208`); rules loaded via `POST /shapes` take effect only
after a restart — flagged in-repo as having *"bitten this workstream five
times"* (`aegis-class-subsumption.rules.ttl:10-13`). There is no REST
`/reason` route; reasoning is CLI + MCP + library only.

### G7. No unified entailment regime

Three engines, three answers to "what is entailed": query-time paths (always
current, never stored), OWL materialization (stale after load, stored as
ordinary facts), Datalog derivation (reactive if enabled, stored as ordinary
facts). They differ in graph scoping, freshness, and provenance conventions,
and no surface reports which regime produced a triple.

### G8. No `explain` / derivation trace

Provenance is in the fact log (`source = "reasoner:<rule-id>"` /
`"owl:materialize"`), and [reasoner.md](./reasoner.md) Phase 5 calls a
derivation trace *"cheap to build once the reasoner exists"* — but no CLI,
MCP, or REST surface walks it.

## Consumer-side needs (the use-case inventory)

Ranked by strength of evidence. Every entry is something a consumer already
does the hard way, or a question a competency suite marks unanswerable.

### U1. Subclass entailment for catalogue queries — production incident

`/home/user/yupana/src/project_queries.rs:81-98` hand-rolls
`?s a/rdfs:subClassOf* aegis:TextRule` under a comment titled *"ORDERING IS
LOAD-BEARING"*, recording that binding to the concrete class emptied the
governed text-rule catalogue silently (incident aegis-368cu.4). Quipu
withholding implicit subclass inference is the documented reason the explicit
path exists.

### U2. Bobbin's dual-typing workaround

`bobbin/src/knowledge/chunks.rs:149` asserts `a bobbin:Chunk,
bobbin:CodeSymbol` on every symbol chunk (and both types again for sections)
because nothing infers supertype membership. A subclass hierarchy plus
entailment lets emitters assert the specific type only.

### U3. Type-level policy targeting

`aegis:targets "CodeModule"` names a **type**, not an instance, so the policy
does not bind to anything (open bead `bobbin-567`; also the
`Policy --targets--> <type of foo::bar>` row in yupana's
`governed-relations.md`). Type→instance entailment is the missing half.

### U4. Deferred SHACL tightening

Five `# TIGHTEN LATER: sh:class bobbin:CodeSymbol` comments in yupana
`shapes/code-edges.ttl`, held back because a class constraint would force
every chunked write to carry each touched module's type declaration.
Materialized types across chunked writes unblock all five.

### U5. The provenance cone (camayoc golden-paths Q5)

*"Which steps of a trajectory are inside the provenance cone of its verified
result?"* — defined in camayoc `docs/design/golden-paths.md:129-135` as **the
transitive derivation closure** from terminal Verifications back through step
outputs. It is what makes trajectory pruning *mechanical instead of human*,
and it is currently assigned to an unbuilt `quipu path cone` command
(design-only bead quipu-gp2). The strongest single conceptual case for
transitive reasoning in the stack.

### U6. Transitive chains camayoc cannot answer today

From camayoc's authoritative coverage map (`scripts/coverage_tables.py`):

- **Supersession chains** — `aegis:supersededBy` is a plain edge; finding the
  current end of A→B→C needs `supersededBy+`. The stored query does one hop.
- **Blocked-on chains** — `camayoc_blocked_on_closed_dependency` matches one
  hop; A blocked on B blocked on C (C closed, B open) is invisible.
- **Section containment** — `bobbin:contains` between sections is a nested
  tree; ancestor/descendant questions (doc-structure Q7/Q8) are UNWRITTEN and
  run against *live* data (three repos ingested hourly) — the cheapest place
  to demonstrate a transitivity win.
- **Epic ancestry** — yupana's `WORK_ITEM_PARENT_QUERY` walks one level of
  `aegis:contains`; the full ladder needs `contains+`.
- **Audit chains** — golden-paths Q14 (`constraint ← path ← exemplars`) is an
  explicit multi-hop chain, GAP.

### U7. Symmetric and inverse axioms

- `bobbin:co_changed_with` is symmetric by meaning but written one-way
  (`bobbin/src/knowledge/coupling.rs:99-103`); PPR silently inherits the
  asymmetry.
- Camayoc's vocabulary is uniformly child-points-at-parent (`stepOf`,
  `inSession`, `attributedTo`, …) and none of its 41 stored queries uses `^` —
  every "children of this parent" question is an inverse traversal waiting to
  go wrong.

### U8. Committed-plane reachability (what quipu should own)

Yupana's blast-radius BFS (`src/graph/blast.rs`) stays in yupana — its docs
are explicit (*"borrow, don't derive"*, `governed-relations.md:70-80`; the
blocking pre-edit guard cannot afford a synchronous store query). What only
quipu can answer is the committed/historical half:

- reachability over `bobbin:calls` **as of a past timestamp** (bitemporal);
- **cross-repo / cross-tenant** closure (yupana's graph is per-tenant);
- chains over edges yupana never derives (`imports` chains,
  `Section --references--> CodeSymbol`,
  `Bead ←implements– Commit –modifies→ entity`).

### U9. Bundle closure (bobbin, design-stage)

`bobbin/docs/design/knowledge-aware-bundles.md` builds skill scoping, tool
authorization, role inheritance, and blast radius on transitive closure over
`contains` / `includes` / `depends_on` (depth-N bundle expansion, graph
distance as a ranking signal). All of it presumes closure the store does not
yet offer.

## House rules any design must clear

Stated forcefully in the sibling repos; violating any of them is a regression
even if the reasoning is sound.

1. **No `rdfs:domain`, ever.** A domain axiom on a shared predicate silently
   retypes whatever it touches once a reasoner materializes it. The live case:
   `aegis:stepOrder` is deliberately shared between trajectory Steps and
   workflow WorkflowSteps (camayoc `docs/design/bootstrap-ontology.md:78-81`,
   `ontology/core.ttl:11-12`). RDFS *range-only* vocabularies are shaped to
   survive a reasoner; domain-based typing is the specific hazard.
2. **No ontology term without a competency question that needs it.** Camayoc's
   first convention; bobbin's roadmap (W2.P1) adopts it verbatim. Reasoning
   features that require new terms must arrive with the question that owns
   them.
3. **Inference is quarantined, not banned.** Camayoc ingress rule 4: inferred
   facts carry `aegis:sourceKind "inferred"`, land in a low-trust plane
   (`crew:inferred`), and are promoted only through an authority gate
   (`scripts/promote_plane.py`, `config/plane-authority.json`, fail-closed).
   The SHACL gate (`sh:minCount 1` + closed `sh:in ("observed" "declared"
   "inferred")` on every gated class) refuses an untagged write, and
   `scripts/gate_probe.sh` proves the refusal is real. Bobbin already runs the
   same machinery (`src/knowledge/quarantine.rs`, trust rank 0).
4. **Some judgments are never stored, even quarantined.** Contested decision
   pairs are the canonical case (`coverage_tables.py:248-252`: *"No term is
   missing; the judgment must not be stored"*). Same family: liveness,
   currency, "still in progress" — facts true at write time, judgments at read
   time.
5. **Entailment never crosses a graph boundary.** The named-graphs ruling
   stands: closure is computed within a graph scope; overlays cannot forge
   parent reachability; `GraphScope::AnyNamed` with paths stays refused.
6. **The hot path cannot pay for reasoning.** Bobbin's inject-context runs on
   every prompt; `src/tripwire/cache.rs` exists because a network round-trip
   per keystroke was unacceptable. Whatever regime lands, its cost must be
   off the per-prompt path (materialized ahead of time, or cached like the
   tripwire projection).

## What this inventory is not

- Not a commitment to OWL DL. [vision.md](./vision.md) Decision 4 resolved to
  *"RDFS + SHACL, OWL 2 RL later"* and rejected tableaux reasoning as rarely
  needed; nothing here reopens that.
- Not a replacement for yupana's live graph. The pre-edit hot path stays on
  in-memory structure; quipu owns the committed, bitemporal, cross-repo half
  (U8).
- Not a licence to store judgments. House rules 3 and 4 bound every
  materialization decision; the placement policy is specified in
  [entailment-regime.md](./entailment-regime.md).

## Companion plans

- [reasoning-engine-fixes.md](./reasoning-engine-fixes.md) — closes G1–G6, G8
  inside the existing engines.
- [entailment-regime.md](./entailment-regime.md) — closes G7: one entailment
  story, including the quarantined-materialization placement decision.
- [flagship-use-cases.md](./flagship-use-cases.md) — drives U1–U6 to
  consumer-visible wins.

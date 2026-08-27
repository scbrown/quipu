# Reasoning Engine Fixes — Plan

> Created: 2026-08-27
> Status: ✅ IMPLEMENTED (2026-08-27, bead quipu-923) — all six phases landed
> the same day, each as its own gated commit on main. Verified by mechanism:
> phase 1 `src/owl_materialize.rs` §4b/4c + `owl_tests.rs` regression pair;
> phase 2 the fixpoint loop in `materialize()` + `ReactiveOwl`
> (`src/owl_reactive.rs`, opt-in `owl.reactive_materialize`); phase 3
> `current_facts_for_attributes_in_graph_excluding_sources` +
> `mutual_class_equivalence_converges_under_retraction` (the flipped probe);
> phase 4 `ReactiveReasoner::reload` + `POST /reason` (`src/server/reason.rs`);
> phase 5 the general join pipeline in `src/reasoner/compile.rs` (N-atom,
> multi-variable, stratified negation); phase 6 `src/explain.rs` + CLI
> `quipu explain` + `POST /explain` + MCP `quipu_explain`.
> One deviation from the plan as written: fact placement stays today's
> source-attributed convention pending [entailment-regime.md](./entailment-regime.md).
> Related: [semantic-reasoning-gaps.md](./semantic-reasoning-gaps.md) (G1–G6,
> G8), [reasoner.md](./reasoner.md) (open questions Q1–Q6, Phase 5),
> [entailment-regime.md](./entailment-regime.md) (fact placement)

## One-Line

Close the documented gaps in the two existing forward engines — make the OWL
materializer honest about what it accepts, make it fixpoint and live, lift the
Datalog compiler caps the DSL already promises, fix the recorded
wrong-fixpoint-under-retraction, and give reasoning a live ruleset and an
`explain`.

## Scope and ordering

Phases are ordered so each lands independently and the riskiest correctness
work (truth maintenance) is not blocked behind feature work. **Fact placement
for everything materialized here follows
[entailment-regime.md](./entailment-regime.md)** (quarantined materialization);
where that regime is not yet built, new materialization keeps today's
convention (`source`-attributed ordinary facts) and migrates with the regime —
the fixes here must not invent a third placement.

### Phase 1 — Materialize what is already accepted (G1, G2)

The API accepts `owl:TransitiveProperty` and `owl:equivalentProperty`, counts
them in `axiom_summary()`, and silently drops them. Follow the pattern of the
fixed `subPropertyOf` case (`src/owl.rs:197-208`):

- **Transitive properties**: per-property closure to fixpoint (semi-naive:
  join new pairs against the base relation until no new pairs). Not one join
  pass — `a→b, b→c, c→d` must yield `a→d`.
- **Equivalent properties**: bidirectional subproperty semantics — facts
  asserted under either property materialize under the other. Reuse the
  subproperty copy machinery in both directions; guard against the ping-pong
  re-derivation by idempotent writes (the store's existing idempotency on
  `transact_to_graph` covers this).

Acceptance:

- A 3-link transitive chain materializes its full closure, and re-running
  materialization is a no-op.
- `axiom_summary()` counts equal axioms *used*, or the summary is changed to
  distinguish accepted-and-used from accepted-and-ignored — the silent-drop
  shape is the bug, whichever way it is closed.
- Tests live beside `src/owl_tests.rs` and include the regression shape from
  `owl.rs:197-208` for both new axiom families.

### Phase 2 — Fixpoint and liveness for OWL materialization (G3)

Two halves, both recorded as lived failures:

1. **Fixpoint across axiom families.** Iterate materialization until closure:
   a type introduced by `rdfs:range` inference must feed subclass closure in
   the same run. Bound the loop by monotonicity (materialization only adds
   facts over a finite term universe) and assert termination in tests.
2. **Re-run on change.** Materialization currently happens at ontology load
   only. Add a `TransactObserver` (post-commit, like `ReactiveReasoner`) that
   re-materializes affected axiom families when a delta touches a predicate or
   class the loaded ontologies mention. It must check `delta.source` to skip
   its own writes (the existing observer discipline), and it must be feature-
   gated or config-gated the way `reactive-reasoner` is — reactive OWL is
   opt-in, matching the reasoner's precedent.

Acceptance: the staleness scenario in
`shapes/aegis-class-subsumption.rules.ttl:20-24` — a subclass merge whose
membership goes stale after new members arrive — stays correct without
re-encoding the axiom as a Datalog rule.

### Phase 3 — Truth maintenance: source-aware load (G5)

The minimum viable fix for the recorded wrong fixpoint
(`probe_mutual_class_equivalence_under_retraction`): `World::load` must
distinguish base facts from derived facts. Derivation re-runs from **base
facts only**; previously derived facts are candidates for retraction, never
premises. The `source` column already carries the distinction
(`reasoner:*` / `owl:materialize`); the load path just has to honor it.

- Re-derive-and-diff then converges under retraction for mutually supporting
  rules: retracting the last base premise retracts the whole derived cluster.
- The pinned probe test flips from documenting the failure to asserting the
  fix.
- Full support-set TMS (per-fact justification tracking) stays deferred to
  Phase 5 of [reasoner.md](./reasoner.md); this phase only removes the known
  incorrectness.

Risk note: rules that *intentionally* chain off derived facts (stratified
rule pipelines) still work — stratum N sees stratum N−1's output within a
single evaluation; the exclusion applies to *prior runs'* materialized output
being mistaken for base facts.

### Phase 4 — Live rulesets and a REST route (G6)

- Reload the reactive reasoner's ruleset when rules change: `POST /shapes`
  (and the ontology/shape registry paths in `src/store/registry.rs`) must
  invalidate the ruleset snapshot the same way `invalidate_owl_cache()`
  already invalidates the OWL cache. No restart to pick up a rule.
- Add `POST /reason` to `src/server.rs` for parity with CLI `quipu reason`
  and the MCP surface: body names the ruleset (or inlines rules), the target
  graph, and dry-run vs. write; response reports derived/retracted counts.

Acceptance: the five-times-bitten scenario (`aegis-class-subsumption.rules.
ttl:10-13`) — load a rule over REST, see derivations without a restart.

### Phase 5 — Lift the Datalog compiler caps (G4)

In order of value, stopping where cost outruns demand:

1. **≥3 body atoms and multi-variable joins.** The stratifier and AST already
   carry these; the compiler's two-atom/single-variable plan is the
   bottleneck. Datafrog supports chained joins; compile a left-deep join tree
   with variable-order planning (join on shared variables in sequence).
   Constants in body positions and repeated variables fall out of the same
   generalization (compile to a filter after the join).
2. **Stratified negation.** Parsing and stratification exist; evaluation
   rejects. Semantics follow [reasoner.md](./reasoner.md) Q6: negation as
   failure over the *materialized state of lower strata*, open-world caveats
   documented at the rule DSL. Enable only when stratification proves the
   negated predicate is fully derived in an earlier stratum (already what the
   stratifier checks).
3. **Aggregation stays out of scope** — no recorded demand, and reasoner.md
   scopes it out.

Every lifted cap deletes its `ReasonerError::Unsupported` arm and its
"unsupported" row in `docs/book/src/reference/reasoner.md`, and adds
compile+evaluate tests.

### Phase 6 — `explain` (G8)

A derivation trace over provenance already in the fact log:

- `quipu explain <s> <p> <o>` (CLI), MCP tool, and REST `GET /explain`:
  resolve the fact, read its `source`; for `reasoner:<rule-id>` fetch the rule
  and re-match its body against the graph to list the supporting facts, then
  recurse; for `owl:materialize`, name the axiom family and the premise
  triples; for plain sources, report the transaction and source string.
- Output is a tree (fact ← rule ← premises), depth-capped, with the
  bitemporal coordinates of each node.
- reasoner.md Phase 5 already judged this *"cheap to build once the reasoner
  exists"*; the only design decision is re-derivation (recompute matches on
  demand — no storage cost, chosen here) versus stored justifications
  (deferred with full TMS).

## Cross-cutting requirements

- **SHACL runs after reasoning** (reasoner.md Q3): validation of a transaction
  that includes materialization sees the post-inference state — already the
  behavior of the staged `owl_domain_range_inferences()`; keep it for
  everything added here.
- **Graph scoping**: all new materialization is graph-scoped
  (`evaluate_in_graph` / `current_facts_for_attributes_in_graph` precedent);
  nothing here weakens the named-graphs boundary ruling.
- **Docs and book**: every phase updates `docs/book/src/concepts/owl.md`,
  `concepts/reasoning.md`, and `reference/reasoner.md` in the same change —
  the owl.md history (constraints documented as enforced while `validate()`
  had no caller) is the cautionary tale.
- **Feature flags**: nothing here changes default-on/off posture; `owl` and
  `reactive-reasoner` stay opt-in, `full` aggregates them as today.

## Suggested bead breakdown

One bead per phase, plus one for the cross-cutting doc pass; Phase 1 and
Phase 4 are independent quick wins, Phase 3 is the correctness priority,
Phase 5 is the largest and most separable.

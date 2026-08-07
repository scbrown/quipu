# Design: Quipu paper plan — a governed bitemporal graph store

> **Implementation status (2026-08-07):** ⬜ **Planning.** No paper text
> exists yet. This document fixes the thesis, the contribution list, the
> research questions, the evaluation plan, and the build order for an
> arXiv-style systems paper about Quipu. The model for structure and
> methodology is Rovai, *"Open Ontologies: Tool-Augmented Ontology
> Engineering with Stable Matching Alignment"* (arXiv:2605.09184) and its
> companion repository — we borrow its *method* (research-question tables,
> one evaluator across all conditions, determinism as an artifact, honest
> negative results), not its subject matter.

## Status

- **Date:** 2026-08-07
- **Status:** Planning — nothing measured for the paper yet beyond numbers
  already recorded in existing design docs.
- **Related:** [vision.md](vision.md), [graph-labels.md](graph-labels.md),
  [policy-edit-hooks.md](policy-edit-hooks.md),
  [multi-db-composition.md](multi-db-composition.md),
  [in-memory-read-model.md](in-memory-read-model.md),
  [shape-versioning.md](shape-versioning.md)

## 1. Thesis

Quipu's one-line thesis is already stated in `README.md` and
`docs/design/vision.md`: **"Start strict. Use agents to bear the cost of
strictness."** The paper's claim is the conjunction no existing system
ships in one embeddable store:

- an append-only **bitemporal EAVT log** (`src/schema.rs`),
- **named-graph partitioning** with overlays, tombstones, and datasets
  (`src/store/overlays.rs`, `src/store/datasets.rs`),
- a **label lattice** of freshness/trust/durability/policy whose
  composition never widens (`src/lattice.rs`, `src/store/labels.rs`),
- a **write-time governance plane** producing signed, bitemporal,
  mechanically auditable verdicts (`src/governance/`),

all inside a single SQLite file, embeddable as a crate, servable over
REST and MCP.

### Working titles

1. *Quipu: A Governed Bitemporal Graph Store* (plain, systems-paper)
2. *Start Strict: Write-Time Governance and Label Lattices in an
   Embeddable Bitemporal Graph Store* (thesis-forward)
3. *Governance by Architecture in the Store: Compiling Policy, Trust and
   Time into One Fact Log* (SARC-forward)

Preference: (2). It names the thesis and the two load-bearing
contributions in one line, the way the model paper's title names its
system and its one technical claim.

## 2. Contributions (ranked)

Each contribution must survive the model paper's test: it is either
measured, mechanically checked, or explicitly withdrawn.

### C1 — SARC conformance as a store primitive

SARC (Besanson, arXiv:2605.07728) specifies governance-by-architecture
with an *external* checker over an *exported* trace. Quipu makes Σ (the
policy set), the trace, the verdicts, the authority chains, and the
dispatch inventory all first-class **bitemporal facts in the same store
the policy governs**:

- write-time policy gate evaluated against the pending post-state inside
  the open savepoint (`src/governance/guard.rs`),
- signed verdicts staged and flushed *after* savepoint resolution so a
  denial's verdict survives its own rollback
  (`src/governance/verdict_facts.rs`),
- escalation router where "the hold is the agent retrying, not the
  engine waiting" (`src/governance/router.rs`),
- authority intersection along the call chain, refusal on empty
  (`src/governance/authority.rs`),
- decidable audit `T ⊨ Σ` in `O(|T|·|C|)`, deterministic, never an LLM
  call, reading Σ back from the graph rather than a snapshot
  (`src/governance/audit.rs`),
- the violation vs incompleteness distinction carried through audit,
  inventory, and inheritance rather than collapsed
  (`src/governance/inventory.rs`, `src/governance/inheritance.rs`).

`docs/design/policy-edit-hooks.md` already records that the signed-verdict
machinery is ahead of SARC's own reference artifact (a JSON spec file and
a Python checker). The paper's job is to demonstrate that, not assert it.

### C2 — The label lattice with Coverage

`docs/design/graph-labels.md` and `src/lattice.rs`: freshness/trust
compose by meet, obligations by join, under the single named invariant
**composition never widens**. The two paper-grade ideas:

- **Undeclared is not a lattice value.** A composed label is a pair
  (fold, `Coverage ∈ {full, partial, none}`); partial coverage fails an
  enforcement floor — fail-safe at enforcement, honest at reporting.
  Neither fail-open ⊤ nor floor-dragging ⊥ is correct, and we can show
  adversarially what each alternative mishandles.
- **The homomorphism `label(A ∪ B) = label(A) ⊓ label(B)` is
  machine-checked** (proptest, quipu #66) — connecting Denning (CACM
  1976) and Carroll et al. (WWW 2005) with a bitemporal store underneath
  that neither lineage had.

### C3 — Three orthogonal axes on every fact, with overlays and tombstones

Named graph × valid time × transaction time on one `(e,a,v)` log; the
three-valued `op` with `Tombstone` marking absence in a composed view
without mutating the lower layer (`src/types.rs`); bind-once overlays
that cannot forge presence in a base they were never bound to; datasets
as a semilattice distinct from the branch tree
(`docs/design/named-graphs.md`).

### C4 — Composition and distribution without id rewriting

Term spaces (`s · 2^40 + k`) make ids globally unique *a priori* because
`Ref` payloads are opaque BLOBs SQL cannot rewrite (`src/schema.rs`);
ATTACH read-only layering with its three stated invariants
(`src/store/attach.rs`); knowledge packs that re-intern rather than
row-copy and hash over sorted N-Triples so id assignment cannot affect
identity (`src/pack.rs`).

### C5 — The honest cost model

Not a contribution in the novelty sense but a deliberate section: the
storage linearity (~8.3 KB/episode), the quadratic `eval_bgp` ceiling
with its measured crossover, the TermCache 3.9–4.9× that is explicitly a
constant factor and not the fix, the SHACL flat-cost result that refuted
an inferred model, and the read-model prototype's 133 s → 0.15 ms at
~385 B/fact. See §5 (RQ5).

## 3. Research questions

Following the model paper, the RQ table goes near the top of the paper
and at least one RQ must be a negative result about our own work.

| RQ | Question | Measured where | Status |
| --- | --- | --- | --- |
| RQ1 | What does write-time governance cost, and does the target-type pre-filter keep ungoverned writes at zero overhead? | new `governance_cost` bench over gate on/off × governed-type selectivity | ⬜ bench to build; `examples/shacl_cost.rs` is the template |
| RQ2 | Is `T ⊨ Σ` decidable in-store on real traces, and what fraction of audit outcomes are violations vs honest incompleteness? | `quipu audit` over recorded hank/shantytown traces; compare against SARC's external reference checker | ⬜ needs a trace corpus; audit passes exist |
| RQ3 | Does fail-safe Coverage catch undeclared-label leaks that fail-open (⊤) and floor-dragging (⊥) designs mishandle? | adversarial probe suite over composed datasets, styled on onto-correctness-bench's 300/300 design | ⬜ bench to build; homomorphism proptest exists |
| RQ4 | Does "start strict + agents bear the cost" produce a better graph than "accept and clean later"? | agent-written graphs with gates on/off; acceptance rate of fabricated/untagged facts; retries; final quality against camayoc competency questions | ⬜ needs camayoc's competency runner; the untagged-probe gate test exists |
| RQ5 | Where is the read ceiling and what do the mitigations actually buy? | `examples/scale_bench.rs`, `examples/mem_read_model.rs` | 🟨 measured; needs multi-run + determinism treatment |

RQ5 is the designated negative result: Quipu's SPARQL evaluator is a
nested-loop join and **no query-speed claim against any dedicated engine
(Oxigraph, RDFox, ...) may be made from this repository** — the
contribution is governance, not query speed. This sentence goes in the
paper verbatim, in the spirit of the model repo's withdrawn-claim
post-mortem (`benchmark/reasoner/README.md` there).

## 4. Methodology adopted from the model paper

These are commitments, not aspirations; each becomes a checkable
artifact in the repo.

1. **RQ table up front** mapping question → where measured → answer,
   naming which answers are negative.
2. **One evaluator across all conditions.** Any scored comparison uses a
   single scorer module; if a number is ever corrected, keep a `--legacy`
   flag that reproduces the old number exactly, and show the fix moves
   zero baseline items.
3. **Rescore stored outputs, never re-run inference to re-score.** Agent
   experiment outputs (RQ4) are persisted; scoring is a separate,
   re-runnable pass.
4. **Determinism as a documented artifact.** A `docs/design/`-style note
   recording set-hashes of N repeated runs for every reported number,
   with any divergence diagnosed (the model repo's
   `docs/determinism.md`). Sort every hash-derived traversal.
5. **Deterministic oracle over LLM judge** wherever the property is
   checkable (audit, labels, vocabulary). Where a judge is unavoidable
   (RQ4 graph quality), persist verdict + rationale + judge identity as
   facts — Quipu can store its own judge verdicts bitemporally, which
   the model repo's SQLite `cq_verdicts` table only approximates.
6. **`BUILD_REPORT.md` per benchmark artifact**: what was fetched, how
   synthetic items were constructed, which runs were discarded and why,
   what the claim does not cover.
7. **Adversarial generation from structure, not random sampling** — RQ3
   probes are generated from the lattice structure (cross-chain trust
   pairs, partial-coverage folds), the way the model repo generates
   contradiction probes from compiled class structure.
8. **Single-run and contamination disclaimers** wherever an LLM is in
   the loop.

## 5. Evaluation plan

### Already measured (reusable with multi-run treatment)

- Storage linearity 1k→20k episodes, ~8.3 KB/episode, ingest 315–390
  episodes/s; index share 41.4% (`docs/design/wasm-support.md` §5.1).
- `eval_bgp` quadratic ceiling, 30 s budget crossover ≈ 2,510 episodes;
  TermCache 3.9–4.9× on 2-hop joins
  (`docs/design/in-memory-read-model.md` §8).
- Read-model prototype 133 s → 0.15 ms at ~385 B/fact resident.
- SHACL write cost flat in delta size (`examples/shacl_cost.rs`).
- Label-lattice homomorphism under proptest (quipu #66).

### To build

- `benchmark/` directory (or `examples/` extensions) with one runner per
  RQ, a shared scorer, per-artifact `BUILD_REPORT.md`, and a CI job that
  runs the cheap subset — mirror the model repo's `Makefile`
  bench targets and `benchmark.yml`.
- RQ1 governance-cost bench: writes with 0/1/N policies, governed-type
  selectivity sweep, gate on/off.
- RQ2 trace corpus: record real enforcement traces from hank promotion
  and shantytown event consumption; run all four audit passes; report
  violations vs incompleteness separately; port Σ to SARC's reference
  checker for the comparison condition.
- RQ3 adversarial label suite: N clean composed datasets + N with one
  undeclared graph, one cross-chain trust pair, one expired label;
  score fail-safe vs simulated fail-open and floor-dragging policies.
  Target the 300/300-style clean separation with zero false positives
  on the clean set.
- RQ4 strictness experiment: same ingestion task (e.g. one of the
  sibling repos, or NeuralAmplifier's `smac:` planes) run by the same
  agent against a gated and an ungated store; oracle = camayoc
  competency suites + the untagged-probe refusal; blocked on camayoc's
  competency runner existing.
- Determinism note covering every number above.

### Sibling repos as evaluation substrate

The stack gives the paper real workloads instead of synthetic ones:
hank promotion is a real SHACL-validated writer (RQ1/RQ2 traffic);
shantytown's event subscription is a real consumer; NeuralAmplifier's
three-plane `smac:` graph is the motivating case for the label lattice
and a natural RQ4 subject; camayoc's competency questions are the
acceptance oracle; bobbin's ingest supplies scale.

## 6. Paper outline

1. **Introduction** — the thesis; why write-time strictness inverts the
   usual "capture now, govern later" posture; agents as the actor that
   makes strictness affordable.
2. **Related work** — start from the surveys already written:
   `docs/design/vision.md` (Jena, Oxigraph, Stardog, RDFox, TypeDB,
   Graphiti, TerminusDB, open-ontologies) and
   `docs/design/graph-labels.md` §9 (Denning 1976; Carroll/Bizer/Hayes/
   Stickler 2005; annotated-RDF semiring provenance). Add SARC
   (arXiv:2605.07728), Open Ontologies (arXiv:2605.09184), Datomic/XTDB
   for bitemporal EAVT, and Tardygrada (proof-carrying verification
   runtime; its VERIFIED/CONFLICT/UNVERIFIABLE three-valued verdicts and
   weakest-link aggregation are independent convergence on our
   violation-vs-incompleteness and least-confident-leaf rules).
3. **Data model** — C3, C4.
4. **The label lattice** — C2.
5. **The governance plane** — C1.
6. **Evaluation** — RQ1–RQ5.
7. **Honest limits and future work** — see §7.
8. **Conclusion.**

## 7. Scope boundaries (honest)

Stated in the paper, not discovered by reviewers:

- **Rules are not bitemporal.** The data is bitemporal; `shapes` and
  `ontologies` are latest-only, so "which shapes were in force at time
  T" is unanswerable today (`docs/design/shape-versioning.md`). Either
  build shape versioning before submission or carry this as the named
  gap — the plan assumes the latter.
- **No query-speed claims** (RQ5, verbatim sentence in §3).
- **Labels are not access control** — a floor refuses a query, it does
  not hide rows (`docs/design/graph-labels.md` §11).
- **Trust propagation, not trust evaluation** — the boundary predicate
  over imported content is declared and reported, never evaluated.
- **Coverage audit is half-decidable** (`src/governance/audit.rs`).
- **Single-store scale** — no clustering/replication; wasm blocked on
  bundled SQLite.
- **LLM-in-the-loop results are single-model unless stated** and
  contamination-inclusive where public corpora are involved.

## 8. Build order

`[U]` unblocks later items, `[P]` parallel-safe. File beads (`bd`) per
item when work starts; this list is the plan, not the tracker.

1. `[U]` Benchmark skeleton: `benchmark/` layout, shared scorer
   convention, `BUILD_REPORT.md` template, `just bench` subcommand
   (per AGENTS.md subcommand convention), CI cheap-subset job.
2. `[P]` RQ1 governance-cost bench (template: `examples/shacl_cost.rs`).
3. `[P]` RQ3 adversarial label suite.
4. `[P]` RQ5 multi-run + determinism note over the existing benches.
5. `[U]` RQ2 trace corpus — needs a recording run against hank
   promotion (coordinate with hank's `H-DEP`/Phase B state) and the
   SARC reference-checker comparison harness.
6. RQ4 strictness experiment — blocked on camayoc competency runner;
   coordinate with camayoc rather than building the runner here.
7. Decide the shape-versioning question: build it (larger scope, closes
   the paper's biggest gap) or write the gap section. Decision gates the
   outline's §7 but nothing earlier.
8. Draft §3–§5 from the design docs (they are close to camera-ready
   prose already); §6 last, from measured results only.
9. Paper source lives in `docs/paper/` (LaTeX, outside the book and its
   lint globs), built by a `just paper` recipe; arXiv cs.DB primary,
   cs.AI cross-list.

## 9. Related

- [vision.md](vision.md) — thesis, competitive survey, Bobbin
  integration history.
- [graph-labels.md](graph-labels.md) — C2 in full, including prior art.
- [policy-edit-hooks.md](policy-edit-hooks.md) — C1 backlog with
  acceptance criteria; the SARC citation.
- [named-graphs.md](named-graphs.md), [multi-db-composition.md](multi-db-composition.md),
  [knowledge-packs.md](knowledge-packs.md) — C3/C4.
- [in-memory-read-model.md](in-memory-read-model.md),
  [wasm-support.md](wasm-support.md) — measured numbers behind RQ5.
- [shape-versioning.md](shape-versioning.md) — the named gap.
- Model artifacts: arXiv:2605.09184 and its repository
  (`fabio-rovai/open-ontologies`), whose `README.md` RQ table,
  `docs/determinism.md`, per-case-study `BUILD_REPORT.md`, and
  withdrawn-claim post-mortem are the patterns §4 adopts.

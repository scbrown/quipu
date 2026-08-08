# Design: Quipu paper plan — a governed bitemporal knowledge graph store

> **Implementation status (2026-08-08):** ⬜ **Planning, third revision.**
> No paper text exists yet. This revision raises the altitude: the paper
> now contributes a **named contract** (the Governed Store invariants,
> GS1–GS6, extending SARC from the agent loop to the store), with Quipu as
> the reference implementation and **one deterministic lifecycle benchmark
> — Census — whose single run scores every research question.** Successors
> can implement the contract on other substrates and run the same
> benchmark, the way SARC's successors build on SARC.

## Status

- **Date:** 2026-08-08
- **Status:** Planning — nothing measured for the paper yet beyond numbers
  already recorded in existing design docs.
- **Related:** [vision.md](vision.md), [graph-labels.md](graph-labels.md),
  [policy-edit-hooks.md](policy-edit-hooks.md),
  [named-graphs.md](named-graphs.md),
  [shape-versioning.md](shape-versioning.md), and hank's
  `docs/book/src/design/sarc-conformance.md` (the joint conformance map).

## 1. Thesis

**"Start strict. Use agents to bear the cost of strictness."**
(`README.md`, `docs/design/vision.md`.)

SARC (Besanson, arXiv:2605.07728) compiles governance obligations into
four enforcement points in the *agent loop*, bound by invariants whose
joint effect is a decidable audit `T ⊨ Σ`. But SARC stops at the loop:
its Σ lives in a file beside the system, its trace is exported, and the
knowledge the agents act on sits in an ungoverned store underneath.

This paper's claim: **the store is the right compilation target.** When
Σ, the trace, the verdicts, the authority grants, and the data are all
bitemporal facts in one store, enforcement is the write path, audit is a
query, and history is replayable. Partitioning (named graphs) supplies
the unit of authority and trust; the label lattice makes composition of
partitions safe. Strictness — refusing invalid, untagged, unauthorized,
or policy-violating writes at the gate, with agents absorbing the retry
cost — is the operating posture that falls out.

### Working titles

1. *The Governed Store: Compiling Governance into a Bitemporal
   Knowledge Graph*
2. *Start Strict: Store-Level Governance Invariants for Agent-Written
   Knowledge Graphs*
3. *Quipu: A Governed Bitemporal Knowledge Graph Store*

Preference: (1). It leads with the contract — the thing successors
build on — and names the mechanism in the subtitle.

## 2. Contributions

The classic triple: a model, a system, a benchmark.

### C1 — The Governed Store contract (GS1–GS6)

A store-level analogue of SARC's loop invariants. A store is *governed*
iff:

- **GS1 — Gated writes.** No fact enters except through the gate;
  constraint predicates evaluate against the pending **post-state**,
  not the pre-state or the request.
- **GS2 — Verdict permanence.** Every gate outcome — allow, deny,
  unknown — persists as a **signed, bitemporal fact that survives
  rollback** of the write it judges. No signing identity ⇒ no verdict,
  never an unsigned one.
- **GS3 — Partitioned authority.** Authority attaches to partitions;
  delegation only narrows (intersection); empty intersection refuses.
  Changing a partition's standing requires authority over the
  meta-partition, not the partition itself.
- **GS4 — Non-widening composition.** A view composed from partitions
  carries a label no stronger than the fold of its parts; undeclared
  parts degrade coverage — they never strengthen the result, and they
  fail enforcement floors.
- **GS5 — In-store decidable audit.** Σ, T, and verdicts live in the
  store they govern; `T ⊨ Σ` is decidable in `O(|T|·|C|)` without the
  model or its prompts, and violation is never collapsed with
  incompleteness.
- **GS6 — As-of replay.** Every governance decision is reproducible
  against the store *as of its transaction* — the facts, labels, **and
  rules** in force at the time.

Quipu satisfies GS1–GS5 today (`src/governance/`, `src/lattice.rs`);
GS6 is the one gap — rules are latest-only
(`docs/design/shape-versioning.md`) — and closing it is in-plan work
(§6 item 4), so the reference implementation meets its own contract by
submission. The contract is the extension point: it is
substrate-agnostic (nothing in GS1–GS6 names SQLite, RDF, or EAVT), so
a successor can claim it for a property-graph store, a relational
system, or a lakehouse and run the same benchmark.

### C2 — Quipu: the reference implementation

The mechanisms, mapped to invariants:

- Bitemporal EAVT log, three-valued `op`, named graphs, overlays,
  tombstones, datasets, term spaces (`src/schema.rs`, `src/types.rs`,
  `src/store/`) — the substrate for GS2/GS3/GS6.
- Write gate evaluating SPARQL ASK against the pending post-state
  inside the open savepoint; target-type pre-filter so ungoverned
  writes run zero checks (`src/governance/guard.rs`) — GS1.
- Verdict staging that survives the denied write's rollback; ed25519
  signatures against a human-authored root of trust
  (`src/governance/verdict_facts.rs`, `signing.rs`) — GS2.
- Authority intersection over named graphs; escalation router whose
  hold is the agent retrying (`authority.rs`, `router.rs`) — GS3.
- The label lattice: meet for freshness/trust, join for obligations,
  Coverage for the undeclared, homomorphism
  `label(A ∪ B) = label(A) ⊓ label(B)` under proptest
  (`src/lattice.rs`, quipu #66) — GS4.
- Audit passes preserving violation vs incompleteness; dispatch
  inventory; attribution tree; replay harness (`audit.rs`,
  `inventory.rs`, `tree.rs`, `replay.rs`) — GS5.
- Shape/ontology versioning (to build) — GS6.

### C3 — Census: one lifecycle, every question

A single deterministic benchmark — named for the quipu's original job —
that exercises all six invariants in one recorded run and scores every
RQ from the same artifacts. See §4.

## 3. Research questions

Each RQ is one invariant-cluster, one number, one arm of the same
Census run. Ground truth is free because the scenario is scripted: the
injector knows every defect it planted.

| RQ | Uniqueness claimed | Question | Metric (from the Census run) |
| --- | --- | --- | --- |
| RQ1 | Post-state gating with zero-cost abstention (GS1) | Does enforcement cost scale only with governed writes? | per-write latency, gated vs control arm; overhead on ungoverned writes (target ≈ 0) |
| RQ2 | Refusal-with-feedback beats accept-and-clean (strictness) | Does the gated store end cleaner than the ungated one, at what retry cost? | planted defects present in final graph: gated (target 0) vs control (all land); retries consumed |
| RQ3 | Σ and T live in the store they govern (GS2/GS5) | Does in-store `T ⊨ Σ` decide identically to SARC's external checker, and what does in-store add? | agreement with `besanson/sarc-governance` checker on exported Σ/T; violation/incompleteness counts vs planted ground truth |
| RQ4 | Coverage-aware lattice composition (GS3/GS4) | Are all widening attempts refused and all clean compositions admitted? | adversarial probes refused m/m; clean probes passed n/n; false refusals (target 0) |
| RQ5 | Bitemporal Σ, T, labels, and data in one store (GS6) | What fraction of historical decisions replays bit-identically as-of its transaction? | replay fidelity across the mid-run rule amendment; pre-GS6 the pre-amendment window fails, post-GS6 target 100% |

Every metric is a count or a latency — no judge, no rubric. The one
optional LLM arm (RQ2 with a real agent instead of the scripted writer)
is an extension, not the core result.

## 4. The Census benchmark

One scripted, seeded, multi-writer lifecycle; one command
(`just bench census`); outputs a trace, a final store, and one metrics
JSON per RQ. No LLM in the core loop — the writers are deterministic
drivers, so the whole run is a deterministic oracle.

**Cast.** A handful of recorder identities with different authority
grants and trust-chain positions; one human decision role (scripted);
two stores (the census store and a provincial pack to import).

**Timeline (six phases, mirroring real census mechanics):**

1. **Founding** — register partitions (districts), writers, authority
   grants, trust chains, shapes, and Σ. GS3 setup.
2. **Recording** — writers assert facts across districts. The injector
   plants labeled defects: untagged facts, out-of-authority writes,
   policy-violating writes, fabricated vocabulary. Gated arm refuses
   each with a signed verdict; control arm (gate off) lets everything
   land. GS1, GS2 → RQ1, RQ2.
3. **Correction** — retractions, supersessions, a promotion between
   trust planes, one escalation that mints a `DecisionRequest` and gets
   a scripted human `Decision`. GS2, GS3.
4. **Composition** — import the provincial pack, ATTACH a read-only
   layer, run composed queries; adversarial composition probes
   (undeclared graph, cross-chain trust pair, expired label, partial
   fold) alongside clean ones. GS4 → RQ4.
5. **Amendment** — Σ and one shape change mid-run; recording continues
   under the new rules. This is what makes GS6 non-trivial: decisions
   from phase 2 must replay under the *old* rules. → RQ5.
6. **Audit** — all audit passes in-store; Σ/T exported to the SARC
   reference checker; as-of replay of every verdict. GS5, GS6 → RQ3,
   RQ5.

**Reproducibility.** Deterministic seed; sorted traversals; set-hash of
the final store and of the trace published in the determinism note; the
control arm is the same script with one flag. `BUILD_REPORT.md` records
the defect catalogue and every discarded design.

**Realism anchors.** Census is synthetic by design (that is what makes
it an oracle), but its shape is taken from the stack's real traffic:
hank promotion (governed writer), shantytown's subscriber (consumer),
NeuralAmplifier's three-plane trust precedence (the lattice's motivating
case). A short "Census-in-the-wild" subsection replays the hank
promotion trace through the same audit to show the benchmark's shape is
not a strawman.

## 5. Paper outline

Conventional knowledge-graph systems paper:

1. **Introduction** — the governance gap for agent-written graphs; the
   strict-at-write thesis; C1–C3.
2. **Background and requirements** — SARC's constraint model and
   decidable audit; bitemporal models; named graphs; what multi-writer
   agent ingestion demands of a store.
3. **The Governed Store contract** — GS1–GS6, each with its failure
   mode when absent (motivating example per invariant).
4. **Quipu: data model and mechanisms** — bitemporal EAVT, partitions,
   lattice, governance plane; how each mechanism discharges its
   invariant.
5. **Implementation** — SQLite substrate, savepoints, signing,
   ATTACH/packs, serving surfaces; engineering characterization
   (storage linearity, gate microbenchmarks) as context, no
   comparative query-performance claims.
6. **The Census benchmark** — scenario, defect catalogue, oracle
   construction, reproducibility.
7. **Evaluation** — RQ1–RQ5 from the Census run + Census-in-the-wild.
8. **Related work** — bitemporal stores (Datomic, XTDB); named-graph
   provenance and trust (Carroll et al., WWW 2005);
   information-flow lattices (Denning, CACM 1976); annotated
   RDF/semiring provenance; SHACL and validation-centric stores;
   governance for agentic systems (SARC and successors); LLM-driven KG
   construction (arXiv:2605.09184, arXiv:2411.09601) as the workload
   motivating strictness. Survey base: `vision.md`,
   `graph-labels.md` §9.
9. **Conclusion** — the contract as the extension point.

## 6. Build order

`[U]` unblocks later items, `[P]` parallel-safe. File beads (`bd`) per
item when work starts.

1. `[U]` Write the GS1–GS6 statement precisely (one page, each
   invariant with its failure mode) — it drives both §3 of the paper
   and the Census defect catalogue.
2. `[U]` Census skeleton: `benchmark/census/` with the scripted
   timeline, defect injector, seed discipline, metrics emitters,
   `just bench census`, `BUILD_REPORT.md`.
3. `[P]` Census phases 1–4 (scores RQ1, RQ2, RQ4 immediately; RQ3's
   in-store half).
4. `[U]` **Shape/ontology versioning** (`shape-versioning.md`) — the
   GS6 feature; unblocks phase 5/6 fully.
5. RQ3 external arm: exporter for Σ/T to the `besanson/sarc-governance`
   checker format.
6. RQ5 as-of replay scoring across the amendment boundary.
7. Census-in-the-wild: replay a recorded hank-promotion trace through
   the audit.
8. `[P]` Optional RQ2 agent arm (real agent vs scripted writer, camayoc
   competency oracle) — extension section only.
9. Draft §3–§6 from this doc and the design docs; §7 last, from
   measured results only.
10. Paper source in `docs/paper/` (LaTeX, outside the book's lint
    globs), `just paper` recipe; venue class: knowledge-graph /
    data-systems (ISWC resources/systems, or arXiv cs.DB + cs.AI).

## 7. Evaluation hygiene

1. One scorer per comparison, shared across arms; corrected numbers
   keep a `--legacy` flag reproducing the old value.
2. Deterministic oracle for the core; the optional agent arm persists
   judge verdict + rationale + identity as bitemporal facts.
3. Determinism note: set-hashes of repeated runs for every reported
   number; sort every hash-derived traversal.
4. `BUILD_REPORT.md` per artifact: input provenance, synthetic-item
   construction, discarded runs, what the claim does not cover.
5. Adversarial probes derived from structure (the lattice, the
   authority graph), not random sampling.

## 8. Scope boundaries (honest)

- **No comparative query-performance claims**; storage and gate costs
  are characterized in §5 as engineering context only.
- **Labels are not access control** — a floor refuses a query, it does
  not hide rows (`graph-labels.md` §11).
- **Trust propagation, not trust evaluation** — the boundary predicate
  over imported content is declared and reported, never evaluated.
- **Coverage audit is half-decidable** (`src/governance/audit.rs`); the
  paper states which passes are total.
- **No Action-Time Monitor** — the ATM belongs to the executing
  harness, not the store; the contract deliberately scopes to what a
  store can guarantee.
- **Census is synthetic by construction** — that is what makes it an
  oracle; the in-the-wild replay bounds, but does not eliminate, the
  external-validity gap.
- **Single-store scale** — no clustering/replication.

## 9. Related

- [vision.md](vision.md) — thesis and competitive survey.
- [graph-labels.md](graph-labels.md) — GS4 in full, prior art in §9.
- [policy-edit-hooks.md](policy-edit-hooks.md) — governance backlog,
  SARC citation and conformance pointers.
- [named-graphs.md](named-graphs.md),
  [multi-db-composition.md](multi-db-composition.md),
  [knowledge-packs.md](knowledge-packs.md) — GS3 substrate and
  composition.
- [shape-versioning.md](shape-versioning.md) — the GS6 feature.
- hank `docs/book/src/design/sarc-conformance.md` — joint conformance
  map and gap list.
- SARC: Besanson, arXiv:2605.07728; reference artifacts at
  `besanson/sarc-governance` (RQ3's comparison arm).
- Evaluation-hygiene lineage: practices in §7 adapted from the
  benchmarking discipline of arXiv:2605.09184's companion repository
  (`fabio-rovai/open-ontologies`).

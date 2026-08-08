# Design: Quipu paper plan — a governed bitemporal knowledge graph store

> **Implementation status (2026-08-08):** ⬜ **Planning, revised.** No paper
> text exists yet. This revision reframes the plan around the three things
> the paper is about — **governance, bitemporality, and strictness** — and
> the two mechanisms that deliver them: **named-graph partitioning** and the
> **label lattice**. The intellectual anchor is SARC (Besanson,
> arXiv:2605.07728): the paper's claim is that SARC-style
> governance-by-architecture can be compiled into the store itself. The
> earlier draft's structural mimicry of arXiv:2605.09184 is dropped; the
> paper follows the conventional shape of knowledge-graph systems papers.
> We keep that project's evaluation hygiene (§5) without inheriting its
> outline.

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

SARC argues that governance controls attached to prompts, dashboards, or
post-hoc documentation are structurally mismatched with obligations that
must constrain *execution*: it compiles constraints into four enforcement
points in the agent loop, bound by invariants whose joint effect is a
decidable audit `T ⊨ Σ`. SARC stops at the agent loop, and its reference
artifact keeps Σ in a JSON file beside the system it governs.

Quipu's claim goes one step further: **the knowledge graph store is the
right compilation target for governance.** When Σ, the trace T, the
verdicts, the authority chains, and the dispatch inventory are all
first-class bitemporal facts in the same store the policy governs:

- the Pre-Action Gate becomes the **write path itself** — a denied write
  never becomes a fact, and its signed verdict survives the rollback
  (`src/governance/guard.rs`, `src/governance/verdict_facts.rs`);
- the audit reads Σ **from the graph**, not from a snapshot beside it
  (`src/governance/audit.rs`);
- **bitemporality** makes "what was known, and what was allowed, at time
  T" a query rather than an archaeology project;
- **partitioning** (named graphs) gives governance its unit of authority,
  trust, and isolation — who may write where, what a graph's contents are
  worth, which graphs a query may see;
- the **label lattice** makes composition of partitions safe: freshness
  and trust meet, obligations join, and the single invariant —
  *composition never widens* — is machine-checked
  (`src/lattice.rs`).

Strictness is what falls out: the store refuses invalid, untagged,
unauthorized, or policy-violating writes at the gate, and agents — not
curators — absorb the retry cost.

### Working titles

1. *Quipu: A Governed Bitemporal Knowledge Graph Store*
2. *Start Strict: Compiling Governance into a Bitemporal Knowledge Graph*
3. *Governance as a Store Primitive: Bitemporal Facts, Partitioned Trust,
   and Decidable Audit in Quipu*

Preference: (2). It names the thesis and the SARC lineage ("compiling")
in one line.

## 2. Contributions

Four, one per pillar; each is either measured or mechanically checked.

### C1 — Governance compiled into the store (SARC as store primitives)

The constraint object `⟨src, class, pred, verif, resp⟩ + θ` is an
`aegis:Policy` in the graph, SHACL-validated at definition time,
including the class↔placement discipline of SARC §4.2 that SHACL core
cannot state (`src/governance/placement.rs`). Enforcement is not a layer
over the store; it is the write path: claims run as SPARQL ASK against
the pending post-state inside the open savepoint, authority intersects
along the call chain and refuses on empty (`src/governance/authority.rs`),
escalation mints a `DecisionRequest` whose hold is the agent retrying —
not the engine waiting (`src/governance/router.rs`), and every outcome is
an ed25519-signed verdict against a human-authored root of trust
(`src/governance/signing.rs`). The audit `T ⊨ Σ` is decided in
`O(|T|·|C|)`, deterministically, never by an LLM, with the
violation-vs-incompleteness distinction preserved end-to-end through
audit, inventory, inheritance, and replay (`src/governance/`).
Conformance is documented jointly with hank
(`hank/docs/book/src/design/sarc-conformance.md`).

### C2 — Bitemporality as the governance substrate

Transaction time × valid time on an append-only EAVT log
(`src/schema.rs`) is usually sold as time-travel for data. Here it is
what makes governance auditable: verdicts, decisions, escalations, and
authority grants are bitemporal facts, so the auditor can replay a past
decision with the evidence that existed *then*; label expiry is
`valid_to` on the label assertion — an expired label is absent, not
false (`docs/design/graph-labels.md`); promotion moves a fact between
graphs as a bitemporal, reversible, auditable event. The one place the
store is not yet bitemporal — shapes and ontologies are latest-only
(`docs/design/shape-versioning.md`) — is exactly the gap RQ5 targets,
and closing it is in scope for the paper (§6, item 5).

### C3 — Partitioning: named graphs as the unit of authority and trust

Named graph × valid time × transaction time as three orthogonal axes on
one fact log; overlays bind once to a parent and cannot forge presence
in a base they were never bound to; tombstones mark absence in a
composed view without mutating the lower layer (`src/types.rs`,
`src/store/overlays.rs`); datasets name arbitrary graph-sets as a
semilattice distinct from the branch tree (`src/store/datasets.rs`).
Authority is graph-scoped — relabelling requires authority over the
meta-graph, not the graph being labelled — which is what turns
partitioning from a namespacing feature into the governance unit.
Term spaces and ATTACH/pack composition extend the same partition
discipline across store boundaries without id rewriting
(`src/store/attach.rs`, `src/pack.rs`).

### C4 — The label lattice: safe composition of partitions

Freshness and trust compose by meet, obligations by join, under the one
named invariant **composition never widens**. Undeclared is not a
lattice value: a composed label is a pair (fold, Coverage), and partial
coverage fails an enforcement floor — fail-safe at enforcement, honest
at reporting. The homomorphism `label(A ∪ B) = label(A) ⊓ label(B)` is
machine-checked by proptest (quipu #66). Trust ranks are comparable only
within a declared chain; cross-chain comparison is an error naming both
chains. Precedence is data (`quipu:trustRank`), so promotion is a graph
move, not a rank edit. (`src/lattice.rs`, `src/store/labels.rs`,
`docs/design/graph-labels.md`.)

## 3. Research questions

| RQ | Pillar | Question | Measured where |
| --- | --- | --- | --- |
| RQ1 | Strictness (cost) | What does write-time enforcement cost, and does the target-type pre-filter keep ungoverned writes at zero marginal cost? | new `governance_cost` bench: gate on/off × policy count × governed-type selectivity; `examples/shacl_cost.rs` is the template |
| RQ2 | Strictness (value) | Does a gated store produce a better graph than accept-and-clean-later, with agents absorbing the retry cost? | same agent, same ingestion task, gated vs ungated store; oracle = camayoc competency suites + the untagged-probe refusal; measure acceptance of fabricated/untagged facts, retries, final quality |
| RQ3 | Governance | Can specification–trace correspondence `T ⊨ Σ` be decided in-store on real traces, preserving violation vs incompleteness — and how does it compare to SARC's external reference checker on the same Σ and T? | `quipu audit` over recorded hank-promotion and shantytown-consumer traces; port Σ/T to the checker at `besanson/sarc-governance` for the comparison arm |
| RQ4 | Partitioning + lattice | Does composition never widen, adversarially? Do fail-safe Coverage floors refuse what fail-open (⊤) and floor-dragging (⊥) designs mishandle, with zero false refusals on clean compositions? | adversarial suite generated from lattice structure: undeclared graphs, cross-chain trust pairs, expired labels, partial folds; homomorphism proptest as the base case |
| RQ5 | Bitemporality | Can the auditor decide `T ⊨ Σ` *as of* a past transaction — replaying a decision against the facts, labels, and rules in force at the time? | as-of replay over the RQ3 corpus; facts/verdicts/labels already bitemporal; **requires shape/ontology versioning** (§6 item 5) for the rules half |

RQ5 is the forcing function for shape versioning: without it, "the rules
in force at time T" is unanswerable and the replay degrades to
facts-only. Building it is in the plan, not in the limitations section.

## 4. Paper outline

The conventional shape of a knowledge-graph systems paper (system paper
with a formal core), not a benchmark paper:

1. **Introduction** — the governance gap for agent-written knowledge
   graphs; the strict-at-write thesis; contributions C1–C4.
2. **Background and requirements** — SARC's constraint model,
   enforcement points, and decidable audit; bitemporal data models
   (transaction/valid time); named graphs; what regulated, multi-writer,
   agent-driven ingestion demands of a store.
3. **Data model** — bitemporal EAVT, three-valued `op`, named graphs,
   overlays and tombstones, datasets, term spaces (C2 substrate, C3).
4. **The label lattice** — axes, meet/join asymmetry, Coverage, the
   homomorphism, authority over the meta-graph (C4).
5. **The governance plane** — compiling constraint objects into the
   write path; verdict ordering under rollback; escalation; authority
   intersection; the audit passes; inventory and replay (C1).
6. **Implementation** — SQLite substrate, savepoints as speculation,
   signing, ATTACH/pack composition, serving surfaces (crate, CLI,
   REST, MCP); microbenchmarks (storage linearity, gate cost) reported
   here, in context, as engineering characterization.
7. **Evaluation** — RQ1–RQ5.
8. **Related work** — bitemporal stores (Datomic, XTDB); named-graph
   provenance and trust (Carroll/Bizer/Hayes/Stickler, WWW 2005);
   information-flow lattices (Denning, CACM 1976); annotated
   RDF/semiring provenance; validation-centric stores and shape
   languages (SHACL); governance for agentic systems (SARC and its
   successors); LLM-driven ontology/KG construction (arXiv:2605.09184,
   arXiv:2411.09601) as the workload that motivates strictness; survey
   base already written in `vision.md` and `graph-labels.md` §9.
9. **Conclusion.**

Related work goes late, per systems-paper convention; §2 carries only
the background the reader needs to parse §3–§5.

## 5. Evaluation hygiene

Commitments, each a checkable artifact in the repo:

1. **One scorer per comparison**, shared across all conditions; any
   corrected number keeps a `--legacy` flag reproducing the old value.
2. **Deterministic oracle wherever the property is checkable** (audit,
   labels, vocabulary); where a judge is unavoidable (RQ2 quality),
   verdict + rationale + judge identity are persisted as bitemporal
   facts in the store itself.
3. **Determinism note**: set-hashes of repeated runs for every reported
   number; sort every hash-derived traversal.
4. **`BUILD_REPORT.md` per benchmark artifact**: provenance of inputs,
   construction of synthetic items, discarded runs and why, what the
   claim does not cover.
5. **Adversarial generation from structure** (RQ4 probes derive from
   the lattice, not random sampling).
6. **Real workloads over synthetic**: hank promotion as the governed
   writer, shantytown's subscriber as the consumer, NeuralAmplifier's
   three-plane `smac:` graph as the lattice's motivating case, camayoc
   competency questions as the acceptance oracle, bobbin ingest for
   scale.

## 6. Build order

`[U]` unblocks later items, `[P]` parallel-safe. File beads (`bd`) per
item when work starts; this list is the plan, not the tracker.

1. `[U]` Benchmark skeleton: `benchmark/` layout, shared-scorer
   convention, `BUILD_REPORT.md` template, `just bench` subcommand,
   CI cheap-subset job.
2. `[P]` RQ1 governance-cost bench.
3. `[P]` RQ4 adversarial lattice suite.
4. `[U]` RQ3 trace corpus: record hank promotion + shantytown
   consumption; port Σ and T to SARC's reference checker for the
   comparison arm.
5. `[U]` **Shape/ontology versioning** (`shape-versioning.md`) — the
   one new store feature the paper requires; gates RQ5's rules half.
6. RQ5 as-of replay harness (facts half can start after item 4; rules
   half after item 5).
7. RQ2 strictness experiment — coordinate with camayoc on the
   competency runner; the untagged-probe gate test already exists
   there.
8. Draft §3–§5 from the design docs (close to camera-ready prose
   already); §7 last, from measured results only.
9. Paper source in `docs/paper/` (LaTeX, outside the book and its lint
   globs), `just paper` recipe; target venue class: knowledge-graph /
   data-systems (ISWC resource/systems track, or arXiv cs.DB with
   cs.AI cross-list).

## 7. Scope boundaries (honest)

Stated in the paper, not discovered by reviewers:

- **No comparative query-performance claims.** Quipu's evaluator is a
  nested-loop join over SQLite; storage and gate costs are
  characterized in §6 (Implementation) as engineering context, and no
  benchmark against dedicated SPARQL engines is run or implied. The
  paper's evaluation is about governance, composition, and audit.
- **Labels are not access control** — a floor refuses a query, it does
  not hide rows (`graph-labels.md` §11).
- **Trust propagation, not trust evaluation** — the boundary predicate
  over imported content is declared and reported, never evaluated.
- **Coverage audit is half-decidable** (`src/governance/audit.rs`); the
  paper states which passes are total and which are not.
- **No Action-Time Monitor** — of SARC's four enforcement points the
  stack implements PAG, PAA, and ER; the ATM is out of scope for the
  store (it belongs to the executing harness) and the paper says so.
- **Single-store scale** — no clustering/replication.
- **LLM-in-the-loop results (RQ2) are single-model unless stated.**

## 8. Related

- [vision.md](vision.md) — thesis and competitive survey.
- [graph-labels.md](graph-labels.md) — C4 in full, prior art in §9.
- [policy-edit-hooks.md](policy-edit-hooks.md) — C1 backlog, SARC
  citation and conformance pointers.
- [named-graphs.md](named-graphs.md),
  [multi-db-composition.md](multi-db-composition.md),
  [knowledge-packs.md](knowledge-packs.md) — C3.
- [shape-versioning.md](shape-versioning.md) — the feature RQ5 forces.
- hank `docs/book/src/design/sarc-conformance.md` — the joint SARC
  conformance map and gap list (ATM, θ calibration, W_q, trust
  predicate).
- SARC: Besanson, arXiv:2605.07728; reference artifacts at
  `besanson/sarc-governance` (the RQ3 comparison arm).
- Evaluation-hygiene lineage: the practices in §5 are adapted from the
  benchmarking discipline of arXiv:2605.09184's companion repository
  (`fabio-rovai/open-ontologies`).

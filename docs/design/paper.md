# Design: Quipu paper plan — a governed bitemporal knowledge graph store

> **Implementation status (2026-08-08):** ⬜ **Planning, fourth revision.**
> No paper text exists yet. This revision settles the intent: a
> **system-first paper** — Quipu as evidence that knowledge graphs can be
> better than they conventionally are, now that agents write them. The
> spine is a four-row comparison (conventional defaults vs Quipu's
> inversions), measured by one deterministic benchmark (Census). The
> Governed Store invariants GS1–GS6 remain, demoted from headline
> contribution to a one-page design-principles section; promoting the
> contract to a standalone framework paper is explicitly future work,
> the SARC-then-successors pattern.

## Status

- **Date:** 2026-08-08
- **Status:** Planning — nothing measured for the paper yet beyond numbers
  already recorded in existing design docs.
- **Related:** [vision.md](vision.md), [graph-labels.md](graph-labels.md),
  [policy-edit-hooks.md](policy-edit-hooks.md),
  [named-graphs.md](named-graphs.md),
  [shape-versioning.md](shape-versioning.md), and hank's
  `docs/book/src/design/sarc-conformance.md` (the joint conformance map).

## 1. Intent and thesis

**Intent.** Show, with a working system, that the conventional knowledge
graph is mis-designed for the agent era — and that the fix is a store
that is strict, bitemporal, partitioned, and governed from the inside.

**Thesis.** "Start strict. Use agents to bear the cost of strictness."
(`README.md`, `docs/design/vision.md`.)

**The spine: four conventional defaults, four inversions.**
Conventional knowledge graph stores — Jena, Neo4j, Oxigraph, and even
temporal stores like Datomic/XTDB — share defaults that made sense when
humans curated the graph and stop making sense when agents write it:

| # | Conventional default | Quipu's inversion | Mechanism | Census measures |
| --- | --- | --- | --- | --- |
| D1 | **Accept, then clean.** Validation is optional, post-hoc, or app-layer; the store trusts its writers. | **Refuse at the gate.** No fact enters except through a gate whose predicates evaluate on the pending post-state; agents absorb the retry cost. | write gate + SHACL + placement (`src/governance/guard.rs`) | RQ1 (cost), RQ2 (value) |
| D2 | **One time axis, or none.** You can query what is true; rarely what-was-believed-when; never what-was-allowed-when. | **Bitemporal everything** — data, labels, verdicts, decisions; history is replayable, expiry is absence. | tx × valid time on EAVT (`src/schema.rs`) | RQ5 (as-of replay) |
| D3 | **Flat trust.** A canonical fact and an LLM guess are the same row. | **Partitioned trust.** Named graphs are the unit of authority, trust, and freshness; composition folds labels and never widens; undeclared degrades coverage. | named graphs + label lattice (`src/lattice.rs`, `src/store/labels.rs`) | RQ4 (composition) |
| D4 | **Governance outside.** Policy lives in dashboards, prompts, or middleware; audit means grepping logs. | **Governance inside.** Σ, the trace, and signed verdicts are facts in the store they govern; `T ⊨ Σ` is a query. | governance plane (`src/governance/`) | RQ3 (audit) |

The paper's claim is the conjunction: each inversion exists somewhere in
the literature; no store ships all four, and the four reinforce each
other (the gate is trustworthy because verdicts are bitemporal; the
lattice is enforceable because partitions gate authority; the audit is
decidable because governance is data).

### Working titles

1. *Quipu: A Governed Bitemporal Knowledge Graph Store*
2. *Start Strict: A Knowledge Graph Store for the Agent Era*
3. *Refuse at the Gate: Rethinking Knowledge Graph Defaults for
   Agent-Written Knowledge*

Preference: (1) — plain systems-paper naming; (2) as subtitle material.

## 2. Contributions

1. **The system.** Quipu: a single-file, embeddable knowledge graph
   store implementing all four inversions — bitemporal EAVT with
   three-valued `op`, named-graph partitioning with overlays/tombstones/
   datasets/term spaces, a machine-checked label lattice with Coverage,
   and an in-store governance plane with signed verdicts, escalation,
   authority intersection, and a deterministic `T ⊨ Σ` audit.
2. **The design principles.** GS1–GS6 (§3): a one-page statement of
   what a store must guarantee before agent-written knowledge can be
   trusted, each principle paired with the failure mode of the
   conventional default it replaces. Framed as principles distilled
   from building Quipu, not as a proposed standard.
3. **The benchmark.** Census (§4): one deterministic multi-writer
   lifecycle whose single seeded run measures all four inversions
   against planted ground truth, plus an in-the-wild replay.

## 3. Design principles (GS1–GS6)

One page in the paper; each principle names the conventional failure it
prevents. Substrate-agnostic on purpose — nothing below names SQLite,
RDF, or EAVT — but presented as distilled experience, with the
standalone contract paper left as future work.

- **GS1 — Gated writes.** No fact enters except through the gate;
  predicates evaluate against the pending **post-state**. *(Prevents
  D1's accept-then-clean debt.)*
- **GS2 — Verdict permanence.** Every gate outcome — allow, deny,
  unknown — persists as a **signed, bitemporal fact surviving
  rollback** of the write it judges. *(Prevents unauditable refusals:
  the denials are exactly the record a conventional store throws
  away.)*
- **GS3 — Partitioned authority.** Authority attaches to partitions;
  delegation only narrows; empty intersection refuses; relabelling
  requires authority over the meta-partition. *(Prevents flat-trust
  escalation, D3.)*
- **GS4 — Non-widening composition.** A composed view carries a label
  no stronger than the fold of its parts; undeclared parts degrade
  coverage and fail enforcement floors. *(Prevents silent trust
  laundering through views, D3.)*
- **GS5 — In-store decidable audit.** Σ, T, and verdicts live in the
  store they govern; `T ⊨ Σ` decides in `O(|T|·|C|)` without the model
  or prompts; violation is never collapsed with incompleteness.
  *(Prevents governance-by-log-grepping, D4.)*
- **GS6 — As-of replay.** Every decision reproduces against the facts,
  labels, **and rules** in force at its transaction. *(Prevents D2's
  unanswerable "what was allowed when".)*

Quipu satisfies GS1–GS5 today (`src/governance/`, `src/lattice.rs`);
GS6 requires shape/ontology versioning
(`docs/design/shape-versioning.md`), which is in-plan work (§6 item 4)
so the system meets its own principles by submission.

## 4. The Census benchmark

One scripted, seeded, multi-writer lifecycle; one command
(`just bench census`); outputs a trace, a final store, and one metrics
JSON per RQ. No LLM in the core loop — the writers are deterministic
drivers, so the whole run is a deterministic oracle: the injector knows
every defect it planted, and every metric is a count or a latency. (The
name: recording censuses is what quipus were for.)

**Cast.** Recorder identities with different authority grants and
trust-chain positions; one scripted human decision role; two stores
(the census store and a provincial pack to import).

**Timeline (six phases):**

1. **Founding** — register partitions (districts), writers, authority
   grants, trust chains, shapes, Σ.
2. **Recording** — writers assert facts; the injector plants labeled
   defects (untagged, out-of-authority, policy-violating, fabricated
   vocabulary). Gated arm refuses each with a signed verdict; control
   arm (gate off) lets everything land. → RQ1, RQ2.
3. **Correction** — retractions, supersessions, a trust-plane
   promotion, one escalation minting a `DecisionRequest` answered by a
   scripted human `Decision`.
4. **Composition** — import the provincial pack, ATTACH a read-only
   layer, composed queries; adversarial probes (undeclared graph,
   cross-chain trust pair, expired label, partial fold) alongside clean
   ones. → RQ4.
5. **Amendment** — Σ and one shape change mid-run; recording continues
   under the new rules, so phase-2 decisions must replay under the
   *old* ones. → RQ5.
6. **Audit** — all audit passes in-store; Σ/T exported to the SARC
   reference checker; as-of replay of every verdict. → RQ3, RQ5.

**Reproducibility.** Deterministic seed; sorted traversals; set-hashes
of final store and trace published in a determinism note; the control
arm is the same script with one flag; `BUILD_REPORT.md` records the
defect catalogue and discarded designs.

**Census-in-the-wild.** A short subsection replays a recorded
hank-promotion trace through the same audit, bounding the
external-validity gap of a synthetic scenario. The stack supplies the
realism anchors: hank promotion (governed writer), shantytown's
subscriber (consumer), NeuralAmplifier's three-plane trust precedence
(the lattice's motivating case).

## 5. Research questions

Each RQ scores one inversion from the same Census run.

| RQ | Inversion | Question | Metric (from the Census run) |
| --- | --- | --- | --- |
| RQ1 | D1 (cost) | Does enforcement cost scale only with governed writes? | per-write latency, gated vs control; overhead on ungoverned writes (target ≈ 0) |
| RQ2 | D1 (value) | Does the gated store end cleaner than the ungated one, at what retry cost? | planted defects in final graph: gated (target 0) vs control (all land); retries consumed |
| RQ3 | D4 | Does in-store `T ⊨ Σ` decide identically to SARC's external checker, and what does in-store add? | agreement with the `besanson/sarc-governance` checker; violation/incompleteness counts vs planted ground truth |
| RQ4 | D3 | Are all widening attempts refused and all clean compositions admitted? | adversarial probes refused m/m; clean probes passed n/n; false refusals (target 0) |
| RQ5 | D2 | What fraction of historical decisions replays bit-identically as-of its transaction? | replay fidelity across the amendment boundary; pre-GS6 the pre-amendment window fails, post-GS6 target 100% |

The one optional LLM arm (RQ2 with a real agent instead of the scripted
writer, camayoc competency suites as oracle) is an extension, not the
core result.

## 6. Paper outline

Conventional knowledge-graph systems paper:

1. **Introduction** — agents now write knowledge graphs; the four
   conventional defaults and their failure modes; the comparison table;
   contributions.
2. **Background and requirements** — what multi-writer agent ingestion
   demands of a store; SARC's constraint model and decidable audit;
   bitemporal models; named graphs.
3. **Design principles** — GS1–GS6, each with the conventional failure
   it prevents (§3 above, tightened to a page).
4. **Quipu: data model and mechanisms** — bitemporal EAVT, three-valued
   `op`, partitions/overlays/tombstones/datasets/term spaces, the label
   lattice, the governance plane; each mechanism tied to its principle
   and its comparison-table row.
5. **Implementation** — SQLite substrate, savepoints as speculation,
   signing, ATTACH/packs, serving surfaces (crate, CLI, REST, MCP);
   engineering characterization (storage linearity, gate
   microbenchmarks) as context; no comparative query-performance
   claims.
6. **The Census benchmark** — scenario, defect catalogue, oracle
   construction, reproducibility.
7. **Evaluation** — RQ1–RQ5 + Census-in-the-wild.
8. **Related work** — bitemporal stores (Datomic, XTDB); named-graph
   provenance and trust (Carroll et al., WWW 2005); information-flow
   lattices (Denning, CACM 1976); annotated RDF/semiring provenance;
   SHACL and validation-centric stores; governance for agentic systems
   (SARC and successors); LLM-driven KG construction (arXiv:2605.09184,
   arXiv:2411.09601) as the workload motivating strictness. Survey
   base: `vision.md`, `graph-labels.md` §9.
9. **Conclusion and future work** — the principles as a candidate
   contract for other substrates (the standalone framework paper).

## 7. Build order

`[U]` unblocks later items, `[P]` parallel-safe. Tracked as beads under
the `paper` label (`bd list -l paper`): skeleton `quipu-zg0` → phases
1–4 `quipu-y41` → {external arm `quipu-4mi`, replay `quipu-tj0` (also
needs GS6 `quipu-krv`), in-the-wild `quipu-0u4`, agent arm `quipu-yr5`,
determinism `quipu-02v`} → drafting `quipu-q89`; plus vocabulary policy
`quipu-64q` and LaTeX scaffold `quipu-418`.

1. `[U]` ☑ Write the comparison table and GS1–GS6 precisely (the
   paper's §1 table + §3 page); it doubles as the Census defect
   catalogue. Done: [paper-principles.md](paper-principles.md).
2. `[U]` Census skeleton: `benchmark/census/`, scripted timeline,
   defect injector, seed discipline, metrics emitters,
   `just bench census`, `BUILD_REPORT.md`.
3. `[P]` Census phases 1–4 (scores RQ1, RQ2, RQ4; RQ3's in-store half).
4. `[U]` **Shape/ontology versioning** (`shape-versioning.md`) — the
   GS6 feature; unblocks phases 5–6 fully.
5. RQ3 external arm: Σ/T exporter to the `besanson/sarc-governance`
   checker format.
6. RQ5 as-of replay scoring across the amendment boundary.
7. Census-in-the-wild: replay a recorded hank-promotion trace through
   the audit.
8. `[P]` Optional RQ2 agent arm (real agent vs scripted writer).
9. Draft §3–§6 from this doc and the design docs; §7 last, from
   measured results only.
10. Paper source in `docs/paper/` (LaTeX, outside the book's lint
    globs), `just paper` recipe; venue class: knowledge-graph /
    data-systems (ISWC resources/systems, or arXiv cs.DB + cs.AI).

## 8. Evaluation hygiene

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

## 9. Scope boundaries (honest)

- **No comparative query-performance claims**; storage and gate costs
  appear in §5 (Implementation) as engineering context only. The
  comparison with conventional stores is about *defaults and
  guarantees*, not throughput.
- **Labels are not access control** — a floor refuses a query, it does
  not hide rows (`graph-labels.md` §11).
- **Trust propagation, not trust evaluation** — the boundary predicate
  over imported content is declared and reported, never evaluated.
- **Coverage audit is half-decidable** (`src/governance/audit.rs`); the
  paper states which passes are total.
- **No Action-Time Monitor** — of SARC's enforcement points the store
  implements PAG, PAA, ER; the ATM belongs to the executing harness.
- **Census is synthetic by construction** — that is what makes it an
  oracle; the in-the-wild replay bounds, but does not eliminate, the
  external-validity gap.
- **Single-store scale** — no clustering/replication.

## 10. Related

- [vision.md](vision.md) — thesis and competitive survey.
- [graph-labels.md](graph-labels.md) — the lattice in full, prior art
  in §9.
- [policy-edit-hooks.md](policy-edit-hooks.md) — governance backlog,
  SARC citation and conformance pointers.
- [named-graphs.md](named-graphs.md),
  [multi-db-composition.md](multi-db-composition.md),
  [knowledge-packs.md](knowledge-packs.md) — partitioning and
  composition substrate.
- [shape-versioning.md](shape-versioning.md) — the GS6 feature.
- hank `docs/book/src/design/sarc-conformance.md` — joint conformance
  map and gap list.
- SARC: Besanson, arXiv:2605.07728; reference artifacts at
  `besanson/sarc-governance` (RQ3's comparison arm).
- Evaluation-hygiene lineage: practices in §8 adapted from the
  benchmarking discipline of arXiv:2605.09184's companion repository
  (`fabio-rovai/open-ontologies`).

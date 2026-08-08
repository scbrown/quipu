# Design: The defaults comparison and the Governed Store principles

> **Implementation status (2026-08-08):** 🟨 **Drafted for the paper.**
> This is build-order item 1 of [paper.md](paper.md): the precise
> statement of the four-row comparison table (paper §1) and the GS1–GS6
> design principles (paper §3), doubling as the **Census defect
> catalogue** (§4 below) that `benchmark/census/` will implement. Every
> mechanism claim is cited to source; the two places where a probe
> requires machinery Quipu does not yet ship are flagged ⚠ inline
> rather than hidden.

## Status

- **Date:** 2026-08-08
- **Status:** Draft for paper §1/§3; input contract for the Census
  injector.
- **Related:** [paper.md](paper.md), [graph-labels.md](graph-labels.md),
  [policy-edit-hooks.md](policy-edit-hooks.md),
  [named-graphs.md](named-graphs.md),
  [shape-versioning.md](shape-versioning.md).

## 1. The comparison table

The paper's claim is a conjunction: each inversion exists somewhere in
the literature; no store ships all four; and the four reinforce each
other. Precision matters most in the *default* column — each default is
stated as the behavior of named systems, so the table is checkable, not
a strawman.

### D1 — Accept, then clean → Refuse at the gate

**The default, precisely.** Conventional stores treat validation as
optional, post-hoc, or the application's job. Neo4j's constraint
surface is uniqueness/existence/type on properties, not
domain-semantic; Jena and RDF4J ship SHACL as a *separate API call* the
writer may invoke after loading; Oxigraph loads any well-formed RDF. In
all of them a writer with write access can insert a fact that violates
the domain's own rules, and cleanup is a later, human-driven pass.

**The failure mode.** With agent writers the accept-then-clean debt
compounds: an LLM writes plausible-but-wrong facts faster than any
curation pass drains them, and downstream consumers read the store in
the window between write and clean.

**The inversion.** No fact enters except through a gate; the gate
evaluates against the pending **post-state**; refusal carries
structured feedback; the agent — not a curator — absorbs the retry.

**Mechanism.** Write-time SHACL (`shacl.validate_on_write`,
episode-scoped), OWL checks (`owl.validate_on_write`), and the policy
gate: `aegis:Policy` claims run as SPARQL ASK against the datums
already staged in the open savepoint, indexed by target type so a
write touching no governed type runs zero ASKs
(`src/governance/guard.rs`).

### D2 — One time axis, or none → Bitemporal everything

**The default, precisely.** Neo4j and the RDF stores have no temporal
model — history is whatever the application journals. Datomic keeps
transaction time; valid time is a user-schema convention. XTDB is
honestly bitemporal — *for data*. In no conventional store are the
**labels, verdicts, decisions, or rules** time-indexed: you can
sometimes ask "what was true at T", never "what was trusted at T" or
"what was allowed at T".

**The failure mode.** Governance questions are historical questions
("why did this write pass in March?"). A store that keeps only current
trust and current policy cannot answer them, and the audit degrades to
log archaeology outside the data model.

**The inversion.** Transaction time × valid time on every fact — and
verdicts, decisions, authority grants, and labels *are* facts. Label
expiry is `valid_to` on the label assertion: an expired label is
absent, not false (`docs/design/graph-labels.md`). The one exception
Quipu still carries — shapes and ontologies are latest-only
(`docs/design/shape-versioning.md`) — is exactly GS6's gap, closed by
in-plan work.

**Mechanism.** EAVT log with `tx` and `valid_from`/`valid_to`, current
state = `op=1 AND valid_to IS NULL` (`src/schema.rs`); three-valued
`op` with `Tombstone` (`src/types.rs`).

### D3 — Flat trust → Partitioned trust

**The default, precisely.** RDF datasets have named graphs and SPARQL
has `FROM`/`FROM NAMED`, but composition is *silent union*: merging
graphs has no label semantics, no notion that one graph is canonical
and another is an LLM's guess, and no refusal when provenance is
undeclared. Property-graph stores don't even have the partition
primitive. Where trust appears at all it is per-user access control,
which governs who may *read*, not what the data is *worth*.

**The failure mode.** Trust laundering through views: one query joins
an attested graph with a quarantined one and the result inherits the
prestige of the attested source. Nothing errors; the widening is
invisible.

**The inversion.** Named graphs are the unit of authority, trust, and
freshness. Composition folds labels under one invariant — **composition
never widens** — with freshness/trust composing by meet, obligations by
join, and undeclared graphs degrading `Coverage` rather than silently
passing or silently poisoning.

**Mechanism.** The label lattice (`src/lattice.rs`): `Coverage ∈
{Empty, None, Partial, Full}` where `Empty` is the fold identity,
distinct from `None`; partial coverage fails enforcement floors —
fail-safe at enforcement, honest at reporting. Trust ranks compare only
within a declared chain; cross-chain comparison is an error naming both
chains. Labels live as facts in the reserved meta-graph, so relabelling
requires authority over `urn:quipu:graph:meta`, not the graph being
labelled (`src/store/labels.rs`). The homomorphism
`label(A ∪ B) = label(A) ⊓ label(B)` is proptested (quipu #66).

### D4 — Governance outside → Governance inside

**The default, precisely.** Where governance exists for conventional
stores it is perimeter machinery: access control lists, middleware
policy engines, dashboards, prompt instructions. SARC itself — the
strongest statement of governance-by-architecture for agents — keeps Σ
in a JSON file beside the system and audits an *exported* trace with an
external Python checker (arXiv:2605.07728 §3.6).

**The failure mode.** Perimeter governance and the data it governs
drift independently: the policy file says one thing, the store contains
another, and no mechanical check connects them. Audit means grepping
logs that were never designed to be evidence.

**The inversion.** Σ, the trace, the verdicts, the authority chains,
and the dispatch inventory are facts in the store they govern.
Enforcement is the write path; `T ⊨ Σ` is a query; the auditor reads Σ
from the graph, not from a snapshot beside it.

**Mechanism.** The governance plane (`src/governance/`): gate
(`guard.rs`), signed verdicts that survive the denied write's rollback
(`verdict_facts.rs`, `signing.rs`), escalation router (`router.rs`),
authority intersection (`authority.rs`), the audit (`audit.rs`),
dispatch inventory (`inventory.rs`), attribution tree (`tree.rs`),
promotion replay (`replay.rs`).

## 2. The Governed Store principles (GS1–GS6)

The normative statements, each with its precise content and the
conventional failure it prevents. Substrate-agnostic on purpose:
nothing below names SQLite, RDF, or EAVT.

### GS1 — Gated writes

**No fact enters the store except through a gate whose predicates
evaluate against the pending post-state.**

*Post-state, not pre-state and not the request*: a constraint like
"no entity holds two placements" can only be checked after the
candidate datums are staged — a pre-state gate passes a write that is
valid alone and invalid in combination. *Zero-cost abstention*: a
write touching no governed target must incur no evaluation cost, or
strictness becomes an argument against adoption.
(`src/governance/guard.rs` — policies indexed by target-type IRI;
claims ASK the open savepoint.)

*Prevents:* D1's accept-then-clean debt.

### GS2 — Verdict permanence

**Every gate outcome — allow, deny, and unknown — persists as a
signed, time-indexed fact that survives rollback of the write it
judges.**

*The ordering is the content*: a denied write is rolled back, so
verdicts must be staged outside the write's transaction and flushed
after the savepoint resolves — the denial's verdict is precisely the
record worth keeping. *Unknown is recorded, not skipped.* *No signing
identity ⇒ no verdict* — never an unsigned one; signatures verify
against a human-authored root of trust the store cannot mint for
itself. A re-entry guard prevents a policy targeting verdicts from
denying the recording of its own denial.
(`src/governance/verdict_facts.rs`, `src/governance/signing.rs`.)

*Prevents:* unauditable refusal — the conventional store throws away
exactly the events an auditor needs most.

### GS3 — Partitioned authority

**Authority attaches to partitions; delegation only narrows; empty
intersection refuses; changing a partition's standing requires
authority over the meta-partition.**

*Intersection, not union*: along a delegation chain
`auth = ⋂ authority(pᵢ)`; the wildcard is the identity; an empty
intersection is a refusal, never a fallback — this is the defense
against authority escalation through tool capability. *Bind-once
composition*: an overlay binds to its parent at creation and rebinding
is an error, so a layer cannot forge presence in a base it was never
bound to (`src/store/overlays.rs`). *Meta-partition rule*: relabelling
a graph requires authority over `urn:quipu:graph:meta` — otherwise a
tenant promotes itself to `attested`
(`src/governance/authority.rs`, `src/store/labels.rs`).

*Prevents:* D3's flat-trust escalation.

### GS4 — Non-widening composition

**A view composed from partitions carries a label no stronger than the
fold of its parts; undeclared parts degrade coverage — they never
strengthen the result — and degraded coverage fails enforcement
floors.**

*The invariant is "never widens", not "everything meets"*:
freshness and trust fold by meet, obligations by join (a `no-export`
part taints the composed set). *Undeclared is not a lattice value*: the
composed result is a pair (fold, Coverage); `Empty` is the fold
identity, `Partial` fails floors. *Incomparable is an error*: trust
from different declared chains refuses comparison by name, because
silent integer comparison is exactly the bug that ranks a learned
tactic above canon. *Expiry is absence*: an expired label leaves
coverage, it does not become false. (`src/lattice.rs`,
`src/store/labels.rs`.)

*Prevents:* trust laundering through views, D3.

### GS5 — In-store decidable audit

**The specification Σ, the trace T, and the verdicts live in the store
they govern; `T ⊨ Σ` is decidable in `O(|T|·|C|)` without access to the
model or its prompts; violation is never collapsed with
incompleteness.**

*Two severities by construction*: a trace that contradicts Σ
(`Violation`) and a trace that under-determines Σ
(`Incompleteness`) demand different responses — fix the system vs fix
the telemetry — and an audit that merges them invites both being
ignored. The same distinction runs through the dispatch inventory
(ungoverned-without-reason is a violation; ungoverned-with-declared-
reason is an acknowledged bypass surface) and constraint inheritance.
*Σ is read from the graph*, not from a file beside it, so
specification and store cannot drift silently. *Honest edge*: coverage
checking is half-decidable, and the audit says which passes are total.
(`src/governance/audit.rs`, `inventory.rs`, `inheritance.rs`.)

*Prevents:* D4's governance-by-log-grepping.

### GS6 — As-of replay

**Every governance decision is reproducible against the store as of
its transaction — the facts, the labels, and the rules in force at the
time.**

*The rules half is the hard half*: facts and labels are already
bitemporal (GS2 makes verdicts facts), but a store whose shapes and
policies are latest-only can only replay *what was known*, not *what
was required* — and a mid-lifecycle rule amendment makes the
difference observable. This is Quipu's one open gap
(`docs/design/shape-versioning.md`), closed by in-plan work; the
Census amendment phase (§4, phase 5) is designed to fail without it.

*Prevents:* D2's unanswerable "what was allowed when".

## 3. How the principles interlock

The four inversions are not independent features; each principle
borrows its force from another. GS1's gate is worth trusting because
GS2 makes its outcomes permanent evidence. GS2's verdicts are worth
believing because signing verifies against a root of trust GS3's
authority model keeps out of the store's own hands. GS4's lattice is
enforceable because GS3 makes partitions the unit a floor can refuse.
GS5's audit is decidable because GS1–GS2 already produced Σ-shaped
traces and verdicts as data. GS6 is what makes all of it *historical* —
without it, every other guarantee is only about now. This
interlocking is the argument that the paper's contribution is the
conjunction, not the parts.

## 4. The Census defect catalogue

The injector's contract: every probe below is planted at a known point
with a known ground truth, so every RQ metric is a count or a latency.
Gated-arm expectations assume all gates on; the control arm is the same
script with the gate off, where every D1/D3 defect **lands silently**
— that contrast *is* RQ2.

### Phase 2 — Recording (GS1, GS2 → RQ1, RQ2)

| ID | Plants | Gated arm expectation |
| --- | --- | --- |
| CEN-U1 | Fact missing a required provenance tag | refused; SHACL feedback names the missing property; signed deny verdict persists |
| CEN-A1 | Write into a district the writer has no authority over | refused; empty-intersection refusal; deny verdict |
| CEN-A2 | Delegated writer exceeding the delegator's grant | refused; intersection narrows, never widens |
| CEN-P1 | Write violating a policy claim on post-state | refused; deny verdict cites the policy IRI |
| CEN-P2 | Write valid against **pre**-state, invalid only in post-state (e.g. second placement for the same entity) | refused — the probe that separates post-state gating from pre-state gating; a pre-state gate passes it |
| CEN-V1 | Fact using a fabricated predicate in a policed namespace | refused ⚠ — requires a closed-world vocabulary policy in Σ ("every predicate in post-state is declared in the ontology"), expressible as an ASK claim; open-world SHACL alone passes it. Decide at Census build time: ship the vocabulary policy (preferred — it is just another Σ entry) or drop the probe. |
| CEN-N1×k | k clean writes touching no governed type | land; per-write gate overhead ≈ 0 (RQ1's abstention metric) |
| CEN-N2×k | k clean writes touching governed types | land; per-write latency = RQ1's enforcement cost |

### Phase 3 — Correction (GS2, GS3)

| ID | Plants | Expectation |
| --- | --- | --- |
| CEN-E1 | Write requiring escalation; scripted human approves | first attempt refused, `DecisionRequest` minted with evidence hash; retry after `Decision` lands; both verdicts persist |
| CEN-E2 | Escalation the scripted human rejects | retry still refused; rejection outranks approval |
| CEN-R1 | Retraction + supersession of a phase-2 fact | old fact closed (`valid_to`), successor asserted; history intact |
| CEN-R2 | Trust-plane promotion | fact *moves* graphs (bitemporal, reversible); rank facts unchanged |

### Phase 4 — Composition (GS3, GS4 → RQ4)

| ID | Plants | Expectation |
| --- | --- | --- |
| CEN-C1 | Composed dataset including one undeclared graph | fold Coverage = `Partial`; enforcement-floored query refused; report shows partial, not failure |
| CEN-C2 | Trust pair from two different declared chains | comparison error naming both chains; no silent ordering |
| CEN-C3 | Label expired before query time | label absent from fold; coverage degrades accordingly |
| CEN-C4 | One `no-export` graph in an otherwise clean set | composed obligations include `no-export` (join, not meet) |
| CEN-C5×n | n clean compositions with full declarations | pass; **zero false refusals** (RQ4's specificity metric) |
| CEN-C6 | Overlay attempting facts against a base it is not bound to | refused; bind-once (`src/store/overlays.rs`) |
| CEN-C7 | Provincial pack imported into the census store | facts re-interned; sorted-N-Triples content hash identical across both stores (`src/pack.rs`) |

### Phase 5 — Amendment (GS6 → RQ5)

| ID | Plants | Expectation |
| --- | --- | --- |
| CEN-M1 | Post-amendment write valid under old Σ only | refused under amended Σ |
| CEN-M2 | Replay of a phase-2 verdict after the amendment | must evaluate under the **old** rules ⚠ — fails while shapes/Σ are latest-only; the probe that forces shape versioning (GS6). Pre-GS6 expected result: rules half of replay fails for the pre-amendment window; post-GS6 target: bit-identical replay, 100% |

### Phase 6 — Audit (GS5 → RQ3)

| ID | Plants | Expectation |
| --- | --- | --- |
| CEN-G1 | Tool class left ungoverned, no reason declared | audit reports **Violation** |
| CEN-G2 | Tool class ungoverned with `ungovernedReason` | audit reports **Incompleteness** (acknowledged bypass), not violation |
| CEN-T1 | Trace record with no attribution | counted as incomplete; **not** placed at the attribution root |
| CEN-X1 | Full Σ/T export to the external SARC checker (`besanson/sarc-governance`) | verdict-for-verdict agreement on the shared decidable subset; disagreements are the RQ3 finding either way |

**Totals discipline.** The injector emits a manifest (defect ID, phase,
seed offset, expected outcome) as the run's ground truth; every RQ
scorer reads the manifest, never the script. The manifest is committed
with the run's set-hash in the determinism note.

## 5. Open decisions

1. ~~**CEN-V1**: ship the closed-world vocabulary policy as a standard
   Σ entry or drop the probe.~~ **Decided (quipu-64q): shipped as a Σ
   entry.** `urn:census:policy:closed-vocabulary` is a deny policy
   whose ASK claim requires every predicate on a `census:Record` to be
   typed `census:DeclaredPredicate`; the probe is refused in the gated
   arm and lands in the control arm. One wrinkle worth keeping: claims
   must scope their patterns with `GRAPH ?g` when the governed facts
   live in named graphs — a plain BGP judges an empty view
   (`benchmark/census/BUILD_REPORT.md`).
2. **CEN-M2 sequencing**: whether Census lands before shape versioning
   (publishing the honest pre-GS6 failure first, then the fix) or
   after (only the clean 100% result). The former is the stronger
   narrative and matches the house style of measured errata.
3. **How many clean probes** (`k`, `n`) — enough for stable latency
   distributions; set when RQ1 variance is first measured.

## 6. Related

- [paper.md](paper.md) — the plan this document discharges item 1 of.
- [graph-labels.md](graph-labels.md) — lattice semantics, prior art.
- [named-graphs.md](named-graphs.md), [multi-db-composition.md](multi-db-composition.md) —
  composition substrate for phase 4.
- [shape-versioning.md](shape-versioning.md) — the GS6 feature CEN-M2
  forces.
- [policy-edit-hooks.md](policy-edit-hooks.md) — governance backlog and
  SARC conformance pointers.
- SARC: Besanson, arXiv:2605.07728 (§3.6 external checker — CEN-X1's
  comparison arm).

# Performant edit hooks for policy

> **Implementation status (2026-07-23, kelly):** ✅ **Phase A implemented** (Phase B
> is hank-side, separate/blocked). Verified by mechanism: the quipu pre-commit policy
> gate is `src/governance/guard.rs` (`PolicyGuard::build` + `evaluate_write`), invoked
> on the write path via `stage_and_guard` (`src/store/ops.rs`) and runtime-gated by
> `[quipu.governance] enforce_on_write` (`src/store/mod.rs`, `src/config.rs`); the
> read-only `quipu_policy_check` MCP/REST call also exists (REST-only until
> quipu-227 registered it, with the six other governance/overlay tools, in
> `tool_definitions()` — agents can now discover the gate). Phase B (hank-side
> structural-policy projection) is a separate system, blocked on hank↔quipu wiring —
> outside quipu's code.

Status: **in progress** — the quipu write-path gate (Phase A below) is being
implemented under this design. The hank-side projection (Phase B) is
tracked in the backlog and blocked on the Phase-4 hank↔quipu wiring.

## Problem

Quipu carries a rich *declarative* governance vocabulary — a `Policy`
(`shapes/governance.ttl`) `targets` an entity **type**, carries a SPARQL-ASK
`claim`, declares a `boundary ∈ {action (pre-edit), transition}` and an
`effect ∈ {allow, warn, require-approval, deny, escalate, record}`. But the
evaluator (`quipu_policy_check`, `src/mcp/governance.rs`) is a **read-only, on-demand**
MCP/REST call: it is registered as `ro_handler!(policy_check, …)` and nothing on
the write path invokes it. So `boundary:"action"` is declarative *intent* with
no engine binding — an edit that leaves a governed entity non-compliant is
**not** rejected. The only pre-write veto that exists today is episode-scoped
SHACL (`src/episode/mod.rs`, gated by `shacl.validate_on_write`).

We want **edit hooks for policy** that actually enforce, and are **performant**
enough to sit on the write path.

## Deciding principle: evidence locality

Evaluate a policy where its evidence is already hot.

- **Governed-fact policies** — the claim reasons over quipu's committed EAVT
  graph (provenance, prior verdicts, workflow/transition state). Quipu holds
  that evidence. These must evaluate at quipu's **pre-commit gate**.
- **Structural-evidence policies** — the claim reasons over call graph,
  reachability, blast radius, symbols. **Hank** holds that hot, per tenant, at
  the `boundary:"action"` edit-time seam (`hank hook pre-edit` + resident
  daemon). These evaluate in hank, against a hot **projection** of quipu's
  canonical policies.

The dividing line is *where the evidence lives*, not *which system owns policy*.
Canonical policy definitions, SHACL validation, verdict signing, and the
human-owned `VerifierRegistration` root of trust **stay in quipu** regardless;
hank holds only a projected read cache.

## Phase A — quipu pre-commit policy gate (this change)

### Insertion point

The single write choke point is `Store::transact_to_graph`
(`src/store/ops.rs`). Today it opens a `Savepoint` RAII object, inserts the
transaction row + datums, then `sp.commit()`. The RAII savepoint mutably borrows
the connection for its lifetime, which is why `&Store` (needed by the
`&Store`-based SPARQL evaluator) is unavailable mid-transaction.

We switch `transact_to_graph` to **manual `SAVEPOINT`/`RELEASE`/`ROLLBACK TO`**
statements — the exact pattern `speculate()` already uses. With no RAII borrow
held, `&self` is usable *after the datums are inserted but before RELEASE*, so
the guard evaluates policy claims against the **pending post-state** (same
connection sees the uncommitted savepoint rows). On a blocking verdict the guard
returns `Err(PolicyDenied)`; the caller does `ROLLBACK TO` and the write never
lands — no partial mutation, mirroring the episode-SHACL precedent.

### The performant part: a target-type-indexed registry

A naive hook re-`SELECT`s each policy's `claim`/`effect` per edit and runs a
SPARQL ASK for every policy on every write. We avoid that with a
`PolicyRegistry` (`src/governance/guard.rs`):

1. **Built once, cached** on the `Store`. It loads every active
   `boundary:"action"` policy — `{policy_iri, target_type, claim, effect,
   evidence_probe}` — and indexes them by **target-type term id**
   (`HashMap<i64, Vec<CompiledPolicy>>` + a `HashSet<i64>` of target types).
   This removes the "fresh `SELECT` per policy per edit" cost outright.
2. **Pre-filter.** For a write, compute the touched entity ids from the datums,
   query their *post-state* `rdf:type`s (one indexed lookup per touched entity,
   already reflecting this txn's assertions), and intersect with the registry's
   target-type set. **A write that touches no governed type runs zero ASKs** —
   the common path pays only the intersection.
3. **Evaluate** only the surviving `(entity, policy)` pairs: bind `$target` to
   the entity IRI, run the cached claim ASK against the pending state.
4. **Invalidate** the cache when a transaction writes a governance-defining fact
   (attribute ∈ {`targets`,`claim`,`boundary`,`effect`,`evidenceProbe`} or an
   `rdf:type = aegis:Policy` assertion) — detected by integer term-id compare
   over the datums, so non-governance writes never touch the registry.

Follow-up (backlogged): cache a *compiled* claim AST, not just the claim string,
once the SPARQL parser is exposed for reuse.

### Effect semantics (v1)

The `claim` states the **compliant condition** (claim satisfied = good), matching
`quipu_policy_check`'s `satisfied` outcome. At the write gate:

- `deny`, `require-approval`, `escalate` → **block** (fail closed) when the claim
  is **unsatisfied** for a touched target. `require-approval`/`escalate` block
  because a write needing a human decision cannot proceed through a seam that has
  no channel to grant one; refusing is the honest behaviour, versus letting a
  `require-approval` policy pass silently. Recording the pending decision and
  routing it to the workflow layer is the remaining half of `Q-APPROVAL`.
- `allow`, `warn`, `record` → **advisory, non-blocking** (write proceeds, no ASK
  is even run).

### Config & safety

Runtime-gated by `[quipu.governance] enforce_on_write` (default **false**),
mirroring `shacl.validate_on_write`. Default builds and existing deployments are
**unchanged**; enforcement is opt-in. Not behind `reactive-reasoner` (the
existing `TransactObserver` seam is post-commit and cannot veto).

### The tree-sitter-tier catalog (`shapes/policies/treesitter.ttl`)

The canonical, SHACL-validated source of the structural policies Hank projects.
Each is composed from the governance atoms: an `aegis:Selector` (a tree-sitter
`.scm` capture — which nodes) and an `aegis:Predicate` (a regex + an
`aegis:matchType` of `must-match` / `must-not-match` / `must-exist`, plus an
optional `aegis:gate` pre-filter — what their text must be), bound into an
`aegis:Policy` at `boundary:"action"` with `tier "tree-sitter"`. Two shipped
examples: `todo-needs-ticket` (a TODO comment must cite a ticket) and
`no-ticket-in-comment` (the opposite direction). The catalog is validated against
`governance.ttl` in `src/governance_tests.rs::treesitter_policy_catalog_conforms`.

The atom field names line up one-to-one with Hank's `rules::Rule`
(`evidenceSource`↔`query`/`pattern`, `matchType`↔`match_type`, `gate`↔`gate`,
`tier`↔`Tier::TreeSitter`), so the Phase-B projection deserializes a policy
straight into a `Rule` — the seam is a decode, not a redesign. An
`aegis:VerifierRegistration` for the `hank` verifier authorizes it to attest
these predicates when its edit-time verdicts promote back (H-PROMOTE-VERDICT).

## Phase B — hank projection (backlogged, Phase-4-blocked)

Hank holds a **hot, compiled projection** of quipu's `boundary:"action"`
policies whose claims are structural, evaluates them at `hook pre-edit` against
the resident graph, and emits verdicts that promote back into quipu as signed
facts (the existing `commit → touched` promote path). The projection is
**strictly one-directional** (quipu canonical → hank cache) with invalidation on
quipu policy writes; hank never defines policy, only caches it — otherwise hank
could allow what quipu would deny. This leans on hank freshness-serving (FR-3)
so a verdict can declare whether the registry it used was fresh or stale, and on
the hank↔quipu dep being wired (commented out today). See the backlog.

## SARC conformance

The Phase A/B split above is one half of a larger picture. Besanson's SARC
framework — *SARC: A Governance-by-Architecture Framework for Agentic AI
Systems: Compiling Regulatory Obligations into Runtime Constraints*, working
paper, Universidad Torcuato Di Tella,
[arXiv:2605.07728v1](https://arxiv.org/abs/2605.07728) [cs.SE], reference
artifacts at <https://github.com/besanson/sarc-governance> — names four
enforcement points in an agent loop (§4.1): Pre-Action Gate, Action-Time
Monitor, Post-Action Auditor, Escalation Router. Eight invariants (§3.5) bind
them, and their joint effect is a *decidable audit* (Definition 2, §3.6): given
a specification Σ and a trace T, an auditor can mechanically check `T ⊨ Σ` in
`O(|T|·|C|)` without access to the model or its prompts.

Measured against that, quipu's write gate and hank's pre-edit hook together are
a solid PAG and a solid policy-layer reference monitor, and the signed
`aegis:Verdict` + `VerifierRegistration` machinery is ahead of SARC's own
reference artifact (a JSON spec file and a Python checker, §3.6). Note also that
SARC positions itself as a specification discipline layered *over* a
policy-as-code substrate rather than a replacement for one (§2.1) — which is
exactly the relationship this document's Phase A gate already has to Phase B's
projection.

The gaps are specific: the constraint object is under-declared — no
`constraintClass`, no operating point θ, no reversibility window τ_rev, so
`aegis:effect` is carrying both the class and the response and the
class-to-placement rules of §4.2 (Table 3) cannot be checked at all. There is no
PAA, no ATM, no escalation router, and nothing checks correspondence.

The gap analysis and build order live in hank's book, at
`docs/book/src/design/sarc-conformance.md` — the same cross-repo citation style
this document already uses; its Sources section carries the full reference list.
The quipu-side work it implies is listed as `Q-SARC-*` in the backlog below.

## Backlog (beads)

Each entry: rationale + acceptance criteria. `Q-*` = quipu, `H-*` = hank.
Status: ☐ open · ☑ done (this change).

### quipu

- **Q-GATE** ☑ Pre-commit veto seam in `transact_to_graph` (manual savepoint).
  *AC:* a `deny` policy whose claim is unsatisfied for a touched target causes
  `transact` to return `Err` and leaves the store byte-identical to before the
  call; `enforce_on_write=false` is a no-op.
- **Q-REGISTRY** ☑ Target-type-indexed, claim-caching `PolicyRegistry` with a
  touched-type pre-filter. *AC:* a write touching no governed type runs zero
  claim ASKs; policy metadata is not re-`SELECT`ed per edit.
- **Q-INVALIDATE** ☑ Registry cache invalidation on governance-fact writes.
  *AC:* adding/retracting a `Policy` (or its `claim`/`effect`/`targets`) is
  reflected on the next enforced write without a process restart.
- **Q-CONFIG** ☑ `[quipu.governance] enforce_on_write` wired from server
  startup; `PolicyDenied` error → HTTP 403. *AC:* default false; toggling it on
  enforces without code change.
- **Q-APPROVAL** ◐ Partial. `require-approval`/`escalate` now **block** at the
  write gate (fail closed) instead of passing silently. Remaining: record a
  pending-approval `Decision` and route it to the workflow layer so an approver
  can release the write. *AC:* a violated `require-approval` policy blocks the
  write AND records a pending-approval decision.
- **Q-CLAIM-AST** ☐ Cache a compiled claim AST in the registry. Deferred: the
  claim's `$target` is a SPARQL variable currently string-substituted pre-parse,
  and `sparql::eval_pattern` takes no seed bindings — reuse needs a
  seed-binding variant that pre-binds `?target` in the BGP evaluator, a core
  SPARQL-internals refactor. The dominant per-edit costs (metadata `SELECT`,
  ungoverned-type ASKs) are already eliminated, so this is a marginal win on the
  governed path only. *AC:* claim parsing happens once per policy load, not per
  edit.
- **Q-TRANSITION** ☐ `boundary:"transition"` enforcement at step transitions
  (the second half of the boundary enum). *AC:* a transition policy gates its
  workflow step the way `action` gates a write.
- **Q-VERDICT-PERSIST** ☑ Persist the write-gate verdict as a signed, bitemporal
  Verdict fact (reuse `signing.rs`). *AC:* every enforced decision is auditable
  and replayable, not just an accept/reject.

  **The ordering is the whole design.** A denied write is ROLLED BACK, so a
  verdict written inside the same savepoint goes with it — and the denial's
  verdict is exactly the one worth keeping, because an accepted write leaves its
  own evidence in the facts it wrote while a refused one used to leave nothing
  at all. (Since 2026-08-22 / camayoc-0d3 that is no longer literally true:
  every gate refusal is additionally recorded as a `write.refused` event —
  gate, destination graph, actor, terse reason, datum count; never the refused
  bodies — using this same stash-then-record-after-rollback pattern. See
  `src/store/events.rs`. The verdict remains the *judgment* record; the event
  is the *incident-rate* record.)
  Verdicts are therefore STAGED on the `Store` during evaluation and flushed
  afterwards, in their own transaction, once the savepoint has resolved either
  way. `a_denied_write_still_records_its_verdict` is the case.

  Three things travel with it:

  - **`unknown` is recorded, not skipped.** When an evidence probe finds nothing
    to judge, "no evidence yet" and "never evaluated" are different facts, and an
    absent verdict makes the gate look as though the policy did not apply.
  - **No signing identity means NO verdict, never an unsigned one.** A bare
    `satisfied` in the record is forgeable by anyone who can write a fact; the
    point of a verdict is that it is an attestation rather than a claim.
  - **Re-entry is guarded.** Writing a verdict is itself a write the gate would
    evaluate, and a policy targeting `aegis:Verdict` would deny the verdict
    recording its own denial. The recording path sets a flag the gate and the
    placement check both honour — a deliberate hole, narrow: only these facts,
    only for the duration of the write.

  The evidence hash covers `predicate|target|outcome`, **not** the graph state.
  The gate's evidence is a SPARQL ASK over the committed store, which has no
  stable serialisation to hash, and inventing one that moved with unrelated facts
  would make every verdict spuriously stale. That makes this binding narrower
  than hank's, which hashes the edit text it genuinely saw — and saying so is
  better than implying a guarantee the shape of the evidence cannot support.
- **Q-SARC-CLASS** ☑ Complete the constraint object in `shapes/governance.ttl`:
  `constraintClass ∈ {hard,soft,escalation}`, `verificationPoint ∈
  {PAG,ATM,PAA,tool_layer,policy_layer}`, `hostedAtLayer` (no `"prompt"` value —
  I6 unrepresentable by construction), an `OperatingPoint` node shape,
  `reversibilityWindowSeconds` + `onTimeout "deny"`, `latencyBudgetMs`,
  `sourceType`, and `"throttle"` in the `effect` enum. Backfill
  `shapes/policies/treesitter.ttl`. *AC:* an action-boundary policy missing a
  class is rejected at write; the shipped catalog still conforms.
- **Q-SARC-PLACEMENT** ☑ Class↔placement conformance pass
  (`src/governance/placement.rs`), run at definition time: hard ⇒ PAG/ATM/tool/
  policy, soft ⇒ ATM/PAA, escalation ⇒ PAG/PAA and must declare τ_rev. *AC:* a
  soft constraint declared at PAG, or an escalation without τ_rev, fails
  validation.

  Gated by `[quipu.governance] validate_placement`, default **false** —
  independent of `enforce_on_write`, which governs *evaluation* of policies
  where this governs *definition* of them. It runs inside the same savepoint as
  the write gate and returns `PolicyDenied`, so a refused definition leaves the
  graph byte-identical. (Since 2026-08-22 the event log does gain one
  `write.refused` row recording the refusal — metadata only, never the refused
  definition itself.)

  Three things surfaced while building it, all worth carrying forward:

  0. **"Unrepresentable" was an overstatement, now corrected.** `onTimeout` has a
     one-value `sh:in` and `hostedAtLayer` has no `"prompt"` value, and this was
     first written up as making the unsafe settings unrepresentable. A `sh:in`
     enum only binds when SHACL runs, and SHACL runs under
     `shacl.validate_on_write` — default **false**, and scoped to episode ingest
     rather than every `Store::transact`. A policy written through `/knot` could
     carry `onTimeout "allow"` and nothing would object. Both values are now
     re-checked by the placement pass, which IS on the write path, before the
     action-boundary exemption (a bad `onTimeout` fails just as silently on a
     transition-boundary policy). The vocabulary omission is defence in depth
     behind that, not a guarantee above it — and the honest ceiling stays
     "refused on quipu's write path", since a raw SQL write bypasses everything.

  1. **Multi-valued fields are refused, not resolved.** Asserting
     `constraintClass "hard"` over an existing `"soft"` leaves BOTH facts
     active — assertion is not replacement. The first implementation read the
     last row and silently picked one, which would have let a re-class land
     while the old placement still validated. A policy with two classes is
     refused as ambiguous, and the message says to retract the stale value in
     the same transaction. `a_clean_re_placement_retracting_the_old_value_lands`
     is the recoverability half: refusing ambiguity is only safe if there is a
     way to legitimately move a policy.
  2. **The rules are pure but the tests are not.** `placement_tests.rs` has both
     unit cases over `Placement::violation` and liveness cases through
     `Store::transact`, plus the flag-off control that makes the rejection
     attributable to this check rather than to something else on the write path.
- **Q-SARC-VOCAB** ☑ Describe the `aegis:` vocabulary, not only constrain it.
  `shapes/*.ttl` carried **zero** `rdfs:domain` / `rdfs:range` / `rdf:Property` /
  `owl:*Property` declarations before Q-SARC-CLASS: every term was defined
  implicitly by the shapes constraining it, which states validity and not
  meaning. `shapes/aegis-properties.ttl` now covers the SARC fields; this bead
  is the rest.

  Per property: `owl:DatatypeProperty` or `owl:ObjectProperty`, `rdfs:label`,
  `rdfs:range`, and an `rdfs:comment` saying what the term MEANS and how it
  differs from its nearest neighbour — a comment restating the label passes a
  presence check while telling a reader nothing.

  `rdfs:domain` **only where the subject class is unambiguous.** It is an
  inference the reasoner materialises, so declaring it on a generic name
  (`aegis:kind`, `aegis:name`, `aegis:threshold`) silently retypes the first
  unrelated thing in the estate to use that name. Property-at-a-time, in
  shape-file order, never one sweep; each batch runs the reasoner over a store
  holding the shipped catalog and asserts the inferred types are the intended
  ones, since materialisation is the risk. *AC:*
  `every_sarc_property_the_shape_constrains_is_also_described` generalises to
  every `aegis:` property the shape graph mentions, so adding a constrained
  property without a declaration fails the build.

  This is what `src/owl.rs` and the reasoner's domain/range inference were built
  for and have been starved of. It is also the precondition for the two
  consumers already committed to: conversational authoring over the catalog, and
  aligning `aegis:` against an external vocabulary (ontology matching takes
  property descriptions as its primary signal and there are none).

  **As built.** The other 39 governance-plane properties are described, and
  `governance_plane_properties_are_all_described` holds it closed over every
  `sh:path` in `governance.ttl` — with a non-vacuity floor, since an extractor
  that silently found nothing would otherwise pass over an empty set.
  `materialising_the_declarations_types_the_shipped_catalog_correctly` runs the
  reasoner over the shipped catalog and asserts no Selector or Predicate became a
  Policy; verified by mutation, because materialisation is the risk.

  Two things came out differently. **`aegis:gate` carries two meanings** — a
  Predicate's applicability condition, and the gate that produced an
  `aegis:Decision` — and the declaration records that rather than picking one,
  with a test asserting it still does. A reader who meets only one of the two
  will write code assuming it is the only one.

  And the scope is the **governance plane only**. The ~100 estate properties in
  `aegis-ontology.shapes.ttl` (`hostname`, `rig`, `park`, `plexId`) are excluded
  by name in both the file and the test: their intended subjects are not all
  obvious from their shapes, and asserting domains by guess *materialises wrong
  `rdf:type`s* rather than merely documenting badly. That batch is its own bead.

- **Q-SARC-ER** ☑ Escalation router (`src/governance/router.rs`):
  `aegis:OperatorGroup` with an M/M/c capacity model, a DecisionRequest queue,
  hold-until-τ_rev, default-deny on timeout, and re-validation of an
  operator-modified action. *AC:* `require-approval` suspends and routes rather
  than failing closed with no channel; an unserviced escalation denies at τ_rev
  and says so.

  **Asynchronous, and the docs say so.** A write gate is synchronous and cannot
  hold a transaction open while a human decides — that would turn an approval
  gate into a lock on the store. The refused attempt MINTS a `DecisionRequest`
  naming the policy, target, evidence hash and `expiresAt`; a human signs an
  `aegis:Decision` bound to the same hash; the NEXT attempt succeeds. The "hold"
  is the agent retrying, not the engine waiting.

  Same staging problem as the verdicts, same answer: the refusal that opens a
  request also rolls the savepoint back, so requests are staged on the `Store`
  and flushed afterwards. A request written in place would vanish with the
  rollback, leaving an operator a refusal with nothing to act on — the exact
  state the router exists to end.

  Four calls worth keeping:

  - **Only an approval permits.** `Pending` and `Expired` are both refusals.
    Reading either as a pass is the default-allow-under-load failure.
  - **A rejection outranks an approval** when both are bound to one evidence
    hash. Two humans disagreeing is not a state to resolve by row order, and the
    safe reading of a disagreement about permitting something is "no".
  - **A retry updates the same request** — the IRI derives from the evidence, so
    an agent retrying every few seconds does not bury the queue in duplicates.
  - **A zero window expires immediately** rather than defaulting. The placement
    check requires τ_rev on an escalation at definition time, so reaching the
    router without one means that check was off; inventing a bound would be
    inventing exactly what I4 requires be declared.

  Not built, and not implied: no scheduler, no notification. `routedTo` records
  WHICH group should rule; delivering to them is a consumer of this record.
  Claiming otherwise would be the dashboard anti-pattern with extra steps.
- **Q-SARC-AUTHORITY** ☑ Authority intersection over named graphs
  (`src/governance/authority.rs`), SARC I5 §9.3. The gap here was NOT missing
  multi-tenancy: named graphs are already a storage-enforced isolation substrate
  (registry with `committed|overlay` class, bind-once parent branches,
  graph-scoped writes/retracts/idempotency). What was missing was AUTHORIZATION —
  one global bearer token, and nothing saying which principal may write to which
  graph.

  `aegis:Principal` holds `authorityOver` graph IRIs (or `*`). A call chain's
  effective authority is the INTERSECTION of every link's, so a delegate can only
  narrow it and a sub-agent cannot use credentials broader than its caller's
  (the §9.5 authority-escalation-via-tool-capability defence). An empty
  intersection permits nothing and is a REFUSAL, never a fallback to the
  principal's own authority — that fallback is the escalation the rule prevents.

  The wildcard is the identity under intersection: it declines to narrow rather
  than widening, so a single-tenant deployment where everyone holds `*` behaves
  exactly as before, while a `*`-holding orchestrator delegating to a scoped
  worker gets the WORKER's scope.

  Gated by `enforce_authority`, default off, and **inert without a chain**: every
  existing caller sets none, and making attribution a hard requirement beneath a
  running deployment would break all of them at once. The flag makes a supplied
  chain binding, so adoption is per-caller and cannot silently widen. *AC:* a
  write outside the chain's authority is refused at the write path with a reason
  naming the chain, the graph and what is held; the same write lands with the
  flag off; an unattributed write is untouched.

- **Q-SARC-AUDIT** ☑ `quipu_audit_check(Σ, T)` (`src/governance/audit.rs`): the
  four correspondence passes — coverage, class-placement compatibility, outcome
  consistency, attribution completeness — returning a structured discrepancy
  report. Deterministic and never an LLM call. *AC:* a trace whose verdict
  response differs from the policy's declared response is reported, naming the
  action, the constraint, and the violated condition.

  Three things came out of building it that the bead did not anticipate.

  1. **Two severities, and they must not be collapsed.** A *violation* is the
     trace contradicting Σ; an *incompleteness* is the trace not saying enough to
     decide. Report everything as a violation and an operator learns to ignore
     the output; report everything as an incompleteness and a soft constraint
     blocking an edit reads as a formatting note. Only violations set the exit
     code.
  2. **The outcome pass has to be mode-aware.** `advise` has a declared ceiling,
     so a hard `deny` that only warned is correct under `advise` and a violation
     under `enforce`. A mode-blind check would have to be wrong about one of
     those two records.
  3. **Coverage is only half-decidable here.** Asking whether every constraint
     that *applied* was evaluated means re-running the selector against the file
     as it stood, and quipu has neither. What ships is the converse direction
     (nothing cited that Σ does not define) plus vacuity, and the module doc says
     so — a checker reporting "coverage: pass" while testing something weaker
     would be the more dangerous artifact.

- **Q-SARC-INVENTORY** ☑ The dispatch-graph inventory for I7
  (`src/governance/inventory.rs`, `shapes/dispatch-inventory.ttl`). Enforcement
  completeness is a property of the dispatch graph, and hank's
  `work-scoped-governance.md` §"What this cannot reach" stated it as prose —
  which goes stale the first time a harness adds a tool, with nothing to notice.
  As `aegis:ToolClass` facts, a new ungoverned class is a failing check.

  The distinction the bead exists for: an executable class traversing nothing
  with **no stated reason** is an unknown hole (violation); the same class with
  an `aegis:ungovernedReason` is an **acknowledged bypass surface**
  (incompleteness). Neither is governed, and the checker never reports one as the
  other. Plus the cross-check the other way — a constraint placed at a point no
  executable class traverses can never fire. *AC:* the shipped inventory passes
  its own check while still reporting all seven of its ungoverned surfaces; an
  empty inventory is an incompleteness, never a pass.

- **Q-SARC-REPLAY** ☑ Promotion readiness from a recorded window
  (`src/governance/replay.rs`). Five gates per rule: liveness, both-outcomes,
  in-spec, recoverability, and new-blocks (not a gate — the number the operator
  is deciding about). Recoverability walks the trace in **order**: a target
  cleared *before* its refusal proves nothing, and counting it would make every
  rule that ever allowed anything look recoverable.

  Three limits ride in the summary rather than a footnote, because a promotion
  number read without them is read as a safety claim: it measures only traffic
  that happened; it counts false-positive *candidates* and never false positives
  (labelling one needs a human); and it bounds no false negatives at all. So θ is
  **calibratable, not calibrated**. *AC:* a two-sided live rule reports no
  objecting gate; a rule that only ever failed does not.

- **Q-SARC-TREE** ☑ The attribution tree (`src/governance/tree.rs`), reassembled
  from principal chains. The trace is emitted as a **sequence**, so this is
  reconstruction, and the output says so three ways: unattributed records are not
  placed (attaching one to whichever root came first would invent the answer),
  implied dispatch nodes are flagged ("did nothing" ≠ "was not recorded"), and
  collapsed nodes get a note (two dispatches of the same worker by the same
  caller are indistinguishable). *AC:* the summary always states "the trace is a
  sequence, not a tree".

- **Q-SARC-INHERIT** ☑ Constraint laundering under delegation
  (`src/governance/inheritance.rs`), plus `aegis:inheritedByDelegates` and
  `aegis:onUndecidable` — which admits only `"escalate"`, the same shape as
  `onTimeout` admitting only `"deny"`. That is the decidability rescue: evaluate
  at the deepest layer where the constraint still decides, or hand it to a human.

  Two severities again, for the same reason. A constraint that **decided on a
  target** and is absent from a deeper action on the *same* target is a violation
  — it proved it could decide there. One evaluated at a dispatch node and never
  in its subtree is an incompleteness: might be laundering, might be a selector
  that matched nothing deeper, and the record cannot tell. *AC:* `is_below` is a
  strict prefix test, so a sibling branch and a return to a shallower chain are
  both silent.

- **Q-SARC-TRUST** ☑ Trust boundaries on imported state:
  `aegis:importsUntrustedState` / `aegis:untrustedOrigin` on the `ToolClass`,
  because the tool class is the thing that actually imports. Reported whether or
  not the class is governed — `governedAt` says its own *actions* traverse a
  point and says nothing about what it *returned*.

  **Not closed:** nothing evaluates imported content. A trust *predicate* over
  sub-agent responses needs a producer that records them, which no part of this
  stack does; building the predicate without one would ship it dark. The boundary
  is declared and reported on every run, and that is the whole claim.

### hank

- **H-SARC-I6** ☑ Check `aegis:hostedAtLayer` against reality at the projection
  seam. The field is declared and unconsumed: nothing compares it to where a
  constraint is ACTUALLY evaluated, so a policy may claim `"tool"` — the layer an
  agent cannot route around — while being enforced solely by hank's
  orchestration-layer pre-edit hook, which an agent bypasses by writing the file
  another way. I6 is checked for well-formedness and not for truth, and a false
  `tool` claim is worse than an honest `orchestration` one because it stops
  people looking.

  Hank knows what it is: a rule evaluated by `hank hook pre-edit` is hosted at
  the orchestration layer, always — a constant, not a setting. On projection,
  compare declared against actual. The check is **one-directional**: declaring a
  weaker layer than the truth understates robustness and is harmless; declaring a
  stronger one is the failure.

  Response is a loud fail-open, never a block — refusing to project a policy
  because its metadata overclaims would disable a rule that does still work,
  trading a documentation error for an enforcement gap. Record the layer actually
  used on the verdict and trace record so the Q-SARC-AUDIT checker can verify
  claim against record rather than taking the claim. *AC:* a projected policy
  declaring `"tool"` and evaluated by hank produces a mismatch notice naming the
  policy, the claimed layer and the real one, and still evaluates and still
  blocks if hard; `"orchestration"` and an absent declaration both produce
  silence.
- **H-DEP** ☐ Pin and wire the `quipu` git dep (commented out in
  `hank/Cargo.toml`; Phase-4 kickoff). *AC:* `--features quipu` builds against a
  real quipu rev; promote path reaches a live `/knot`.
- **H-PROJECTION** ☐ Hot projection of quipu `boundary:"action"` structural
  policies into hank, one-directional with invalidation. *AC:* a policy added in
  quipu appears in hank's registry on the next refresh; hank never originates a
  policy definition.
- **H-EDIT-EVAL** ☐ Evaluate projected structural policies at `hook pre-edit`
  against the resident graph. *AC:* an edit violating a structural `deny` policy
  is blocked at edit time with a tier-tagged verdict.
- **H-FRESHNESS** ☐ Serve FR-3 freshness so a projected-policy verdict declares
  fresh/stale of the registry it used. *AC:* a verdict computed against a stale
  projection is tagged stale, never silently `fresh`.
- **H-PROMOTE-VERDICT** ☐ Promote hank edit-time verdicts into quipu as signed
  facts (extend the `commit → touched` promote path). *AC:* a hank verdict lands
  as a bitemporal quipu Verdict attributable to the hank verifier identity.

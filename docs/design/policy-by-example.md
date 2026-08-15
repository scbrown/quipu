# Policy by example: from an observed edit to a governed rule

> **Implementation status (2026-08-15):** 🟨 **Quipu side built (quipu-6bl)** —
> sequencing steps 1, 3 and 4. `aegis:exemplar` is in the vocabulary
> (`shapes/governance.ttl` + `shapes/aegis-properties.ttl`); the drafting
> scaffold emits born-advisory (`effect "warn"`, hard-coded), placement-aimed
> Turtle (`src/governance/draft.rs`, `quipu policy draft`) with the placement
> check still running at ingest; the backtest replays a candidate over the
> store's bitemporal history pre-creation, distinguishing "0 hits" from
> "cannot evaluate" (`src/governance/backtest.rs`, `quipu policy backtest`);
> the rejection seam offers the request IRI as exemplar
> (`src/governance/router.rs`) and guard refusals cite their motivating case
> (`src/governance/guard.rs`). **Step 2 (yupana exemplar extraction: spool ref
> → Selector + tiered predicates) and step 5's end-to-end skill gesture remain
> design only.**

## Problem

A human sees an edit — in review, in the verdict spool, in a PR — and
wants it blocked in the future. Today that intent must be hand-compiled
into Turtle: a Selector, a Predicate, a constraint class, a verification
point, an effect, an OperatingPoint. The distance between "never do this
again" and a valid `aegis:Policy` is where the intent dies. The policy
surface grows only as fast as someone is willing to author governance
atoms by hand.

## The flow

One gesture, four mechanical steps, no hand-authored Turtle:

1. **Point at the exemplar.** The human names a concrete instance — a
   verdict-spool entry, a commit + path + hunk, an escalated request
   they just rejected. `quipu policy from-edit <ref>` (CLI), or a
   one-click "make this a standing policy" offered exactly where a
   human already rules on instances: the escalation router's rejection
   seam. A rejection *is* the "not this" signal; today it dies with the
   single (policy, target) pair it was bound to.

2. **Draft the candidate.** From the exemplar, the tooling extracts:
   - the **Selector** — yupana already knows the node context the
     offending text lived in (line_comment, string literal, identifier,
     import, file class);
   - a **Predicate at each viable tier**: exact (the specific token or
     id, membership-checked), lexical (a generated narrowing pattern,
     offered for human approval, never self-asserted as authority),
     similarity ("edits like this one" — the exemplar's embedding as
     the anchor, threshold suggested from the backtest below);
   - the **required metadata scaffolded**, not hand-typed: constraint
     class, verification point, effect, OperatingPoint tolerances,
     hosting layer — pre-filled to satisfy the definition-time
     placement check, which still runs and still refuses a malformed
     result. The human edits a filled-in form, not an empty vocabulary.

3. **Backtest before birth.** The candidate is immediately replayed
   against recorded history (the traces and spool the store already
   holds): *"this rule would have fired N times in the last 90 days —
   here are the hits."* The human sees the false-positive surface
   before the rule exists. Threshold and tier suggestions come from
   this backtest, not from intuition.

4. **Born advisory, promoted by evidence.** The accepted candidate
   lands as `effect "warn"` — never enforcing on day one. The existing
   advisory→enforcing promotion gates apply unchanged: liveness,
   two-sidedness, new-blocks, recoverability, blast radius, measured
   over real traffic. "Easy to express" must not mean "easy to deploy a
   bad hard rule"; the ease is front-loaded into drafting and
   backtesting, while promotion keeps its evidence bar.

## Provenance and explanation

- The policy records its motivating case: `aegis:exemplar` links the
  drafted policy to the verdict/edit record that birthed it. A later
  refusal under the policy can cite it — "blocked: similar to the edit
  that motivated this rule" — so refusals arrive explained by example,
  completing the identify-and-inform-before-refusing ordering.
- The human's intent sentence is kept verbatim as the policy's label
  and claim rationale; the policy is `declared` provenance with the
  human as authority. The drafting tool's suggestions are scaffolding;
  what the human accepts is what the human authored.
- An exemplar-anchored similarity predicate follows the similarity
  disciplines unchanged (`docs/design/semantic-grounded-edit-policies.md`):
  embedding tier, nonzero tolerances, advisory/escalate placement,
  score and corpus watermark in every verdict.

## What this reuses

| Need | Existing mechanism |
|---|---|
| The exemplar record | verdict spool, enforcement traces (yupana) |
| The rejection seam | escalation router decision records (provisional A §3) |
| Refusing malformed drafts | definition-time placement validation (provisional A §7.2) |
| Backtest | replay: "what would this have stopped" (provisional A §5.3) |
| Advisory→enforcing bar | promotion gates over recorded traces (§5.3) |
| "Like this" predicates | embedding tier (semantic-grounded-edit-policies.md) |
| Hot enforcement of the result | yupana projection + pre-edit seam |

## Sequencing

1. `aegis:exemplar` linkage in the policy vocabulary; drafting scaffold
   that emits placement-valid Turtle from a filled template. (quipu)
2. Exemplar extraction: verdict-spool ref → Selector + tiered predicate
   candidates. (yupana)
3. Backtest command: candidate policy × recorded window → hit list with
   FP surface. (quipu replay)
4. Reject-to-policy offer at the escalation seam. (quipu router)
5. CLI/skill gesture end-to-end; refusals cite their exemplar. (both)

# Census BUILD_REPORT

The honest record of this benchmark: what went into it, how synthetic
items were constructed, what was discarded, and what its results do not
claim. Update this file in the same change as any harness behavior
change — an out-of-date honesty record is worse than none.

## Inputs and provenance

- No external data. The scenario is fully synthetic and scripted; the
  defect catalogue is `docs/design/paper-principles.md` §4, transcribed
  into `examples/census/catalogue.rs`.
- The only entropy input is `--seed` (SplitMix64, self-contained).
  Timestamps are logical (a minute counter from a fixed epoch); no wall
  clock reaches the manifest.

## Construction of synthetic items

- Cast and places are fixed constants (`phases.rs`), not sampled — probe
  ids stay stable across seeds; the rng varies volumes and orderings
  only.
- Defect probes are constructed to be the *plausible* mistake an agent
  writer makes (untagged fact, out-of-authority write, post-state-only
  violation), not random noise. Each entry's `plants` field says
  exactly what was planted.

## Discarded runs and designs

- **Plain-BGP claims (discarded during quipu-y41).** Σ's claims were
  first written without `GRAPH ?g`; every graph-scoped write was then
  judged against the default-graph view, which is empty for district
  facts — the tally-label claim denied a compliant write. All claims
  now wrap their patterns in `GRAPH ?g`. Kept as a finding: a claim's
  dataset scope is part of the claim, and the paper's governance
  section gets a sentence on it.
- **`urn:census:record_u1` as CEN-U1's subject (discarded).** The
  episode path mints `urn:census:record-u1` (hyphen preserved); the
  first control run scored 5/6 defects present because the scorer
  asked about a subject that never existed. Corrected against the
  store, not the script.

## Findings the numbers carry

- **Replay is asymmetric, and honestly so.** A *satisfied* verdict
  re-derives fully as-of its instant — the facts persisted and both the
  data and the rules are bitemporal. A *denied* verdict cannot be
  re-derived from the store alone: the staged delta was rolled back by
  design (GS2 keeps the verdict, deliberately not the attempt), so
  replay for denials verifies the rules-in-force instead. Full denial
  re-derivation would require the trace to carry the attempted delta —
  which hank-style traces do, and the store deliberately does not.
- **A latest-only replay would misreport every pre-amendment decision.**
  All 50 phase-2 satisfied tally verdicts re-derive faithfully under
  the claim in force at their instant, and all 50 evaluate *unsatisfied*
  under the amended claim — the number that separates "the runtime got
  it wrong" from "the spec moved."
- **The audit's floor interaction.** Phase 4's freshness floor, left
  set, refuses the audit's own evidence queries; phase 6 clears it. An
  enforcement floor and an auditor are different readers with different
  rights — worth a sentence in the paper.

- **Abstention is policy-gate-scoped.** The gated arm's ungoverned
  writes are not free: authority intersection (GS3) runs on every
  graph-scoped write by design and is not abstention-eligible. RQ1's
  zero-overhead claim applies to the policy gate's target-type
  pre-filter only; `metrics/<arm>/rq1.json` says so inline.

## What the results do not claim

- Phases 5–6 entries in the manifest are `planned`; `rq5.json` is
  `pending` and `rq3.json` is `partial` (verdict counts only) until
  quipu-krv / quipu-tj0 / quipu-4mi land.
- Census is synthetic by construction — that is what makes it an
  oracle. External validity is bounded, not eliminated, by the
  Census-in-the-wild replay (bead `quipu-0u4`).
- Single-run latency numbers are not results; RQ1 reports
  distributions over repeats, per the determinism note (bead
  `quipu-02v`).

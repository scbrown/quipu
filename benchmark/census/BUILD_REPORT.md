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

_Record every discarded run and the reason here, with the seed and the
git SHA. Nothing so far — the skeleton has produced no scored runs._

## What the results do not claim

- The skeleton executes phase 1 only; every phase 2–6 entry in the
  manifest is `planned`, and no RQ metric is produced yet (each
  `metrics/rq*.json` says `pending` and names the bead that fills it).
- Census is synthetic by construction — that is what makes it an
  oracle. External validity is bounded, not eliminated, by the
  Census-in-the-wild replay (bead `quipu-0u4`).
- Single-run latency numbers are not results; RQ1 reports
  distributions over repeats, per the determinism note (bead
  `quipu-02v`).

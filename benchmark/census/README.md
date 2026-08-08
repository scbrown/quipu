# Census — the paper's lifecycle benchmark

One scripted, seeded, multi-writer lifecycle over a governed store; a
single run emits a ground-truth manifest and one metrics file per
research question. The scenario, the defect catalogue, and the design
rationale live in `docs/design/paper-principles.md` §4; the paper plan
is `docs/design/paper.md`.

## Run

```bash
just bench census                      # gated arm, seed 42
just bench census --arm control        # same script, gate off
just bench census --seed 7 --out /tmp/census
```

## Layout

- `examples/census/` — the harness (deterministic drivers; no LLM):
  `main.rs` (arms, seed, output), `phases.rs` (the six phases),
  `catalogue.rs` (the defect catalogue, transcribed from the design
  doc), `manifest.rs` (ground-truth manifest + metric stubs),
  `rng.rs` (SplitMix64; the seed is the only entropy).
- `out/` (gitignored) — `census-<arm>.db`, `manifest.json`,
  `metrics/rq{1..5}.json`.
- `BUILD_REPORT.md` — the honesty record: inputs, construction,
  discarded designs, claim boundaries.

## Status

Skeleton (bead `quipu-zg0`): phase 1 (Founding) executes against a live
store; phases 2–6 are registered in the manifest as `planned`.
Implementation order and owners: `bd list -l paper`.

## Scoring discipline

Every scorer reads `manifest.json` — the injector's declared ground
truth — never the phase scripts. The manifest is the run's contract;
if a scorer needs a fact the manifest lacks, the manifest grows, the
scorer does not peek.

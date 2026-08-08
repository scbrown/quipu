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
- `out/` (gitignored) — `census-<arm>.db`, `manifest-<arm>.json`,
  `metrics/<arm>/rq{1..5}.json`.
- `BUILD_REPORT.md` — the honesty record: inputs, construction,
  discarded designs, claim boundaries.

## Status

All six phases execute (beads `quipu-zg0`, `quipu-y41`, `quipu-krv`,
`quipu-tj0`, `quipu-4mi`): founding, recording with all six defect
probes, correction with the escalation round-trips, the seven
composition probes, the mid-run amendment, and the audit — as-of
replay, dispatch inventory, trace audit, and the external-checker
export. Remaining paper work: `bd list -l paper`.

## External checker (CEN-X1)

The gated run exports `out/sarc-export/{spec.yaml, trace-faithful.json,
trace-padded.json}`. Score against the SARC reference checker:

```bash
git clone --depth 1 https://github.com/besanson/sarc-governance /tmp/sarc
python3 benchmark/census/sarc_check.py \
    --export benchmark/census/out/sarc-export \
    --sarc-src /tmp/sarc/src
```

## DEMM export (CEN-X2)

The gated run also exports `out/demm-export/native_records.jsonl` — the
run's 56 decisions as quipu-native evidence records (guard trace, signed
verdict ledger, bitemporal policy snapshot) for the DEMM-Bench
decision-evidence sufficiency benchmark. Degradation, adaptation, and
scoring live in `benchmark/demm/`.

## Scoring discipline

Every scorer reads `manifest.json` — the injector's declared ground
truth — never the phase scripts. The manifest is the run's contract;
if a scorer needs a fact the manifest lacks, the manifest grows, the
scorer does not peek.

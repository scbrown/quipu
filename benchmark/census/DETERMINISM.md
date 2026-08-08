# Census determinism note

Every reported number's reproducibility status, with the measured
hashes. Re-measure after any harness change; a number whose hash story
is unknown is not a result. General rule (inherited from the store):
sort every hash-derived traversal, and keep wall clocks out of anything
a hash covers.

## Measured (2026-08-08, native x86-64, `--release`)

**Manifests are byte-identical per (seed, arm).** Three consecutive
seed-42 gated runs into three separate directories:

```text
e5f5f135394e189d9abc559a08a352c698e78ffe9041af9004afe3f5576603ee  ×3
```

Two seed-42 control runs:

```text
248611b2e62b1f7fff17691129005d2b9f5a957d16b27ba58444557fc1297d86  ×2
```

A seed-7 gated run hashes differently, as it must — the rng varies
clean-write district assignment, and probe ids/expectations stay fixed
by design.

**The sarc-export is byte-identical across runs** (three files, each
hash appearing once per run):

```text
2b1d74bc29d5e5be1d5d01ac454c7fa23e5208825b07899694fd5b1fc23ef353
5547f68d1b0983d2d2d7e6e4badbfb1b3219423d2889cdc12c1d91bba522035b
8760086004ace3afa8e15ccfe488f675dd3286888b13c0f59191bb3bce4588d0
```

**RQ2 / RQ4 / RQ5 metric bodies are value-identical across runs.**
RQ1 (`rq1.json`) varies by design — it is a latency distribution;
single-run numbers are not results (`BUILD_REPORT.md`), and any
reported RQ1 figure must aggregate repeats.

## One divergence found and fixed while writing this note

The first three-run measurement produced three DIFFERENT gated-manifest
hashes. The diff was one field: CEN-X1's `observed` string embedded the
absolute output directory, so runs into different directories differed
by path alone. The manifest now names `sarc-export/` relatively. Kept
here per the withdraw-in-place discipline: the divergence was real, the
cause was found, and the fixed hashes above are post-fix.

## What is deliberately NOT byte-stable

- **The store files (`census-<arm>.db`).** Two wall-clock leaks are
  accepted: the graph registry's `created_at` (`Store::graph_create`
  uses real time — registry metadata, never queried by any scorer) and
  the ed25519 signing key, generated per output directory, which makes
  verdict IRIs (signature-derived) differ across environments. Content
  identity across id assignment and signing environments is the pack
  content hash (sorted N-Triples, `src/pack.rs`), not file bytes.
- **Timestamps in `sarc-export` traces** are logical indices, not wall
  time — that is why the export IS byte-stable.

## How to re-measure

```bash
for i in 1 2 3; do
  cargo run --release --example census -- --out /tmp/det$i
done
sha256sum /tmp/det*/manifest-gated.json   # expect one hash, three times
```

# DEMM — decision-evidence sufficiency against an external benchmark

Runs [DEMM-Bench](https://arxiv.org/abs/2606.20634)
(`agent-runtime-evidence/decision-evidence-benchmark`) against quipu as
a ninth evidence regime: does the evidence quipu emits per governed
decision suffice to reconstruct the benchmark's eight Decision Event
Schema properties — actor identity, principal authority, action
boundary, policy basis, decision basis, data/resource touch, lifecycle
context, verification strength — and does a reader of that evidence
avoid *overclaiming* under the benchmark's eight degradation conditions?

## Run

```bash
just bench census                  # emits demm-export/ (probe CEN-X2)
git clone --depth 1 \
    https://github.com/agent-runtime-evidence/decision-evidence-benchmark /tmp/demm
python3 -m venv /tmp/demm/.venv && /tmp/demm/.venv/bin/pip install -e /tmp/demm
/tmp/demm/.venv/bin/python benchmark/demm/run.py --demm-src /tmp/demm
```

## Layout

- `examples/census/demm.rs` (probe CEN-X2) — exports the census run's 56
  recorded decisions as quipu-native records, three evidence planes each:
  the writer-side `guard_trace` (writer, principal chain, tool, target
  graph — the store deliberately does not persist these for denials),
  the signed `verdict_ledger` fact queried back from the store, and the
  bitemporal `policy_snapshot` (claim as-of vs current, authority
  grants).
- `degrade.py` — the benchmark's eight degradation conditions as
  content-level deletions (plus one contradiction) over those planes,
  mirroring its construction-oracle semantics.
- `adapter.py` — the quipu regime adapter: reconstructs the eight
  property categories from record content only, and derives the shallow
  container-presence flags the benchmark's baselines consume.
- `run.py` — builds the 64-case corpus (8 conditions x 8 question
  families, each case a real census decision), labels it with the
  benchmark's own construction oracle, and scores the adapter and the
  benchmark's five deterministic container-presence baselines.
- `out/` (gitignored) — cases, scorer outputs, per-case results,
  summaries, `quipu_headline.json`.

## Result (seed-42 census export, deterministic)

| scorer | overclaim rate | mean PSA |
|---|---|---|
| quipu property-level reconstructor | **0.00** (0/64) | **1.00** |
| trace-present baseline | 0.875 | — |
| ledger-present baseline | 0.875 | — |
| schema-present baseline | 0.875 | — |
| container-checklist baseline | 0.625 | — |
| source-specific validator baseline | 0.625 | — |

The benchmark's published reference on its own 64-case corpus:
container-presence baselines overclaim on 50–75% of cases; its
redacted property-level scorer reaches 56.25% mean PSA at zero
overclaim. Repeat runs here are byte-identical.

## Scoring discipline and claim boundaries

- Ground truth is the benchmark's construction oracle
  (`construction_oracle_v1`), not ours; the degradations are built to
  its category vectors, and the adapter is scored against them.
- The adapter reads evidence **content only** — case ids are opaque
  (`quipu-demm-NNN`), no degradation names reach any scorer-facing
  field, and predictions are computed from the degraded record before
  the case's labels exist in scope (the benchmark's label-leakage
  rules).
- PSA 1.0 is a claim about *format decidability*, not reconstruction
  difficulty: quipu's three evidence planes carry explicit, separable
  markers for each property, so content-level rules recover exactly
  what each degradation left. The benchmark's own headline scorer is
  restricted to redacted container indicators and tops out at 56.25%
  PSA on evidence that collapses those distinctions — the comparison
  measures what a record format preserves, not scorer cleverness.
- The zero-overclaim column is the load-bearing one: signed verdicts,
  named policies with as-of claims, and explicit authority grants mean
  absence is detectable as absence, so a content-level reader never has
  to guess sufficiency from container presence.
- Evidence is synthetic-lifecycle (the seeded census run), one store,
  one Σ; this establishes decidability of quipu's record format under
  the benchmark's degradation semantics, not field prevalence.

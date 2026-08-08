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

| scorer | sufficient | overclaim | underclaim | mean PSA |
|---|---|---|---|---|
| quipu property-level reconstructor | 8/64 | **0.000** | 0.000 | **1.00** |
| source-specific validator (quipu-internal validity) | 8/64 | 0.000 | 0.000 | — |
| container-checklist (all three planes present) | 56/64 | 0.750 | 0.000 | — |
| trace-present / ledger-present / schema-present | 64/64 | 0.875 | 0.000 | — |

DEMM-Bench's published reference on its own 64-case corpus (its Tables
4–5): trace/schema-present 0.75, ledger-present 0.50, its
container-checklist and source-specific validators 0.00 — and its
redacted-input candidate scorer 56.25% mean PSA at zero overclaim.
Repeat runs here are byte-identical.

Two contrasts carry the finding. **Presence predicates do worse on
quipu than on the benchmark's own corpus** (0.875 vs 0.50–0.75):
quipu always emits all three planes, so content-level degradation
leaves every container present and presence carries no information —
the container fallacy at its ceiling. **Validity and property-level
reading both abstain correctly, but only the latter localizes**: the
quipu-internal validator (field completeness, evidence-hash
recomputation, executor–chain consistency, grant scope) reaches zero
overclaim by refusing every degraded record outright, while the
property-level reader additionally names which of the eight properties
each degradation destroyed — including the two slices the benchmark's
candidate found hardest (conflicting-identity, PSA 0.25 there; action
boundary, PSA 0.25 overall), both decidable from quipu's guard trace
because the record names its tool, target, graph, and chain
explicitly.

## Scoring discipline and claim boundaries

- Ground truth is the benchmark's construction oracle
  (`construction_oracle_v1`), not ours; the degradations are built to
  its category vectors, and the adapter is scored against them.
- The adapter reads evidence **content only** — case ids are opaque
  (`quipu-demm-NNN`), no degradation names reach any scorer-facing
  field, and predictions are computed from the degraded record before
  the case's labels exist in scope (the benchmark's label-leakage
  rules).
- PSA 1.0 is a claim about *format decidability*, not scorer
  superiority: quipu's three evidence planes carry explicit, separable
  markers for each property, so content-level rules recover exactly
  what each degradation left. The benchmark's own candidate scorer is
  deliberately restricted to redacted container indicators (its
  conservative no-human configuration) and is not comparable
  head-to-head; the degradations and the adapter here are also ours,
  built to the oracle's published semantics — this is a self-run
  ninth-regime extension in the direction the benchmark's future-work
  F2 invites, not a leaderboard entry.
- The zero-overclaim column is the load-bearing one: signed verdicts,
  named policies with as-of claims, and explicit authority grants mean
  absence is detectable as absence, so a content-level reader never has
  to guess sufficiency from container presence.
- Evidence is synthetic-lifecycle (the seeded census run), one store,
  one Σ; this establishes decidability of quipu's record format under
  the benchmark's degradation semantics, not field prevalence.

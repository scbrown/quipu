# Arm D preparation and offline replay

The [preregistered protocol](PREREGISTRATION.md) governs this experiment.
These tools do not call Jev or modify a graph. Stored pilot and held-out results
are included for offline replay.

Run the isolated controls with:

```sh
just -f benchmark/alias-resolution/justfile test
```

## Private preparation

`prepare.py` accepts a checkpointed, private SQLite copy and a private mapping
to the published arm-B corpus. It verifies the registered cohort order, classes
and endpoint bijection, selects ROOT explicitly, and reads assertions before
the earliest recorded transitive identity connection. It applies both valid
and transaction time, conservatively withholding legacy closed rows without a
retraction transaction and timestamps without an unambiguous timezone.
Unknown history is counted; current facts never fill a historical gap.

The original snapshot and its checksum/provenance must be retained separately.
Preparation creates its output exclusively with mode 0600; it refuses to replace
an existing artifact. Neither this output nor its source mapping is public.

```sh
just -f benchmark/alias-resolution/justfile prepare \
  /private/source.db /private/cohort.json /private/prepared.json
just -f benchmark/alias-resolution/justfile candidates \
  /private/source.db /private/prepared.json \
  /private/model.onnx /private/tokenizer.json /private/candidates.json
```

`candidates.py` pins its Python dependencies and records model/tokenizer hashes.
It uses masked mean pooling, L2 normalization and a 256-token limit, matching
the source encoder implementation. Its free scores use the same cleaned
historical labels, descriptions and types. Candidate ranking uses the left
endpoint of each recorded pair as the anchor, same asserted type, and the top
20 by maximum label/cosine similarity. Ties use the private entity identifier.
Recorded positive components are excluded. The output keeps ranks and evidence
for independent negative adjudication; absence of an identity edge is not proof
of distinctness. Proposed distinguishing evidence includes an earlier explicit
distinctness assertion or incompatible commit identifiers in asserted labels.
These are review inputs, not automatically accepted gold labels.

The content filter removes recognized identity and source annotations and
flags answer-bearing prose for review. It is deliberately conservative and
is not a proof that arbitrary prose contains no answer. Every final item needs
content review before freezing. A preparation `eligible` flag means mechanical
prerequisites passed; it does not authorize a request. The final request size,
input/label/prompt manifests, reviewer check and spending ledger remain separate
mandatory gates. No paid-call runner is included here.

## Replay

`replay.py` computes curves from an item manifest and stored raw numeric
responses. Item entries require `id`, `gold` (`same` or `different`), `class`,
`pilot`, `eligible`, and `baselines` containing `label`, `cosine`, and optionally
`id-control`. The response file maps item IDs to the provider's raw response
objects (`model`, `answers.q.choice`, `probabilities`, `confidence`). A release
must preserve request binding through a reviewed manifest; the scorer does not
certify that a response came from the stated request.

```sh
just -f benchmark/alias-resolution/justfile replay items.json responses.json
```

All positives, including unavailable evidence and abstentions, remain in recall.
Precision with no identity proposals is null. A missing embedding component
makes the union unavailable. Negative abstentions do not count as true negatives.
The output reports false identity proposals next to recovered aliases and zero
actual merges. Wilson intervals are descriptive binomial intervals; shared
endpoints and selected historical pairs limit any population interpretation.
Held-out-only results exclude the pilot. No threshold is fitted to the cohort.

Raw preparation output still contains private names. Publication requires a
separate scrubbed view and the artifact scrub gate; this directory's tools
neither anonymize that output nor make its publication safe.

The completed pilot's numeric responses and all ten item statuses are recorded
in `pilot-responses.json` and `pilot-attempts.json`; `pilot-metrics.json` is the
offline replay restricted to pilot items. Eight responses, two unavailable
items, ten attempts: all four items under the bounded-retry amendment succeeded
on their first attempt. Earlier timeouts were not retried. Availability is 8/10
both per first attempt and per item; no actual merges were performed.

The known usage-derived list-price subtotal is $0.000533358, plus unknown possible
charges for two failed attempts. Total account spend is unknown. Request bytes,
latency, and usage are reported per call. The first failure latency uses attempt
timestamp to error-receipt mtime; subsequent latencies use a monotonic timer.
Pilot comparisons are small and selected; semantic negatives are easy. The separately authorized full run is now complete.

## Completed held-out run

`responses.json` contains the 212 valid pilot and held-out responses;
`held-out-attempts.json` records every one of the 216 held-out attempts, including
12 timeouts. `held-out-completion.json` and `first50-gate.json` record the run's
stopping/accounting state. Pilot attempts remain in `pilot-attempts.json`.
First-attempt availability was 192/204 and eventual availability 204/204; pilot
availability was 8/10 under its separately recorded policies.

At the registered threshold, held-out Jev recovered 11/88 original positives
(11/55 eligible) with one false identity proposal. ID-form recovery was 10/43
(10/36 eligible), with no false proposals. Semantic recovery was 1/45 (1/19
eligible), with one false proposal; label matching recovered 3/45 (3/19 eligible)
with none. Across both cohorts Jev recovered 14/93, not the entire alias gap.
The known usage-derived subtotal including the pilot is $0.014897148, plus
unknown possible charges for 14 timeouts. Actual merges remain zero.

See `REPLAY.md` for reproducing both recall denominators and cohort separation,
`metrics.json` and `curves.csv` for threshold curves, `comparison-summary.json`
for the registered operating points, and `PROVENANCE.md` for input/privacy limits.

The preregistered requirement that Jev beat the free baselines fails on the
semantic stratum. This is a negative finding for Jev, not a recommendation to
add a paid entity-resolution step.

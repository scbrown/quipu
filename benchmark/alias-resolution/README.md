# Arm D preparation and offline replay

The [preregistered protocol](PREREGISTRATION.md) governs this experiment.
These tools do not call Jev or modify a graph. No model result is claimed yet.

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

The interrupted pilot's numeric responses and all ten item statuses are recorded
in `pilot-responses.json` and `pilot-attempts.json`; four items are explicitly
not yet attempted. The subsequent bounded-retry amendment in the registration
applies only to those four untouched items. Historical observations retain their
original policy and are not silently replaced by retries.

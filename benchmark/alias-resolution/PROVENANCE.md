# Arm D provenance and replay scope

The protocol was committed before paid inference (`300b79d8`); preparation,
free scores and the proposed item manifest followed in `582dcdde`. The original
ten-item pilot inputs and labels were separately frozen before transmission.
Timeout-only registration `c870f1e4` preceded its continuation. Retry registration
`26532f46` preceded the final four pilot items. Full-run registration `6245f2f3`
preceded all held-out calls. The later publication view is not a replacement for
any frozen request.

The preserved source snapshot predates the experiment. Evidence selection uses
ROOT assertions before each pair's earliest recorded transitive identity
connection, with transaction and valid-time restrictions. Current descriptions
or vectors do not fill historical gaps. Historical text was encoded locally
with the pinned MiniLM model and tokenizer, not fetched as current vectors.

Model file SHA-256:
`759c3cd2b7fe7e93933ad23c4c9181b7396442a2ed746ec7c1d46192c469c46e`

Vocabulary asset SHA-256:
`da0e79933b9ed51798a3ae27893d3c5fa4a201126cef75586296df9b4d2c62a0`

The model and tokenizer are not redistributed. The encoder configuration and
package versions are in `baselines.py` and `candidates.py`.

The private freeze retains source correspondence, exact requests, canonical
request hashes, approval records, client source, and write-before-send attempt
receipts. Public files omit source correspondence and private request hashes.
The public scorer verifies response structure and the requested model name; it
does not independently authenticate the provider or prove request binding.

`proposed-items.json` is the frozen numeric item set: all 93 original positives
and 155 negatives. Eligibility retains the unavailable-evidence and oversized
items explicitly. `item-views.json` is a later, structurally scrubbed view of
that set, with private text withheld. `EXAMPLES.md` contains editorially scrubbed
illustrations. Neither is claimed to reproduce the original inference inputs.

The public numeric responses and the frozen baseline scores permit offline
recomputation of decision metrics and threshold curves with `replay.py`, without
API calls. Recomputing lexical/embedding scores or repeating model inference on
the original inputs requires access to the preserved private evidence. This
boundary is explicit so scoring reproducibility is not mistaken for full input
reproducibility.

Usage-derived cost uses the [published Jev price](https://docs.typesafe.ai/models)
checked for the run: $0.042 per million input tokens, with output tokens free.
This is list-price arithmetic, not a provider billing receipt.

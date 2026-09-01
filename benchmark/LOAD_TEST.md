# Mixed load performance ratchet

`scripts/quipu-load-test.py` drives a running Quipu server through a deterministic
mix of bounded SPARQL, full-graph SPARQL, inferred `rdf:type` SPARQL,
precomputed-vector search, and episode writes. The fixture loads a subclass
ontology and writes instances of that subclass, so both the query-time RDFS
expansion and default reactive OWL write path are exercised. It steps through
concurrency 1, 4, and 8 and records p50/p95/p99 latency,
throughput, HTTP error classes, and server peak RSS.

The CI job starts an empty ephemeral server and publishes both the JSON report and
server log. No embedding model or external service is required: `/search` receives
a fixed 384-element vector.

Run it locally after starting `quipu-server`:

```sh
scripts/quipu-load-test.py \
  --url http://127.0.0.1:3030 \
  --baseline benchmark/load-test-baseline.json \
  --output load-test-report.json
```

The checked-in limits are deliberately runner-tolerant absolute guardrails. Tighten
them only from repeated CI observations; do not loosen them to clear one slow run.
The `baseline` object preserves the fixed-corpus result from `aegis-bzofv` as the
first historical point, while `limits` controls the CI verdict.

`read_progress_during_writes` counts successful read requests whose execution
overlapped an episode request. A zero fails the ratchet: it means the run did not
demonstrate that WAL readers progress while Quipu's single fair writer is occupied.
HTTP 408 is reported separately from 502, client timeout, transport failure, and
other status codes. A 408 means the query exhausted its own budget; it is not by
itself evidence that the writer lock or server is wedged.

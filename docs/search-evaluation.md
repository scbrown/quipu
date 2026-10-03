# Judged search evaluation

Run `just search-eval test` for the offline CI subset. Five synthetic query classes
exercise the runner against captured responses with hand-calculated grades. These
are runner regression tests, **not** embedding quality or server performance tests.
No network, model download, or operational corpus is needed.

For retrieval measurements, restore a consistent database backup and its archived
packs into an isolated directory. Adjust restored pack paths, record that change,
and use the same model, tokenizer, embedding dimension, sequence length and search
configuration as the serving build. Bind the server to a literal loopback address.
The runner refuses remote origins and redirects. Do not forward its port to a live
service. Disable automatic writes/backfill and freeze the corpus for the run.

```sh
just search-eval run --suite /path/to/golden.json \
  --endpoint http://127.0.0.1:3031 \
  --corpus-sha256 <restored-database-sha256> \
  --snapshot-at <UTC-backup-time> --expected-sha <full-clean-server-sha> \
  --output /path/to/baseline.json
```

The suite schema is illustrated in `tests/fixtures/search-eval/suite.json`.
Each query carries class, provenance, judgment rationale, and entity relevance
(grades 0–3). Optional `prefixes` expand response entity prefixes; optional query
`params` carry search settings without overriding query text or limit. Keep real
traffic, operational entity identifiers, judgments and raw responses in the
appropriate private evidence repository, never in a public fixture.

Freeze judgments **before** examining the baseline ranking. Record the assessor,
sampling method, source timestamp/session, relevant evidence and limitations.
A positive-only seed set is incomplete: its recall measures labeled positives,
not every relevant entity in the corpus. Unjudged hits score zero. Inspect both
positive misses and unjudged high-ranked candidates before treating a later
score improvement as general retrieval quality; version any revised judgments
and recompute every compared baseline. Do not tune on a claimed held-out set.

Reports retain ordered entities and scores, effective requests, suite/corpus
hashes, clean server identity, UTC time, per-query results, and class aggregates.
Metrics use exponential gain nDCG@10, positive-label recall@20, and reciprocal
rank truncated at 20 (MRR@20). Class/overall quality scores are macro averages.
The runner performs one unmeasured warmup per query and three timed repetitions
by default. Latency percentiles use nearest rank over all measured repetitions,
and include HTTP plus JSON parsing. They describe the isolated host, not live
production latency. HTTP errors, malformed responses, duplicate entities,
non-finite scores, changing versions and unstable ordered scores fail the run.

Use `--compare reference.json` to require identical requests, corpus, judgments,
and ordered entity/score pairs for an unchanged semantic regression arm. This
strict gate deliberately refuses a changed request: comparisons that introduce
new parameters such as `alpha=1` need a separately documented request equivalence
check before claiming semantic compatibility. Keep diagnostic comparisons across
corpus refreshes separate from regression gates.

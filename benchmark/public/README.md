# Public benchmark evaluation

This directory runs version-pinned, public benchmark material against an
ephemeral Quipu store. It does not use a production graph.

## SPARQL 1.1 syntax

The first executable slice scores the W3C RDF Tests SPARQL 1.1 query-syntax
manifest. It includes only Working Group-approved positive and negative syntax
tests. A positive test passes when Quipu's parser accepts it; a negative test
passes when Quipu emits its explicit `SPARQL parse error` diagnostic. Runtime
evaluation errors are therefore not misreported as grammar failures.

```sh
git clone https://github.com/w3c/rdf-tests /tmp/rdf-tests
git -C /tmp/rdf-tests checkout 369a90d1a60c021b746df2e411da0ff36258a758
python3 benchmark/public/sparql11_syntax.py \
  --suite /tmp/rdf-tests/sparql/sparql11/syntax-query \
  --quipu "$(cargo metadata --no-deps --format-version 1 | \
    python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/quipu" \
  --output benchmark/public/results/sparql11-syntax.json
```

The output records the suite revision, Quipu revision/version, per-test result,
and totals. The script refuses a dirty or differently pinned suite unless
`--allow-unpinned-suite` is explicit.

## Phase-one benchmark choices

| Layer | Public artifact | Decision |
|---|---|---|
| conformance | W3C RDF Tests, SPARQL 1.1 | Selected; syntax runner implemented here. Query-evaluation manifests are the next slice. |
| performance | WatDiv 0.6 | Selected over LUBM because its query-shape diversity directly tests pathological join shapes; its publisher supplies 10M, 100M, and 1B datasets. |
| extraction | Text2KGBench | Feasible in principle, but not store-only: it requires a frozen model/prompt configuration and ontology-guided triple scorer. Keep it in the Caboodle end-to-end phase. |

## SPARQL 1.1 evaluation classes

The evaluation runner discovers the approved tests from the pinned W3C aggregate
manifests and publishes a separate ledger for query evaluation, protocol,
update, entailment, and result formats. Every executable case gets a fresh
temporary SQLite database; no production or developer store is reused.

```sh
cargo build --release --bin quipu
QUIPU_BIN="$(cargo metadata --no-deps --format-version 1 | \
  python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/quipu"
python3 benchmark/public/sparql11_evaluation.py \
  --suite /tmp/rdf-tests/sparql/sparql11 \
  --quipu "$QUIPU_BIN" \
  --output benchmark/public/results/sparql11-evaluation.json
```

Add `--class query-evaluation`, `protocol`, `update`, `entailment`, or
`result-format` to reproduce one independently scored class. A nonzero runner
exit means at least one executable test failed or the harness could not evaluate
it; the JSON ledger is still written in full.

At Quipu `436b3f9` and W3C RDF Tests `369a90d1`, the independently reported
results are:

| Class | Passed | Failed/error | Unsupported | Total |
|---|---:|---:|---:|---:|
| query evaluation | 18 | 131 | 19 | 168 |
| protocol | 0 | 0 | 34 | 34 |
| update | 0 | 0 | 37 | 37 |
| entailment | 0 | 0 | 70 | 70 |
| result format | 2 | 1 | 4 | 7 |

Unsupported rows are explicit per test. Current class-level gaps are the HTTP
request-sequence executor (protocol), a SPARQL Update surface, entailment-regime
setup, named-graph fixture loading, RDF graph-result comparison, and blank-node
isomorphism. These results are never combined into a single compliance score.

WatDiv ingest is currently blocked on a general RDF bulk-loader. Quipu's `/import`
accepts Quipu share bundles, not arbitrary benchmark N-Triples, while `knot` is a
governed transactional assertion path rather than a billion-triple loader. Do not
quote a large-scale performance number until the identical WatDiv dataset can be
loaded into both Quipu and the comparator with measured counts.

## Claim boundary

This is a parser-conformance score, not a claim of full SPARQL 1.1 compliance.
Protocol, query evaluation, update, entailment, and result-format manifests are
reported as separate classes and must remain separate.

## Baseline result

At Quipu `857bc77f` plus the default-base parser fix and W3C RDF Tests
`369a90d1`, **86/86 approved query-syntax cases pass**. The final two cases both
depended on relative IRI resolution: `IN(1,<x>)` (`syntax-oneof-03.rq`) and a
relative graph IRI in a `CONSTRUCT FROM` query
(`syntax-construct-where-02.rq`). The JSON report carries the complete 86-case
ledger. This number says nothing about query-result correctness; evaluation
manifests are deliberately excluded.

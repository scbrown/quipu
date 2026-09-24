# Competitor conformance: the same W3C harness, other stores

Quipu's [SPARQL 1.1 conformance](../../docs/book/src/benchmarks/conformance.md)
numbers come from a checked-in runner at a pinned
[W3C RDF Tests](https://github.com/w3c/rdf-tests) revision. This directory
runs the **same** runner's discovery, test selection and result comparison
against other stores, so the numbers sit in one table on equal terms.

## Results at rdf-tests `369a90d`

Primary columns use **RDF term equality**, the rule quipu itself is held to.
"Same value" counts failures whose answer had the right values but a
different lexical form (for example `"1"^^xsd:decimal` for `"1.0"^^xsd:decimal`).
Those stay failures and are reported separately, so a design choice is not
presented as a wrong answer.

| system | version | query evaluation | of those failures, same value | update |
|---|---|---:|---:|---:|
| quipu | 0.8.0 | 168/168 | 0 | 37/37 |
| Oxigraph | 0.5.11 | 159/168 | 8 | 37/37 |
| rdflib | 7.6.0 | 154/168 | 9 | 27/37 |
| Jena Fuseki | — | pending | | |
| RDF4J | — | pending | | |

The quipu row is quipu's own published ledger, produced by the quipu runner through its CLI
rather than by a driver here. It uses the same discovery, selection and comparison code, but the
comparison is stricter: exact labels, and no same-value tag.
Per-case ledgers: [`results/`](results/). Jena Fuseki and RDF4J need a pinned
JVM and are the next step.

**Disclosure.** Quipu parses SPARQL with `spargebra` and models RDF with
`oxrdf`, both from the Oxigraph project. Where the two agree on syntax, part
of that agreement is shared code.

## How it stays fair

- **One comparison.** Results travel as SPARQL JSON and are parsed by the
  quipu runner's own `expected_json`. Graph answers, and the dataset after an
  update, are compared by loading the expected graph into a fresh instance of
  **the system under test** and dumping both sides the same way. Differences
  in how two parsers print a literal cannot score as wrong answers. Blank
  nodes compare up to one consistent renaming.
- **Same conditions.** Each system gets a fresh in-memory store for every case,
  the same 30 s timeout, and fixtures parsed with their file URI as the base
  IRI, as quipu parses them.
- **Pinned.** Binaries are pinned by version and sha256, and a mismatch is a
  refusal. rdflib is pinned by the `uv run --with rdflib==<version>`
  invocation, and every ledger records the version it ran.
- **Every competitor failure is triaged before it is published:** competitor
  defect, harness defect, or spec ambiguity. A harness defect is fixed for
  every system and every system is re-run. Harness defects found so far, each
  now pinned by a test in [`test_competitors.py`](test_competitors.py):
  - relative IRIs in RDF/XML fixtures needed a base (`:subquery06`)
  - language tags are case-insensitive in RDF 1.1 (`:strlang02`)
  - a simple literal is an `xsd:string` literal in RDF 1.1 (`:ucase01` and 15 more)

## Run it

```bash
git clone https://github.com/w3c/rdf-tests /tmp/rdf-tests
git -C /tmp/rdf-tests checkout 369a90d1a60c021b746df2e411da0ff36258a758
python3 benchmark/competitors/competitors.py run --system oxigraph \
  --suite /tmp/rdf-tests/sparql/sparql11 --output /tmp/oxigraph.json
uv run --no-project --with rdflib==7.6.0 python3 benchmark/competitors/competitors.py run \
  --system rdflib --suite /tmp/rdf-tests/sparql/sparql11 --output /tmp/rdflib.json
python3 -m unittest benchmark/competitors/test_competitors.py
```

`competitors.py provision oxigraph` downloads and verifies the pinned Oxigraph
binary and prints its path. The WatDiv performance comparison uses the same
binary.

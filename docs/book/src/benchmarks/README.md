# Public benchmarks

> **Read this before quoting any number from this section.** Every benchmark class
> below is scored **separately** and is never combined into a single figure. A
> blended score would hide exactly the classes that are unimplemented, unrun, or
> measuring somebody else's system. Unrun classes are published as **NOT RUN**
> rather than omitted — a page that lists only what went well flatters by silence.

This page is an **index**, not a ledger. Numbers produced by Quipu's own runners
are published here and re-derivable from this repository. Numbers produced
elsewhere in the stack stay in the repository that produced them and are **linked
with the commit that published them** — they are not copied into a table here,
because a copied number rots silently while its source moves on.

| Class | What it measures | Published by | Status |
|---|---|---|---|
| [SPARQL 1.1 conformance](conformance.md) | Quipu's query engine against the W3C RDF Tests at a pinned revision | this repository | **published**, re-derivable |
| [Extraction → ingress](#extraction--ingress-text2kgbench) | a governed RML write of frozen upstream extractions into a disposable Quipu | [caboodle](https://github.com/scbrown/caboodle) `0a1b169` | **published**, with the boundary below |
| [Performance](#performance-watdivlubm) | WatDiv / LUBM against Oxigraph | — | **NOT RUN** |

## Extraction → ingress (Text2KGBench)

Caboodle publishes a pinned Text2KGBench run at
[`evaluations/text2kgbench/results/2026-09-03/report.json`](https://github.com/scbrown/caboodle/blob/0a1b169/evaluations/text2kgbench/results/2026-09-03/report.json)
(commit `0a1b169`). Read it there; this page deliberately does not restate its
score table.

**What it is a measurement of, in the report's own words** (`evaluation_scope`):

> separate boundary measurements; upstream artifact is not graph-extract output

That sentence is the whole reason this section exists rather than a row of F1
numbers on Quipu's trust page:

- The **extraction** half is a `frozen_upstream_replay` of the dataset's own
  **Vicuna-13B** baseline responses, hash-pinned and re-scored. It measures a
  third-party model on 25 cases (`selection.method: first_n_in_gold_file_order`,
  so a fixed prefix, not a random sample). **It is not a measurement of Quipu, of
  Caboodle, or of `graph-extract`**, and quoting its F1 as one would be wrong in
  both directions: it is neither our credit nor our fault.
- The **ingress** half is ours and is the number this project can stand behind:
  127 input triples materialised to **635 quads, all conforming**
  (`ingress.write.conforms: true`, `count: 635`), through a governed Camayoc RML
  write into a disposable store, with the mapping and source both hash-pinned.

So the honest one-line summary is: *the pipeline ingests a third party's
extractions into a governed graph without dropping or mangling any of them.* The
quality of the extractions themselves is the upstream baseline's, and improving
on it is a separate, unrun benchmark.

## Performance (WatDiv/LUBM)

**NOT RUN.** No WatDiv or LUBM figures exist, against Oxigraph or anything else,
and none should be quoted from anywhere until a pinned runner produces them here.

This row exists so the absence is visible. The rule for this section is that a
class with no result is published as NOT RUN and kept in the list, because the
alternative — leaving it out until it looks good — is how a benchmark page stops
being evidence and becomes marketing.

## The rules this section is held to

Inherited from the benchmark programme, and stated here so a future page cannot
quietly drop one:

1. **Classes stay separately scored.** Never blended into one compliance
   percentage.
2. **Every number comes from a version-pinned, checked-in runner** that exits
   non-zero on regression. `just conformance-check` enforces this for the
   conformance page: the committed ledger and the rendered page must agree.
3. **Unsupported and unrun cases stay in the denominator**, each with a named
   reason. NOT RUN is published as NOT RUN.
4. **Cross-repository results are indexed and linked, never copied.** A copy is a
   number with no owner: it cannot be re-derived from the page that shows it, and
   it goes stale without anyone editing it.
5. **A result carries the boundary of what it measured.** The extraction section
   above is the worked example — the same figures, published without their
   scope, would assert something about Quipu that nobody measured.

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
| [Bulk ingest](#bulk-ingest-watdiv) | Quipu's own load rate for a pinned WatDiv dataset | this repository (`benchmark/public/watdiv_ingest.py`) | **published**, re-derivable |
| [Performance](#performance-watdivlubm) | WatDiv / LUBM query latency against Oxigraph | — | **NOT RUN** |

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

## Bulk ingest (WatDiv)

**This is NOT the Oxigraph comparison.** It measures one thing: how fast Quipu loads a pinned
third-party dataset into a fresh store. No other engine is involved, and nothing here says
anything about query latency. The comparison class below remains NOT RUN.

### The measurement

**Quipu ingested a 10,916,457-triple WatDiv dataset in 2,848.9 s — 3,831.8 live facts per second
— into a 3,227,811,840-byte store (295.7 bytes per fact).** Release build, single process,
`--chunk 50000`, on an idle-to-moderately-loaded 20-core host with 66 GB RAM.

The population appears in the same sentence as the rate deliberately: WatDiv's "10M" archive
contains 10,916,457 triples, not 10,000,000, and a rate quoted "at 10M" would be wrong by 9%
before anyone checked anything else.

| | |
|---|---|
| dataset | WatDiv 10M archive, `sha256 1d0a8a47…`; extracted N-Triples `sha256 7cfe0341…` |
| triples | 10,916,457 declared, 10,916,460 live facts after load |
| wall time | 2,848.9 s |
| rate | 3,831.8 live facts/s |
| store | 3,227,811,840 B = 295.7 B/fact |
| build | release |

**Why the fact count exceeds the triple count by exactly 3:** a declared ingest writes three
completion markers (declared count, source digest, completion) into the graph. That identity is
the load's own anti-vacuity check — a silently truncated load cannot produce it.

**Throughput is a before/after delta of live facts read from the store**, never the loader's
parse count. The two differ: the parse count reports triples the parser saw, and a re-ingest of
identical content parses everything and writes nothing.

### What this number does NOT support

* **It is not a per-triple constant.** Rate varies ~10x within a single run (below), so a figure
  taken from part of a load is not the load's rate. Only end-to-end figures are quoted here.
* **It does not extrapolate.** See the refuted hypothesis below.
* **It is not a comparison.** Quipu and Oxigraph share the SPARQL parser and RDF data model
  (`spargebra`, `oxrdf`), so any future comparison measures storage and evaluation layers, never
  independent engines, and must say so.

### Rate is NOT constant within a load, and the obvious explanation is wrong

Instrumented per committed chunk, the first quarter of a 10,916,457-triple load runs about **10x
slower** than the rest (first-quartile median ~1,099 facts/s; later quartiles at or above the
instrument's resolution). Correlation with host load average is **-0.09 across 2.1-8.7** — that
is, essentially none — and with position in the run **+0.49**, over 161 commit intervals.

The obvious reading is that the transition happens after a fixed number of facts. **That is
refuted.** A 108,997,714-triple load of the same dataset family, same binary, same chunk size,
was **14x slower at the same absolute commit count** (commits 41-55: 1,263 facts/s, against
18,333 facts/s at those commits in the smaller load). Whatever causes the speed-up, it is not
"N facts ingested".

Two explanations remain untested and are recorded rather than chosen: a proportional effect (the
transition at some fraction of the dataset) and working-set residency (the smaller store is
3.2 GB and caches readily; the larger is 32.3 GB). They are not equivalent — the first says the
cost never amortises, the second says it amortises whenever the store fits in memory.

### Re-deriving it

```text
python3 benchmark/public/watdiv_ingest.py --scale 10M \
  --archive <watdiv.10M.tar.bz2> --quipu <release quipu> \
  --db <scratch>.db --output benchmark/public/results/watdiv-ingest.jsonl \
  --pins benchmark/public/results/watdiv-pins.tsv
```

The archive is fetched once from the published WatDiv site; the runner pins its digest on first
sight and **verifies** it afterwards, aborting on a mismatch rather than benchmarking bytes
nobody pinned. The source is **streamed from the archive** and never unpacked — at the 100M scale
the extracted form is ~15.6 GB, which would double the footprint of a run designed to leave
nothing behind.

Guards that decide whether a row may be quoted, each covered by a test in
`benchmark/public/test_watdiv_ingest.py`:

* a **non-zero exit** or a **contended host** marks the row `valid_result: false` with the reason
  named — the row is still written, because an unlabelled fast number is the hazard, not a
  labelled slow one;
* an **unreadable store** reads as UNKNOWN rather than a zero baseline, which would otherwise
  inflate the delta by whatever the store already held;
* an archive with **no `.nt` member** is refused rather than silently benchmarking the first file
  it finds.

## Performance (WatDiv/LUBM)

**NOT RUN.** No WatDiv or LUBM **query latency** figures exist against Oxigraph or
anything else, and none should be quoted from anywhere until a pinned runner produces
them here. The bulk-ingest section above is a different class and is not a substitute:
a load rate says nothing about how fast either engine answers a query.

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

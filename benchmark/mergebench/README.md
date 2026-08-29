# mergebench — the divergence benchmark

One seeded base graph, two independently edited copies, eight merge strategies
scored against one oracle. No LLM anywhere in the loop: the writers are
deterministic drivers, so the run is its own ground truth.

The paper this feeds is the shape-aware three-way merge paper. Read
[`BUILD_REPORT.md`](BUILD_REPORT.md) — the honesty record — **before quoting a
number from here.** It states what the synthetic arm can and cannot establish,
and one of the plan's own hypotheses does not survive it.

## Run

```bash
just bench merge                                   # seed 42, defaults
just bench merge --seed 7 --entities 400 --edits 400 --overlap 0.8
just bench merge --sweep                           # wall time vs graph size
just bench merge --selftest                        # prove the instruments
```

Outputs land in `benchmark/mergebench/out/` (gitignored):

- `metrics-seed<N>.json` — every metric for every arm.
- `RESULTS-seed<N>.md` — the same numbers as a markdown table, with the command
  that produced them in its header. Prose cites this file; nothing is retyped.
- `sweep-seed<N>.json` — the scale sweep.

To audit the two line-merge arms, keep git's raw output:

```bash
MERGEBENCH_DUMP=/tmp/dump just bench merge     # writes merge-<form>.out
```

## Layout

- `examples/mergebench/` — the harness.
  - `shapes.rs` — the benchmark's SHACL contract. The ONLY place cardinality
    lives: the generator, the operator, and the post-merge validator all read
    it. A synthetic `bench:` vocabulary with no relation to any deployed
    ontology, so the whole synthetic arm is publishable without a scrub pass.
  - `model.rs` — triples, graphs, and the two serialisations the line-merge
    arms are measured on.
  - `generate.rs` — the base graph, the two divergent edit streams, and
    `ground_truth`, which derives the oracle from the three graphs and the
    shapes rather than from what the generator intended.
  - `strategies.rs` — the eight arms.
  - `score.rs` — the metrics.
  - `selftest.rs` — the instrument controls.
  - `rng.rs` — `SplitMix64`. The seed is the only entropy in the whole run.

## The arms

| arm | what it is |
|---|---|
| `git-turtle-reserialized` | `git merge-file` on subject-grouped Turtle, independently re-serialised per side |
| `git-turtle-stable` | the same, with one stable order for all three inputs — the BEST case for a line merge |
| `git-canonical` | `git merge-file` on the sorted triple set (a share bundle's `export.nt`) |
| `union` | ours ∪ theirs |
| `lww-theirs` | last-writer-wins at slot granularity |
| `triple-3way` | set-algebraic triple-set three-way merge, no schema knowledge |
| `context-merge` | node-overlap heuristic — the Quit Context Merge shape, and the nearest neighbour in the related work |
| `shape-aware` | set algebra, with the shapes graph deciding which slots the algebra may settle |

The line-merge arms are the real `git merge-file`, not a reimplementation of
it: a baseline a reviewer cannot check against the tool it claims to be is not
a baseline. Where `git` is absent those arms report **unavailable**, never zero.

## The accounting contract

Every arm answers the same two questions, and the answers are in tension by
construction:

- **`human_decisions`** — slots it refused to settle. A slot it hands over is
  held at its BASE value in its output, so it cannot flag a conflict *and* land
  a value for it.
- **`shacl_violations`** — violations in what it landed automatically, against
  the same shapes that defined the conflicts. This is corruption it admitted
  without charging anyone for it.

Either can be driven to zero by sacrificing the other, so neither is reportable
alone. `triples_lost` / `triples_spurious` are the third leg: they catch the arm
that admits no corruption *and* asks no questions because it quietly discarded
one side's work.

## Metrics whose zero you must not trust on sight

`--selftest` exists because several columns here are unfalsifiable from
outside the harness. It proves, and exits non-zero if it cannot:

- the SHACL validator reports a violation when one is present **and** none when
  the graph is legal (both directions — a checker that never fires and one that
  always fires produce indistinguishable columns);
- two runs at one seed are identical;
- the oracle is recomputable from the three published graphs alone;
- `git-turtle-stable` is genuinely stably serialised (a regression: it was not,
  and reported 121 unparseable lines that were the harness's fault, not git's);
- an absent `git` is reported as unavailable rather than as a clean sweep;
- every arm holds its conflicted slots at base.

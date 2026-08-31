# mergebench — build report

The honesty record for the divergence benchmark: what was built, what it can
establish, what it cannot, and which designs were discarded. Read this before
quoting a number from `RESULTS-seed<N>.md`.

Provenance: built to the project's recorded plan and novelty ruling, as Arm A of
the evaluation. (Tracker references removed for public release — a private
tracker id is unresolvable to a reader of this repository.)

## 1. What the harness does

One seeded base graph over a synthetic `bench:` vocabulary. Two copies are then
edited **independently** — separate RNG streams, so neither side's edits can be
a function of the other's — using the six-operation edit model from the plan:
assert, retract, re-describe, re-type, alias-mint, node-add. An `--overlap`
parameter sets how often an edit aims at the contended quarter of the entity
set, so collision probability is controlled directly rather than left to a
birthday effect.

Eight strategies then merge `(base, ours, theirs)` under one accounting
contract, and are scored against one oracle.

## 2. Claim boundaries

**Ground graphs only.** The benchmark's graphs contain no blank nodes, so
canonicalisation here is sorting. RDFC-1.0 / URDNA2015 is the substrate the
paper builds on and is **not exercised by this arm**; nothing here is evidence
about blank-node canonicalisation.

**One machine, one process, single runs.** Wall times are indicative. The
line-merge arms include process spawn for `git merge-file` (order 5 ms), which
dominates their timing at every size measured — those columns are not
comparable with the in-process arms and must not be reported as if they were.

**Slot granularity.** A conflict is reported over a `(subject, predicate)`
slot, because that is the unit `sh:maxCount` is declared over. An operator
working at a different granularity would need a different scorer.

**Synthetic edit distribution.** The edit model is uniform over six operations
and an overlap parameter. Real agent traffic is not uniform. That is Arm B's
job (`benchmark/replay`), and no claim about real divergence rates may be
sourced from this arm.

## 3. The circularity, stated rather than managed away

The shape-aware operator and the oracle both read the same shapes graph, and
for the three triple-visible conflict classes their rules coincide **by
construction**. Its precision of 1.000 on the synthetic arm is therefore a
property of the design, not evidence about it.

This is not hidden by making the oracle cleverer. It is handled three ways:

1. **The oracle is recomputable by a reader, with one stated exception.** For
   the three triple-visible classes `ground_truth` derives the oracle from the
   three graphs and the shapes and nothing else — no edit log, no generator
   intent — and `--selftest` proves a reader holding only those inputs
   recomputes them exactly, printing the count it recovered. An oracle derived
   throughout from the generator's intent would score the generator and would be
   uncheckable by anyone else.

   **The alias-mint class is the exception, and it is not optional.** Two names
   for one entity is by construction invisible in the triples — that is §4, the
   paper's own limitation, and the reason the class is in the benchmark at all —
   so no oracle could recover those slots from the three graphs. `ground_truth`
   therefore takes the two alias maps as arguments alongside the graphs, and the
   selftest says so in its own PASS line rather than in a footnote:

       PASS  oracle-recomputable: 8 triple-visible conflicts recovered from the
             three graphs alone (4 alias conflicts need the mint log, as documented)

   What that costs is bounded and worth stating: across seeds 1, 7, 42, 99 and
   2026 the class is **21 of 54** oracle conflicts, and **every arm detects 0 of
   them, the shape-aware one included**. So it bounds the recall column and
   nothing else — no arm's precision or true-positive count rests on a number a
   reader cannot recompute.
2. **Results are reported per conflict class**, so a reader can see exactly
   which rows are definitional and which are not.
3. **The synthetic arm's contribution is the COST of the alternatives**, not
   the correctness of ours. Correctness evidence for the operator is Arm B,
   which replays recorded multi-agent divergences against the SHIPPED operator
   rather than this reference implementation. A discrepancy there is a real
   finding; agreement here is not.

## 4. The class the shape-aware operator cannot see

`alias-mint` — both sides minting differently-named nodes for the same entity —
is in the oracle and is missed by **every** arm, including the shape-aware one.
Two names for one thing is not visible to any triple-level operator, so a
recall of 1.000 is not achievable by anything in this table.

It is in the benchmark deliberately. It is the most common real divergence
defect in the recorded corpus, and excluding it would have produced a
shape-aware arm scoring 1.000 on both precision and recall, which would have
been an artefact of what we chose to measure. The false-negative column is a
reported result.

## 5. A hypothesis that did not survive

The plan's **H3** stated that canonicalisation alone already beats
non-canonical line merge materially. Measured across seeds 1, 7, 42, 99 and
2026, that is **true against the re-serialised arm and false against the stable
one**:

- versus `git-turtle-reserialized`, canonicalisation is a large win — it
  removes ordering churn entirely, taking unparseable output lines to zero;
- versus `git-turtle-stable`, canonical N-Triples raises **more** conflicts,
  not fewer. Sorting scatters each subject's triples into a single sorted run,
  so two sides' additions land adjacent and collide, where subject-grouped
  Turtle had absorbed them into one block.

The honest statement is therefore narrower than the hypothesis: canonicalisation
buys **syntactic integrity and independence from serialisation order**, and it
does *not* by itself reduce the number of conflicts below a stably serialised
Turtle file. The paper must say that; it is a trade, not a win. Regenerate both
with `just bench merge --seed <N>` and compare the `git-turtle-stable` and
`git-canonical` rows.

## 6. Discarded designs

**Ground truth as a log of planted edits.** Rejected: it makes the oracle a
record of the generator's intent, so no reader with the three graphs can check
a single number, and any generator bug becomes invisible by definition.

**Reporting a conflicted slot's values in the merged graph.** Rejected: an arm
could then flag a conflict *and* land both values, scoring well on the decision
column and the corruption column at once. Conflicted slots are held at base
instead, which `--selftest` enforces across all arms.

**A hand-written three-way line merger.** Rejected: a line-merge baseline that
is not the tool it claims to be cannot be checked by a reviewer. The arms shell
out to real `git merge-file`, and report **unavailable** where git is absent —
never zero, which would render as a perfect score.

**Splitting conflict hunks from clean text before parsing.** Implemented, then
removed after measurement: it stripped each hunk's subject header (which lives
in the clean text above the marker) and reported 121 unparseable lines on an
arm whose output git had written perfectly well. That number measured the
harness. Turtle statement state crosses marker boundaries, so the reader has to
as well; it is now a single sequential pass.

**A single "git-turtle" arm.** Rejected as a straw man once measured. With each
side independently re-serialised, line merge conflicts on nearly everything —
true, but it invites the reply that nobody re-serialises. The stable arm is the
strong form of the baseline and is the one the paper must argue against;
keeping both makes the ordering effect an explicit ablation rather than a
hidden assumption.

## 7. Instrument controls

`just bench merge --selftest` proves seven properties and exits non-zero on
any failure. Two are worth naming here because the metrics they guard are
otherwise unfalsifiable:

- **The SHACL validator is exercised in both directions** — a legal graph must
  conform with zero violations, and two values on a `sh:maxCount 1` predicate
  must not. Without both, a validator that never fires and one that always
  fires produce the same columns, and every `0` in the corruption column would
  be worthless.
- **`git-turtle-stable` is genuinely stably serialised.** It was not: the
  within-subject permutation was seeded `seed % len`, and the two sides hold
  different numbers of triples, so the "stable" arm was re-serialised churn
  wearing a stable arm's name. Both outcomes of this check have been observed —
  reintroducing the defect fails it on 122 of 200 shared subjects.

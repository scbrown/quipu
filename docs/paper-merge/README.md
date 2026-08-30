# The merge paper source

`main.tex` + `sections/` + `references.bib` — an arXiv-style paper on
shape-aware three-way merge for RDF knowledge graphs.

**This is the SECOND paper in this repository.** `docs/paper/` is the Quipu
store paper ("A Governed Bitemporal Knowledge Graph Store"), and it was already
occupied — hence this directory and its own recipe. Build with
`just paper-merge` (tries tectonic, latexmk, then pdflatex).

## Where the numbers come from

Nothing here is hand-carried. Every figure regenerates:

| section | source | command |
|---|---|---|
| Arm A (§5) | `benchmark/mergebench/` | `just bench merge --seed <N>` |
| Arm A controls | — | `just bench merge --selftest` (7 properties) |
| Arms B/C (§6) | `benchmark/replay/` | `cargo run --example replay --features shacl` |
| Arm B control | — | `cargo run --example replay --features shacl -- --negative-control` |

Read each benchmark's `BUILD_REPORT.md` — the honesty record — **before quoting
a number**. They state what each arm can and cannot establish, and one of the
plan's own hypotheses does not survive Arm A.

The Arm A table in §5 aggregates seeds 1, 7, 42, 99 and 2026 at 200 entities,
400 edits and overlap 0.5. `benchmark/mergebench/out/` is gitignored, so
regenerate before checking a range.

## Open before submission

- [ ] **One bibliography entry remains unverified: `ibanez2012suset`.** The
      other five were checked against Crossref or the publisher on 2026-08-29
      and carry resolved DOIs; the Quit Store entry matched exactly, and the
      R43ples entry was **corrected** by the pass (the remembered 2014 workshop
      paper became the verified 2016 SEMANTiCS one). SU-Set is not Crossref
      indexed, as expected for a 2012 workshop paper, so its venue and year rest
      on recollection. Confirm it from the authors' publication list, or cite
      the CRDT-for-RDF line generically in §8 instead.
- [ ] **Confirm the Context Merge description in §8 against Quit Store itself.**
      The citation is verified; that the *sentence* fairly describes what Quit
      Store specifies is a separate check, and the paper's whole
      no-priority-claim position rests on it.
- [x] ~~Build the PDF.~~ Compiles clean with tectonic 0.17.0: 11 pages, 0
      unresolved references or citations. `just paper-merge` finds no engine on
      hosts without one — that is a host gap, not a source problem.
- [ ] **Run the scrub gate over the finished artifact**:
      `scripts/arxiv-scrub-gate.sh` — it reports INCOMPLETE until this directory
      exists, and it gates the paper source with no path exemption.
- [ ] **Author metadata**: `main.tex` mirrors the store paper's author block.
      Confirm the acknowledgement and artifact-availability text, and add a
      Zenodo DOI if this artifact is archived separately.

## Claim discipline

Constrained by the novelty ruling, and the constraint is not cosmetic:

- **No "first" claims.** Not first RDF three-way merge, not first semantic RDF
  merge, not first cardinality-aware ontology merge. Git-backed RDF versioning
  with merge strategies is established prior work.
- Quit Store's Context Merge is the **prior-art baseline this operator
  extends**, and it is in the benchmark as an arm rather than described from a
  distance.
- Contribution is **implementation and evaluation** of SHACL-derived conflict
  semantics, with "to our knowledge" scoped to the searched literature.
- Claims stay scoped to the arms actually run. The synthetic arm's precision is
  definitional and §5 says so; the 939 figure is a counterfactual and §7 says
  so; the alias blindness is reported as a measured column, not omitted.

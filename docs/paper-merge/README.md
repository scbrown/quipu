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

Everything below that a crew member can settle is settled. What remains is
Stiwi's to do, and is marked so.

- [x] ~~One bibliography entry remains unverified: `ibanez2012suset`.~~
      **Resolved 2026-08-30 — by finding the remembered record does not exist.**
      There is no paper titled "SU-Set: An Operation-based CRDT for Triple
      Stores": zero hits in DBLP (checked against Skaf-Molli's complete author
      record, not a keyword search), Crossref and OpenAlex. SU-Set is the name
      of the **CRDT**, not of a paper. It is defined in Ibáñez et al.,
      *Synchronizing Semantic Stores with Commutative Replicated Data Types*,
      WWW '12 Companion, pp. 1091–1096, `10.1145/2187980.2188246` — whose
      publisher abstract ends "In this paper, we define SU-Set, a CRDT for
      RDF-Graph that supports SPARQL Update 1.1 operations." The remembered
      author list was exactly right, which is why the entry looked sound. The
      key is unchanged; the entry's `note` carries the name and the 2013 IJMSO
      extended version. **All six entries now carry a resolved identifier.**
- [x] ~~Confirm the Context Merge description in §8 against Quit Store itself.~~
      **Checked 2026-08-30 against the paper's own §8.4**, read from the source
      PDF rather than from the abstract. Every claim held: Quit is "Quads in
      Git"; Context Merge "produces merge conflicts, as soon as the changes of
      both merged commits overlap at a node", marking the subject and object of
      each added or removed statement with its originating commit. §8 was
      **strengthened for fairness, not corrected for error**: it now says Context
      Merge *identifies* conflicts and hands them to a person (its own section
      heading is "A Supervised Approach to Identify Conflicts") rather than
      "reconciles"; it records that avoiding semantic rules is Quit Store's
      stated, deliberate choice rather than an oversight; and it frames 845-vs-33
      as two different questions being asked, not one strategy failing at the
      other's task. The no-priority-claim position is stronger for it.
- [x] ~~Build the PDF.~~ Rebuilt 2026-08-30 after the above: 11 pages, **0
      unresolved references or citations, 0 BibTeX warnings** (the one remaining
      warning, an unsortable authorless `terminusdb` entry, was fixed with a
      `key`). `main.pdf` is committed and tracks this source.
- [x] ~~Run the scrub gate over the finished artifact.~~ **PASS**, exit 0, all
      three arms with controls proven, 7/7 components. Re-run after every source
      change; it is cheap.
- [x] ~~Author metadata.~~ Acknowledgement and artifact-availability text
      confirmed; ORCID present. `\date` is now **pinned** rather than `\today`,
      because `main.pdf` is committed and a floating date made every rebuild
      differ from the committed artifact for a reason unrelated to the paper.
- [ ] **STIWI: the submission itself.** Public disclosure from his account. See
      `SUBMISSION.md` in this directory for the prepared metadata (categories,
      abstract, license) and the one decision that is still open — the reversible
      corpus blob still reachable in public history at `8b369b2`.
- [ ] **STIWI (optional): Zenodo DOI.** Only if this artifact is archived
      separately from the repository; nothing in the paper depends on it.

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

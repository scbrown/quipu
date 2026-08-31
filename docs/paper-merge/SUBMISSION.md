# arXiv submission card

**Submission is Stiwi's action.** This file is the prepared metadata so the
walk-through is a form-fill, not a research task. Nothing here has been sent
anywhere.

Prepared by arnold, 2026-08-30, against `main.tex` at this commit. If the source
changes, rebuild and re-read the abstract below — it is a transcription of the
PDF, not a second draft of it.

Re-transcribed by kelly, 2026-08-30, after the pre-submission review changed the
abstract's third paragraph (it reported 105 and 93 for the same quantity four
lines apart). The abstract below is a fresh extraction from the rebuilt PDF and
was diffed against it.

---

## 1. The one decision that is not a form field

`8b369b2:benchmark/replay/corpus/corpus.json` is on the public GitHub remote and
is **still fully reversible** — the labels were `sha256(salt + iri)[:10]` with
the salt committed in the public build script, so any candidate IRI can be
confirmed by recomputing the digest. Re-measured after the fix: 24 of 27 probe
names recovered from that blob. Host names, service names and the crew roster.

The current tree is clean (resealed from a CSPRNG, map discarded, gate PASS).
**A later scrub commit does not fix history**: a push publishes every object, and
the blob stays reachable by sha.

Rewriting public history on Stiwi's repo is his call. Recommendation:
**do it before the paper points readers at the repository** — the window is small
and it is the cheapest it will ever be. If instead we accept it, that should be a
recorded decision rather than something that happens by default.

This does not block submission. It is only cheap *before* it.

---

## 2. Form fields

**Title**

    Shape-Aware Three-Way Merge for RDF Knowledge Graphs:
    The schema defines the conflict — an implementation and evaluation

**Authors**

    Steve Brown (ORCID 0009-0009-1720-1785)

The `\thanks` block in `main.tex` carries the acknowledgement of implementation
and drafting assistance from Claude (Anthropic), and the artifact-availability
statement. Both were reviewed 2026-08-30 and are accurate as written.

**Primary category** — recommended: `cs.DB` (databases).
The paper's contribution is a merge operator and its evaluation in a store.

**Cross-lists** — recommended: `cs.SE` (the structured-merge lineage the paper
places itself in) and `cs.AI` (multi-agent knowledge graphs, which is where the
replay corpus comes from). Add `cs.DC` only if a reviewer asks; the paper is not
about distribution.

**Comments field** — suggested:

    11 pages. Artifact, benchmark and replay corpus at
    https://github.com/scbrown/quipu ; every table names the command that
    regenerates it.

**License** — recommend `CC BY 4.0`. The artifact is already public; a
non-commercial or no-derivatives licence would sit oddly beside it. This is a
one-way door on arXiv: the licence cannot be loosened after announcement.

**Abstract** (plain text for the web form — no LaTeX, transcribed from the built
PDF):

Version control works for source code because its merge understands the medium: lines. Knowledge graphs shared between people and software agents are not lines, and merging their serialisations makes conflicts out of ordering and misses the ones that matter. We describe and evaluate a three-way merge for RDF in which the schema decides what a conflict is: the merge is set algebra over canonical triples, and a slot is contended only where a SHACL shape declares that it can hold at most one value. Multi-valued predicates union; functional predicates with divergent values are handed to a person. The same shapes graph then validates the merge result, so the schema that defines conflicts also audits their resolution.

We do not claim the first three-way merge for RDF. Git-backed RDF versioning with merge strategies is established prior work, and the closest neighbour -- Quit Store's Context Merge -- is included here as a baseline that this operator extends rather than replaces. Our contribution is an implementation in a production multi-agent store and an evaluation of what schema-derived conflict semantics buys against that neighbour and against six other strategies.

On a synthetic divergence benchmark over five seeds, the shape-aware operator raised 33 conflicts, all true positives, with no false positives, no triples lost, none fabricated, and no SHACL violations admitted. The context-overlap baseline detected the same 33 conflicts and asked 845 questions to do it. We also report a limit no schema removes: when two sides mint different names for one entity, the divergence is invisible to any triple-level operator, including this one. Replaying a recorded production corpus against the shipped operator -- 105 repairs a person actually performed, 93 of them after excluding 12 chained pairs, and 939 real duplicate-value incidents -- reproduces both the blindness (0 of 93 alias repairs, matching 0 of 21 on the synthetic arm) and the benefit, on data nobody generated for the purpose.

---

## 3. What to upload

arXiv prefers LaTeX source over a PDF. Upload the source; it builds there.

    main.tex  references.bib  sections/*.tex

`main.bbl` is **not** committed. arXiv runs BibTeX, so source-only is fine — but
if the build fails on their side, generate `main.bbl` locally and add it, which
is the standard remedy. Do not upload `main.pdf` alongside the source; arXiv
takes one or the other.

---

## 4. Verification state at hand-off

| check | result |
|---|---|
| `arxiv-scrub-gate.sh` | **PASS**, exit 0 — 3 arms, every control proven, 7/7 components |
| bibliography | 6 entries, **all six** with a resolved identifier; 0 unresolved citations |
| BibTeX warnings | **0** |
| build | tectonic, 11 pages, 0 unresolved references |
| bead ids in the artifact | **0** — the one advisory (`examples/replay/main.rs`) is gone |
| policy projection | the gate was run against the config that the pre-push guard's own selftest **passes**; see the note below |

**Note on the policy projection, because it is the one thing a reader should not
take on trust.** Regenerating the pattern config on 2026-08-30 produced a config
that **breaks its consumers**: the private-IPv4 arm contained `\d`, which
`grep -E` reads as a literal `d`, so it matched nothing. The gate caught it and
refused to report (exit 2, "CONTROL FAILED"); the pre-push guard's selftest
caught it independently. The working config was restored and re-verified by
selftest — failing under the bad config, passing under the good one, both
observed — before the gate result above was taken.

The cause was **not** a defect in the generator. It was that the generator was
run from a checkout 426 commits behind its origin; the current one translates the
construct and refuses to emit anything `grep -E` would misread. The installed
config was then proven byte-identical to a fresh generation from current source,
so the projection behind the verdict above is current, not merely working.

None of this touches the artifact. It is recorded because "the gate passed" is
worth exactly as much as the instrument behind it, and for four minutes that
instrument was provably dead — which is the situation this gate's controls exist
to make visible rather than survivable.

---

## 5. Rebuilding

    just paper-merge          # tries tectonic, latexmk, then pdflatex

`tectonic` is installed at `~/.local/bin/tectonic` as of 2026-08-30. Before that
no TeX engine existed on any fleet host and each rebuild fetched one into a
session scratchpad, which is why two builds of this paper report two different
tectonic versions.

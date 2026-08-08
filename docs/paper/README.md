# The paper source

`main.tex` + `sections/` + `references.bib` — the arXiv-style systems
paper drafted from the design docs and the measured Census results.
Build with `just paper` (tries tectonic, latexmk, then pdflatex).

Provenance: plan in `docs/design/paper.md`; principles and defect
catalogue in `docs/design/paper-principles.md`; every evaluation number
traces to `benchmark/census/` (manifest, metrics, `DETERMINISM.md`,
`BUILD_REPORT.md`, `wild/`, `agent/`).

This directory is outside the mdBook and its lint globs on purpose.

# Conformance grammar: the versioned step-matching contract

> **Implementation status (2026-08-20, quipu-gp1/gp3):** 🟨 **Specified,
> versioned, and implemented by its first consumer.** `src/path/grammar.rs`
> implements `gp-grammar/1` and the trajectory backtest
> (`src/path/backtest.rs`) evaluates with it and carries the version on every
> report, per [golden-paths-blessing.md](./golden-paths-blessing.md) §4. The
> second consumer — yupana's conformance guard (FR-40/FR-41 in yupana's
> `docs/golden-path-guard.md`) — is still design-only and is bound by the
> carry/refuse rules below from its first cut.
>
> Created: 2026-08-20
> Status: SPECIFIED (v1) — consumers pending
> Related: [golden-paths-blessing.md](./golden-paths-blessing.md), camayoc
> `docs/design/golden-paths.md` (the ontology the step signature reads from).

## Why one contract

"Trajectory T conforms to golden path P" is decided in two places that must
never disagree: quipu's **backtest** (which justifies a promotion by replaying
a candidate path over history) and yupana's **guard** (which serves live
conformance verdicts against the promoted path). If the two implement even
slightly different matching, a promotion justified by the backtest enforces
something else at the guard — invisibly, because each side looks internally
consistent. So the definition lives here once, versioned, and every artifact
that used it says which version it used.

## Version identifier

A grammar version is the string `gp-grammar/<major>`, starting at
`gp-grammar/1`. Major only: any change to matching semantics is a new major
version, because there is no such thing as a backwards-compatible change to
what "conforming" means — a verdict under the new rules is not comparable to
a backtest under the old ones.

## The carry rules (binding on every consumer)

- Every **projected path** carries the grammar version its promotion was
  backtested under.
- Every **backtest report** carries the grammar version it evaluated with.
- Every **conformance verdict** carries the grammar version it applied, and
  it must equal the projected path's — a guard holding a path backtested
  under `gp-grammar/1` while implementing only `gp-grammar/2` REFUSES, and
  the path lands in the verdict's `unevaluated` list with the version
  mismatch named. Silent cross-version evaluation is the drift this document
  exists to prevent.
- A consumer receiving a version it does not implement reports the artifact
  as **unevaluated, never silently skipped** — the same rule yupana already
  applies to `selectorLang "sparql"`.

## gp-grammar/1

### Step signature

A trajectory step's signature under v1 is the pair:

```text
(actionKind, targetClass)
```

- `actionKind` — the step's `aegis:actionKind` string, compared exactly
  (case-sensitive).
- `targetClass` — the class of the step's `aegis:actionTarget` when the
  target is an IRI in the graph: the lexicographically smallest of its
  `rdf:type` IRIs (a deterministic pick when several), or the reserved word
  `untyped` when it has none. When the target is a literal, the reserved
  word `literal`; when the step has no target, the reserved word `none`.

A step with no `aegis:actionKind` has **no v1 signature** and is
**unevaluable**: it appears in the `unevaluated` list of any verdict or
backtest that encounters it. It never counts as a match and never counts as
a deviation — an unrecorded action kind is missing data, not misconduct.

### Path pattern

A golden path's v1 pattern is the ordered list of signatures of its kept
steps: the exemplar trajectory's steps in `aegis:stepOrder`, minus the steps
named by its `aegis:omitsStep` rulings. Dead-end steps (`aegis:deadEnd`) are
not part of the pattern; matching one is reported as a hazard note, never a
deviation by itself.

### Matching

Trajectory T conforms to path P under v1 iff the sequence of T's evaluable
step signatures contains P's pattern as a **subsequence** (in order, gaps
allowed). Formally: there exist indices `i1 < i2 < … < ik` into T's
evaluable steps such that `sig(T[ij]) = P.pattern[j]` for all `j`.

- **Gaps are allowed** because real work interleaves the path with local
  detail the path does not legislate; v1 constrains order, not exclusivity.
- **The first deviation point** — what FR-41/FR-42 report — is the position
  of the first pattern element that can no longer be matched by any
  remaining trajectory step, together with the earliest trajectory step
  after the last match (the follower's `aegis:deviatesAt` anchor).
- A **prefix in progress** (some but not all pattern elements matched, with
  trajectory still open) is *conforming so far*, distinct from both
  conformance and deviation; the guard reports it as such.

### What v1 deliberately does not do

- No similarity, no embeddings, no model judgment — signatures match exactly
  or not at all. Softer matching is a future major version, and lands in the
  inferred plane until it is one.
- No branching or optional pattern elements: one exemplar, one linear
  pattern. Multi-exemplar paths (pattern merge) are future work and will be
  a new major version, because merging changes what "the pattern" is.
- No time or actor constraints. `aegis:actor` is recorded and queryable but
  does not affect v1 matching.

## Serialization

Wherever a pattern travels (projection to yupana, a backtest report), it is
this JSON, so the two consumers parse one shape:

```json
{
  "grammar": "gp-grammar/1",
  "path": "<GoldenPath IRI>",
  "pattern": [
    {"actionKind": "edit", "targetClass": "literal"},
    {"actionKind": "run", "targetClass": "literal"},
    {"actionKind": "verify", "targetClass": "literal"}
  ],
  "deadEnds": [
    {"actionKind": "edit", "targetClass": "literal", "note": "<hazard label>"}
  ]
}
```

Unknown top-level keys are ignored (a minor addition is allowed to *add*
information); an unknown `grammar` value is the refuse case above.

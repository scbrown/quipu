# Cross-graph concept alignment

Status: implementation boundary for `aegis-sosiaa` (child of `aegis-iv3df7`).
Owner: malcolm. Reviewer: wu.

Directive (Stiwi, 2026-09-04, verbatim): *"For sharing, we need a way to allow
the operator to connect similar concepts across graphs."*

Sharing moves a graph. It does not move an opinion about what the graph's
concepts *are*. Two homelab stores that both know about the bobbin release
artifact call it `bobbin-release` and `Bobbin_release-artifact`, and after an
import you have both, forever, with no way to say they are one thing and no way
to say it once. This document specifies the step that closes that: **propose,
decide, record** — and it fixes the record as a first-class, shareable artifact
rather than a pile of edits to somebody else's graph.

## What exists today

Measured at `9cc4348`, 2026-09-04. Each claim carries its file.

- **A matcher already runs at write time.** `resolve_entity`
  (`src/resolution/mod.rs`) returns `EntityCandidate { iri, score, matched_on }`
  where `matched_on` is one of `canonical_name:exact` (score 1.0),
  `canonical_name:jaro_winkler:<n>` (`mod.rs:307`), or `embedding:<n>`
  (`mod.rs:158`). `/episode` surfaces these as `resolution_hints`
  (`src/mcp/mod.rs:43`) and same-episode collisions as `resolution_contentions`
  (`mod.rs:70`).
- **Import already aligns — destructively, unrecorded, without asking.**
  `resolve_and_rewrite` (`src/share_import.rs:207`) takes every `rdfs:label` in
  the incoming graph and resolves it against the local store. The auto-merge
  gate is `score == 1.0 && matched_on == "canonical_name:exact"`
  (`share_import.rs:217`) — an **exact canonical-name match**, not a score
  threshold; the fuzzy band above the 0.85 cut-off is report-only. On an exact
  match it **rewrites the foreign IRI to the local one in subject and object
  position** (rewrite loop `234-251`). Everything else lands in
  `ImportResolution.candidates` — a field on the HTTP response, and nowhere else.
- **There is no `same_as` primitive.** `quipu knot <file.ttl> [--graph <iri>]`
  (`src/cli.rs:55`, `src/mcp/knot.rs`) is bulk Turtle assertion into ROOT or a
  registered `committed` graph. `owl:sameAs` travels through it as ordinary
  triples, exactly like any other predicate. This is worth stating plainly
  because the bead and `CLAUDE.md` both read as though a sameAs verb exists.
- **Named graphs are registered, not conjured.** `knot` refuses an
  uninterned or unregistered graph IRI and names `graph_create` as the remedy
  (`knot.rs:43-65`); it refuses overlay-class graphs outright.
- **A share is already a verified, addressable artifact.**
  `manifest.ttl` + `payload.nq` + `shapes.ttl`, RDFC-1.0 canonical N-Quads,
  identity `urn:sha256:<digest>` over the canonicalized manifest, DCAT +
  PROV-O lineage — see [standard-share-artifact.md](standard-share-artifact.md).
  Import verifies every checksum before opening a destination store.

## The gap, stated precisely

Not "nothing asks the operator" — something does, badly:

1. The candidates below the exact-match bar are **reported and discarded**. The
   operator has no command that shows them again, and no command that acts on
   one.
2. A decision, once made by hand, is **not remembered**. Re-import the same
   colleague's graph next month and you re-do the same work, because nothing
   recorded that you did it.
3. A **rejection** is not expressible at all. "These two are NOT the same thing"
   is the operator's most valuable output — it is the only judgement a matcher
   can never re-derive — and today it has nowhere to live, so it is re-proposed
   forever.
4. The alignment is **not shareable**. Two operators aligning the same pair of
   graphs each do the whole job alone.
5. The one alignment quipu *does* perform is **destructive and provenance-free**:
   the foreign IRI ceases to exist, so the merge cannot be audited, explained, or
   undone, and no fact records that it happened.

Point 5 contradicts the principle the bead states as an assumption —
*"'similar' = candidates the operator confirms, never applied on score alone."*
Be precise about which half is violated. The gate is an exact canonical-name
match, so quipu is *not* merging on a fuzzy score — the narrow reading of that
principle holds. What it does without asking is **rewrite an IRI out of
existence** on the strength of two nodes sharing a label, and an exact name
match is not proof of identity: two graphs can each hold a `Repository` called
`bobbin` and mean different things by it. The defect is the destructiveness and
the silence, not the threshold. See
[Migrating the existing auto-merge](#migrating-the-existing-auto-merge).

## Decision 1 — the artifact is SSSOM

The alignment set is a **SSSOM mapping set**
([Simple Standard for Sharing Ontological Mappings](https://mapping-commons.github.io/sssom/)).
Do not invent a format.

Verified against the spec, 2026-09-04:

| fact | value |
| --- | --- |
| slot namespace | `sssom:` = `https://w3id.org/sssom/`, slot IRI = namespace + slot name |
| justification vocabulary | `semapv:` = `https://w3id.org/semapv/vocab/` |
| required `Mapping` slots | `subject_id`, `object_id`, `predicate_id`, `mapping_justification` |
| negative mappings | `sssom:predicate_modifier` with value `Not`, range `PredicateModifierEnum` |
| serialisations | SSSOM/TSV, SSSOM/JSON, SSSOM/RDF, OWL/RDF — implementations **MUST** support TSV, **MAY** support the rest |
| RDF shape | a `sssom:MappingSet` named by `mapping_set_id`, linked by `sssom:mappings` to members typed `owl:Axiom`, named by `record_id` or blank |

Why SSSOM rather than a quipu-native table: it already has the four things we
would otherwise get wrong. Negative mappings are in the model, not bolted on.
The justification is a controlled vocabulary rather than a free-text string, so
"why is this mapping here" survives the session that produced it. Confidence,
author and date are standard slots, so provenance is not a quipu dialect. And a
mapping set is text — diffable in review, and readable by `sssom-py` and the
wider mapping-commons tooling without us writing a converter.

`semapv` terms we will actually emit (all verified present in the vocabulary):

| our matcher | `mapping_justification` |
| --- | --- |
| `canonical_name:exact` | `semapv:LexicalMatching` |
| `canonical_name:jaro_winkler:<n>` | `semapv:LexicalSimilarityThresholdMatching` |
| `embedding:<n>` | `semapv:EmbeddingBasedMatching` |
| several signals combined | `semapv:CompositeMatching` |
| operator accepted or rejected | `semapv:ManualMappingCuration` |

The last row is the load-bearing one: **the justification recorded is the
operator's, not the matcher's.** What the matcher proposed is evidence, carried
alongside; what the operator decided is the mapping.

## Decision 2 — it lives in the graph as RDF, and exports as TSV

Two forms, one source of truth.

**In the store**, the mapping set is SSSOM/RDF written into a dedicated
registered `committed` named graph — `urn:quipu:align:<a-hash>:<b-hash>` —
created through `graph_create` like any other plane. It is not written into
either source graph. This is a hard boundary: an imported graph must stay
byte-recoverable against its own share hash, and it cannot if we edit it.

Writing it as RDF into a named graph is what buys the rest for free. The
alignment set is then an ordinary quipu share: `quipu share` packages it with
the existing manifest, canonicalization and `urn:sha256:` identity; `quipu
import` verifies it with the existing checksum path; it appears in SPARQL like
any other graph. A bespoke sidecar file would need all of that written again.

**On the wire**, `quipu align export --format sssom-tsv` is **required, not
optional** — the SSSOM spec makes TSV the mandatory serialisation, so a
TSV-less implementation is not an SSSOM implementation, and interop with
`sssom-py` is most of the reason to adopt the standard at all. `--format
sssom-json` and `--format sssom-rdf` are cheap once the model exists.

The `owl:sameAs` triples that make the alignment *do* something are **derived**
from the mapping set, written into the same alignment graph, and never authored
by hand. One direction of derivation, always: mapping set → knots. A knot with
no mapping behind it is a bug, and `quipu align verify` should say so.

Mappings whose `predicate_modifier` is `Not` derive nothing. They exist to
suppress a future proposal, and that is their whole job.

## Decision 3 — propose, decide, apply are three commands

They are separable because they fail differently, are re-run at different
cadences, and have different audiences. A single interactive `align` that does
all three cannot be run from CI, cannot be reviewed in a PR, and cannot be
resumed.

```text
quipu align propose <graph-a> <graph-b> [--out set.sssom.tsv] [--spec link-spec.toml]
quipu align decide  <set.sssom.tsv>     [--threshold 0.95] [--json]
quipu align apply   <set.sssom.tsv>     [--graph <alignment-iri>]
quipu align export  <alignment-iri>     [--format sssom-tsv|sssom-json|sssom-rdf]
quipu align verify  <alignment-iri>
```

`align` slots into `src/main.rs` dispatch beside `share` / `import` / `merge`,
in a new `src/cli_align.rs`. MCP gets `quipu_align_propose` / `_decide` /
`_apply`; REST gets `POST /align/propose|decide|apply`. Same three verbs at
every surface — no surface may do a thing the others cannot.

### propose

Emits candidate pairs as a mapping set in which **every mapping is unreviewed**:
`predicate_id` is the proposed predicate, `mapping_justification` is the
matcher's semapv term, `confidence` is the score, and `author_id` is absent —
absence of an author is what marks a row as undecided. It writes nothing to the
store.

Candidate generation reuses `resolve_entity` rather than growing a second
matcher. Two things it must add:

- **Per-type link specifications**, in the manner of Silk and LIMES: a
  declarative rule per `rdf:type` rather than one global fuzzy-name threshold.
  `Repository: same url OR (jaro_winkler(name) > 0.9 AND same owner)` is a rule
  a human can read, argue with, and version. A single global threshold is not.
  Ship a default spec, allow `--spec` to override, and keep it data.
- **Several weak signals combined into one calibrated score**, in the manner of
  Fellegi-Sunter — name, type, shared literal keys (`url`, `sha`, `path`),
  embedding distance. An ad hoc weighted sum is the thing to avoid; the
  combination rule belongs in the link spec, and the per-signal evidence must
  survive into the mapping so a reviewer can see *why*, not just *how much*.

`propose` must be **deterministic** for a given (graph-a, graph-b, spec, store
revision). A candidate list that reshuffles between runs cannot be reviewed in
a diff.

### decide

The operator loop, shaped after OpenRefine reconciliation and Wikidata
Mix'n'match: a table of pairs with their evidence, `y` / `n` / `skip`, and a
`--threshold` for bulk-accepting the confident band. Take LogMap's discipline of
**asking only about the uncertain band** — pairs above the auto-accept
threshold and below the auto-reject floor are the only ones worth a human's
attention, and the two cut-offs belong in the link spec next to the scoring that
produced them.

Every decision writes back into the same mapping set:

- accepted → `predicate_id: owl:sameAs` (or `skos:exactMatch` / `skos:closeMatch`
  where the operator says the concepts are near, not identical),
  `mapping_justification: semapv:ManualMappingCuration`, `author_id`,
  `mapping_date`.
- rejected → the same, plus `predicate_modifier: Not`.
- skipped → left unauthored, and re-proposed next time. Skip is not rejection,
  and conflating them loses the operator's most durable output.

`decide` is not required to be interactive. Editing the TSV by hand, or
generating it from a script, must produce the same result — that is most of why
the artifact is text.

### apply

Writes the mapping set into the alignment graph and derives the `owl:sameAs`
knots from the authored, non-negated rows. It must be **idempotent and
byte-identical on re-apply**: applying the same set twice writes nothing the
second time and changes no count. This is the `aegis-ezsx0` discipline and it is
an acceptance criterion below, not an aspiration — a second `apply` that appends
a second anything is the same defect class as `aegis-x1175`.

Re-applying after either side re-imports is the case that matters. The alignment
survives because it is keyed on IRIs, held outside both graphs, and derived
rather than hand-written.

## Migrating the existing auto-merge

`resolve_and_rewrite` must stop rewriting IRIs. The replacement:

1. `/import` emits a **proposed mapping set** for the incoming graph instead of
   an `ImportResolution` that only lives in a response body. Exact
   canonical-name matches are proposed at confidence 1.0 with
   `semapv:LexicalMatching`, not applied.
2. The staged graph keeps its foreign IRIs. Nothing about it is rewritten, which
   is also what makes it re-verifiable against its share hash.
3. `promote` gains an alignment step, or refuses to promote a graph whose
   proposed set has undecided rows above the uncertain-band floor — malcolm's
   call, and it should be stated in the docs page either way.

This is a behavioural change to a shipped path and it is the one part of this
work that is not purely additive. It needs its own commit, its own test showing
the foreign IRI survives import, and a line in the release notes.

An `--auto-exact` escape hatch preserving today's behaviour is acceptable if
malcolm finds a caller that depends on it; it must be off by default, because
the current default is the thing the directive is asking us to fix.

## Precondition: alignment is the sole writer of `quipu:distinctFrom`

Added during implementation (malcolm, with wu, 2026-09-05). It is a precondition
rather than a note because the property it protects cannot be recovered once
lost.

A rejection splits in two, and only one half asserts anything:

| operator outcome | recorded as | derives |
| --- | --- | --- |
| accept | `author_id`, `owl:sameAs` | `owl:sameAs` |
| assert different | `author_id`, `predicate_modifier: Not` | `quipu:distinctFrom` |
| decline — *not enough evidence* | `quipu_review: declined`, **no `author_id`** | nothing |
| skip | neither | re-proposed |

The split exists because `quipu:distinctFrom` is a **positive assertion of
non-identity**, and most rejections in a review loop mean "not enough evidence",
not "definitely different". Deriving one from a bare reject converts absence of
evidence into an assertion — the mirror of the `skos:closeMatch` error above.
The consequence is asymmetric in the dangerous direction: a wrong `owl:sameAs`
merges two entities and looks wrong to the next reader, while a wrong
`distinctFrom` **suppresses the pair everywhere, forever, and invisibly by
construction**, because the system's response to it is to stop proposing the
candidate. A declined row therefore carries no `author_id`, so an SSSOM consumer
that has never heard of `quipu_review` reads it as an unauthored proposal —
which is exactly true, since nothing was asserted.

Measured 2026-09-05, behind a passing control: `https://quipu.dev/ontology/distinctFrom`
holds **zero** assertions. (The 2230 `aegis:distinctFrom` in the fleet graph are a
deliberately separate predicate, read by the graph-extract skill's own
adjudication gate — `skills/graph-extract/SKILL.md` says so explicitly.)

**That zero is what makes `align verify` total.** While alignment is the only
writer of the quipu predicate, "every `distinctFrom` traces to an asserted
mapping" admits no exceptions — there is no foreign corpus for a bad assertion
to hide in. The moment an import, or any other feature, writes that predicate,
the check degrades from total to partial **and cannot be restored**: nothing
afterwards can tell which untraceable assertions were alignment's.

So:

- `verify` **FAILS** on an untraceable assertion. It does not warn. A warning
  about an invisible-by-construction suppression is read past once and then
  forever.
- `verify`'s failure message states this precondition, so an operator hitting it
  is told the difference between "fix the bug in `apply`" and "this check has
  just permanently weakened".
- **Traced is not correct.** `verify` proves PROVENANCE. A traced assertion is
  one somebody took responsibility for; whether it is *true* is a question this
  check does not ask, and its passing output says so — a green check whose
  meaning is overread is how a review step becomes a rubber stamp.

If another feature does need that predicate, that is a decision to take
deliberately and to record here, not to discover from a `verify` run that has
quietly stopped meaning what it says.

## Not in scope

- **Complex correspondences.** 1:1 `owl:sameAs` first. EDOAL and the Alignment
  API cover n:m and transformed correspondences; reach for them only when 1:1 is
  shown to be too weak, and say so on a bead when it is.
- **Predicate and class alignment.** PARIS aligns `runs_on` with `hosted_by` as
  well as the nodes, and two homelab graphs will disagree about predicate names
  as much as node names. Real, and a separate piece of work.
- **Importing a matcher as a dependency.** Silk, LIMES, LogMap and Splink are
  JVM or Python. We take the *shapes* — declarative link specs, calibrated
  multi-signal scoring, uncertain-band-only review — and none of the code.
  quipu already has the matcher primitives.
- **Automatic merging.** Not now, not behind a flag. The directive says
  *operator*.

## Acceptance

Each of these is a test, not a claim.

1. `propose` on two stores that hold `bobbin-release` and
   `Bobbin_release-artifact` emits a mapping set containing that pair, with the
   matcher's justification and a confidence, and writes nothing to either store.
2. `propose` run twice over an unchanged pair of graphs produces
   byte-identical output.
3. A rejected pair is absent from the next `propose` over the same graphs, and
   the reason is readable in the set as `predicate_modifier: Not`.
4. `apply` writes `owl:sameAs` into the alignment graph and into neither source
   graph — asserted by querying both sources for `owl:sameAs` and getting zero.
5. `apply` twice over the same set: second run reports zero writes, and the
   triple count and every `rdfs:comment` count are unchanged.
6. The alignment graph shares and re-imports as an ordinary quipu share, and
   `apply` of the re-imported set on a third store reproduces the same knots.
7. A SPARQL query that traverses `owl:sameAs` returns facts from both source
   graphs for one concept — the demo step appended to the `iv3df7.3` transcript.
8. `export --format sssom-tsv` output validates under `sssom-py`.
9. `/import` leaves foreign IRIs intact (the migration above), with the previous
   behaviour reachable only behind an explicit opt-in.

## Docs

One page under Sharing & Federation in `docs/book/src/sharing/`, true at every
sentence: what an alignment is, why it lives outside both graphs, the three
commands, and the round trip. Plus the `iv3df7.3` transcript step — import,
align, query across both — because the story is the deliverable and a primitive
nobody can see demonstrated is not shipped.

## Open questions for the implementer — ANSWERED (malcolm, 2026-09-05)

- **`promote` warns, and blocks only on request.** The `/import` migration below
  is already a behavioural change to a shipped path; making `promote` newly
  refuse in the same release means a broken caller cannot be attributed to
  either change. Revisit once the primitive has usage.
- **The alignment graph IRI is derived**, `urn:quipu:align:<a-hash>:<b-hash>`,
  with an `--as` override. Acceptance criteria 5 and 6 both need it to be a
  function of the inputs rather than of an operator typing the same string
  twice.
- **`skos:closeMatch` round-trips and derives nothing.** `predicate_id` stays a
  free string, so a close match written by hand or by `sssom-py` survives
  unchanged, and `derives_knot` is false for it. Near is not identical;
  deriving `owl:sameAs` from a close match would launder a similarity into a
  fact.

The original questions, for the record:

- Does `promote` block on undecided high-band candidates, or warn? Blocking is
  safer and more annoying; state the choice in the docs page.
- Should the alignment graph IRI be derived from the two graph hashes (stable,
  opaque) or operator-named (readable, collision-prone)? Derived is
  recommended, with an `--as` override.
- `skos:closeMatch` implies the concepts are near but not identical, so it must
  **not** derive an `owl:sameAs`. Is a close match worth supporting in v1, or
  is `sameAs`-or-nothing the honest first cut?

## Prior art

Recorded in full on `aegis-sosiaa` (sattler, 2026-09-04). In brief: SSSOM for the
record; Silk and LIMES for declarative link specifications; PARIS for joint
instance/relation/class alignment; LogMap for logic-based repair and
uncertain-band interaction; Splink and Dedupe for Fellegi-Sunter scoring;
OpenRefine reconciliation and Wikidata Mix'n'match for the operator loop; OAEI
if we ever want a benchmark number.

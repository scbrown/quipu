# Semantic, entity-grounded edit policies

> **Implementation status (2026-08-25):** 🟨 **Partial — quipu's half of
> sequencing steps 1, 3 and the vocabulary of 4–5 is BUILT; the evaluation
> half (yupana) is not.** The regex-tier catalog this evolves
> (`shapes/policies/treesitter.ttl`, projected and enforced by yupana's
> pre-edit hook) is live, and so is the vocabulary that lets a grounded or
> inexact-tier policy be *stated and refused* — but nothing yet *evaluates* a
> grounded predicate, so no edit is decided by membership today.
>
> **BUILT (quipu):**
>
> - **§Sequencing step 1 — the grounded vocabulary and its definition-time
>   rules.** `aegis:candidateSource` and `aegis:groundingQuery` are declared
>   properties (`shapes/aegis-properties.ttl`); `"must-ground"` and
>   `"must-not-ground"` are admitted `aegis:matchType` values alongside the
>   lexical trio (`shapes/governance.ttl`, the `sh:in` on `aegis:matchType`).
>   The cross-field rule SHACL core cannot express — a grounded match type
>   without an `aegis:groundingQuery` is refused at definition time — lives in
>   `src/governance/placement.rs`, with the shape-level admission tested in
>   `src/governance_tests.rs` (`a_grounded_predicate_conforms`,
>   `an_inexact_tier_predicate_conforms`, `predicate_tier_out_of_enum_is_rejected`)
>   and the cross-field refusals in `src/governance/placement_tests.rs`.
> - **§Sequencing step 3 — grounded catalog entries.**
>   `aegis:pred_no_ticket_in_comment_v2` ships the token/id-set/`must-not-ground`
>   form from Design A verbatim (`shapes/policies/treesitter.ttl`), and
>   `aegis:pred_implements_grounded` ships the embedding-tier, `citation`-candidate
>   form from Design B with its `aegis:OperatingPoint`
>   (`shapes/policies/linkage.ttl`). Both are validated as *shipped data*, not
>   fixtures, by `linkage_policy_catalog_conforms` and its tree-sitter twin.
> - **The honesty rules for the inexact tiers (Design B/C vocabulary).**
>   `"embedding"` and `"model"` are admitted `aegis:tier` values, and a policy
>   composing either is refused at definition time when it hard-denies at the
>   PAG, declares no `aegis:OperatingPoint`, declares no
>   `aegis:falsePositiveTolerance`, or claims a tolerance of `0.0`
>   (`src/governance/placement.rs`). A classifier cannot assert exactness here.
>
> **NOT built — the true remainder:**
>
> - **§Sequencing step 2, entirely (yupana).** The grounding-set projection
>   into the hot plane, the three-outcome evaluation, and the `unresolvable`
>   violation class. This is the load-bearing gap: `aegis:groundingQuery`'s own
>   contract says a missing or failed projection must render the rule
>   UNEVALUATED and loud rather than "empty set, nothing grounds, allow" — and
>   the component that would honour that rule does not exist yet. Until it
>   does, a grounded predicate is a well-formed statement nothing acts on.
> - **§Sequencing step 4's evaluation half.** The work-item vector matrix
>   projected from bobbin's beads index, and score/threshold/model/corpus-
>   watermark carried in the verdict. Only quipu's vocabulary and calibration
>   refusals landed.
> - **§Sequencing step 5's evaluation half (Design C).** Same split: the
>   `"model"` tier value and its placement rules exist; the generative
>   judgment, its quarantined fact output, and the routing behind it do not.

## Problem

The shipped `no-ticket-in-comment` / `todo-needs-ticket` pair decides
"is this a ticket reference?" with a regex (`\b[A-Z]+-[0-9]+\b`). That is
the wrong authority twice over:

1. **Over-inclusive.** `ABC-123` in a comment about an RFC, a part number,
   or a test fixture matches the pattern and is denied, though no such
   ticket exists anywhere.
2. **Under-inclusive.** "fixes the frontier-recompute bead" references
   tracked work as surely as `bobbin-bnq` does, and the regex is blind to
   it.

Meanwhile the store *already knows what a ticket is*. Work items are
entities in the graph — beads exports, camayoc `aegis:WorkItem` records,
dispatch inventory — each with an id, a status, and bitemporal history.
Ticket-ness is a membership question against governed data, not a lexical
shape.

## Design A — entity-grounded predicates (deterministic, no model)

Extend the `aegis:Predicate` vocabulary:

```turtle
aegis:pred_no_ticket_in_comment_v2 a aegis:Predicate ;
    aegis:name "no-ticket-in-comment" ;
    # No regex. Candidates are simply the TOKENS of the introduced text —
    # yupana already parsed the comment; tokenization needs no shape
    # assumption. (An optional aegis:candidateSource pattern remains
    # available purely as a narrowing optimization, never as authority.)
    aegis:candidateSource "token" ;
    # Truth is membership: a SPARQL SELECT naming the authoritative set.
    aegis:groundingQuery "SELECT ?id WHERE { ?w a aegis:WorkItem ; aegis:identifier ?id }" ;
    aegis:matchType "must-not-ground" ;
    aegis:tier "tree-sitter+graph" .
```

The id set itself defines what a ticket looks like: a token grounds if
and only if the graph holds it. A hash-set membership test per token is
O(tokens of introduced text) against a projected set, well inside the
5 ms budget — the regex is not merely demoted, it is unnecessary.

Evaluation of a candidate token then has **three outcomes, not two**:

| Candidate | Outcome | `no-ticket-in-comment` (deny) | `todo-needs-ticket` (warn) |
|---|---|---|---|
| resolves to a real work item | **grounded** | violation — real ticket named in a comment | satisfied |
| matches the shape, resolves to nothing | **unresolvable** | distinct violation class: a *fabricated reference* | NOT satisfied — a hallucinated ticket cannot satisfy the rule |
| no candidates at all | — | satisfied | violation (TODO with no reference) |

The `unresolvable` outcome is the grounding-integrity payoff: an agent
that invents `QUIP-999` to satisfy `todo-needs-ticket` is caught by the
graph, not by a reviewer. It is reported as its own violation class,
never folded into either neighbor (the typed-non-answer discipline).

**Fast-plane mechanics.** Yupana cannot run SPARQL per keystroke. The
grounding query runs at *projection* time: yupana projects the id set
(plain set, or a hash/bloom structure at scale) into the hot plane
alongside the rules, under the existing machinery — same freshness
declaration, same durable cache with age-in-verdict, same
supertype-targeted projection. Membership at evaluation time is O(1),
inside the 5 ms budget the catalog already declares. A missing or failed
grounding projection makes the rule **unevaluated (loud)** — never
"empty set, nothing grounds, allow."

**Bitemporal sharpening.** Because work items are bitemporal,
`todo-needs-ticket` can require the referenced item to be *open at edit
time* — computed at read time from the graph, never stored (the
liveness-by-absence discipline). A TODO citing a closed ticket is its own
advisory outcome.

## Design B — similarity-tier predicates (vector grounding, falsifiable)

A comment can reference tracked work with no id at all ("fixes the
frontier-recompute problem"). Before reaching for a generative model,
there is a deterministic middle tier: **embedding similarity against
the work-item corpus**.

- **The corpus exists.** Bobbin already indexes beads
  (`bobbin src/index/beads.rs`) — id, title, description, embedded.
  The grounding matrix is a projection of that index (or an equivalent
  embedding pass over the graph's work items): a few thousand vectors,
  brute-force cosine in microseconds, no ANN infrastructure needed.
- **Deterministic and falsifiable.** Given a pinned embedding model and
  corpus snapshot, the score is reproducible. The verdict records the
  matched item, the score, and the threshold —
  `cosine(comment, bobbin-bnq) = 0.83 ≥ 0.75` — which is a falsifier in
  the catalog's own style: re-embed and recompute to disprove.
- **Still a classifier, honestly.** A threshold trades FP against FN,
  so the predicate carries `aegis:tier "embedding"` (a distinct value:
  reproducible-but-approximate, unlike both `tree-sitter+graph` exact
  membership and generative `model` judgment), a **nonzero**
  `OperatingPoint`, and placement at PAA or the escalation router —
  never hard PAG denial. The embedding-model identity and corpus
  snapshot watermark ride the verdict, since a score means nothing
  outside the model and corpus that produced it (the trust-chain rule,
  applied to embeddings).
- **Freshness as ever.** The vector matrix projects under the same
  freshness/cache-age machinery; a stale matrix is declared in the
  verdict, and a missing one renders the rule unevaluated, never
  satisfied.

The predicate ladder is then three closed tiers, weakest authority to
strongest claim: exact membership (`tree-sitter+graph`, hard-capable) →
similarity (`embedding`, advisory/escalate, score-falsifiable) →
generative judgment (`model`, advisory only, quarantined output).

## Design C — model-tier predicates (generative judgment, last resort)

Where similarity is insufficient — judgments requiring reasoning over
the sentence, not nearness to a corpus — a generative model decides,
and the stack's own rules for model judgments apply:

1. **A new tier value, closed as ever.** The predicate carries
   `aegis:tier "model"` — a fifth value alongside
   treesitter/lsp/cpg/engine-state. A semantic verdict can never
   masquerade as an exact one; the tier rides the verdict into the graph.
2. **An honest OperatingPoint.** The catalog's own comments state the
   rule: an exact predicate carries 0.0/0.0 tolerances; *an inexact hard
   predicate would carry a tolerant FP number and a zero FN one, never
   the reverse*. A semantic predicate declares nonzero tolerances or is
   refused at definition time.
3. **Placement follows fallibility.** A classifier-backed predicate does
   not hard-deny at the pre-action gate: the placement matrix routes it
   soft at the post-action auditor, or through the escalation router
   (`effect "escalate"`) when a human should rule. The latency budget
   agrees — a model call does not fit a 5 ms hook; the PAA and the
   router are asynchronous seams.
4. **Its facts are quarantined.** If the semantic judgment is recorded
   as a fact ("this comment references bobbin-bnq"), it lands
   `sourceKind "inferred"` in the low-trust plane, promotable by
   authority, never self-asserted as observed — the camayoc ingress
   discipline unchanged.

## What this reuses (nothing new to invent)

| Need | Existing mechanism |
|---|---|
| Policy definition, validation, placement rules | `shapes/governance.ttl`, definition-time placement check |
| Hot projection + freshness + cache age | yupana `project.rs`, `projection_cache.rs` |
| Loud non-answers (unevaluated, unresolvable) | yupana typed-outcome discipline |
| FP/FN honesty for inexact predicates | `aegis:OperatingPoint` |
| Human ruling on semantic denials | escalation router (provisional A §3) |
| Embedded work-item corpus | bobbin beads index (`src/index/beads.rs`) |
| Score meaningless outside its model+corpus | trust-chain rule (provisional A §4), applied to embeddings |
| Quarantine of model-written facts | camayoc `sourceKind` + label lattice |
| Read-time open-ness of a cited ticket | bitemporal store, liveness-by-absence |

## Further applications of similarity-as-grounding

The pattern — resolve text to governed entities by similarity, seal the
score in a falsifiable verdict, never let similarity hard-deny — applies
wherever an agent cites, claims, or should-have-cited an entity. The
rule everywhere: similarity **grounds or advises**; exact membership
stays the only hard tier; every verdict carries
score/threshold/model/corpus-watermark; anything written lands
`inferred` and quarantined.

**Ordering principle: identify and inform before refusing.** Yupana's
edit seam answers *"what is this?"* first — fast entity identification
plus surfaced context (the governed entity being touched, its attached
policies, nearest precedents, nearby lessons and denials) — before the
slower, heavier evaluation that may refuse under quipu policy. Stage 1
is advisory, always-on, and hot-plane cheap (membership lookups and
brute-force cosine, microseconds to low milliseconds); stage 2 is the
policy verdict, which may consult projections, run heavier analysis, or
route to escalation. The ordering is load-bearing twice: an agent that
sees the context self-corrects before tripping policy, making refusals
rarer; and when a refusal does land, the stage-1 context is already in
the loop, so the refusal arrives explained rather than bare. All six
applications below are stage-1 citizens except where a grounded exact
match feeds a hard stage-2 rule.

1. **Claimed-linkage verification** (grounding-integrity, strongest).
   A commit or bead-close *claims* `aegis:implements` — check the
   diff/commit content's similarity against the cited item's
   description. Grounded / **cited-but-dissimilar** (a fabricated
   linkage — the provenance edge checkable rather than trusted) /
   no-citation. The unverified claim is exactly how a wrong provenance
   edge poisons replay-derived rules.
2. **Settled-decision collision.** Before a new `Decision` episode
   lands (camayoc), similarity against `crew:declared` standing
   decisions — "this duplicates or conflicts with a settled human
   decision," advisory or escalate, *before* the write. Convention
   memory made enforceable: re-litigation is noticed by the store.
3. **Duplicate work-item detection at `bd create`** — the beads corpus
   is already embedded (bobbin); a new issue near an open one gets an
   advisory naming the near-duplicate.
4. **Precedent for the escalation router** — a minted DecisionRequest
   carries its nearest prior *decided* requests, so the human sees
   precedent, with the similarity score on record.
5. **Denied-edit recurrence** — the verdict spool as corpus; an edit
   near a previously denied one surfaces the prior verdict as
   advisory, teaching the agent from refusals it never saw.
6. **Competency-gap detection** (camayoc) — a question resolving to no
   competency question above threshold is reported as an ontology
   gap ("no coverage"), never silently answered.

## Sequencing

1. Vocabulary: `aegis:candidateSource`, `aegis:groundingQuery`,
   `must-ground`/`must-not-ground` match types; shapes + placement rules
   for them. (quipu)
2. Grounding-set projection into the hot plane; three-outcome
   evaluation; `unresolvable` violation class. (yupana)
3. `todo-needs-ticket` v2 requiring an open, existing item. (catalog)
4. Embedding tier: work-item vector matrix projected from bobbin's
   beads index (or an equivalent embedding pass), `tier "embedding"`,
   score/threshold/model/corpus-watermark in the verdict, PAA or
   escalate placement. (bobbin + yupana + quipu vocabulary)
5. Model tier: vocabulary value, OperatingPoint requirement,
   PAA/escalate placement, quarantined fact output. (quipu + yupana,
   after 1–4 prove the grounding loop)

Related: `shapes/policies/treesitter.ttl` (the v1 pair this supersedes),
`docs/design/policy-edit-hooks.md` (the hook seam), camayoc
`docs/design/ingress.md` (the quarantine the semantic tier lands in),
camayoc `docs/patents/provisional-grounding-cluster.md` § 9 (the
edit-boundary enforcement disclosure this embodiment extends).

# Semantic, entity-grounded edit policies

> **Implementation status (2026-08-15):** 🟥 **Design only.** Nothing below is
> implemented. The regex-tier catalog this evolves
> (`shapes/policies/treesitter.ttl`, projected and enforced by yupana's
> pre-edit hook) is live; this document specifies its successor tiers.

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
    # The regex DEMOTES to candidate extractor: it proposes tokens,
    # it no longer decides truth.
    aegis:candidateSource "\\b[a-zA-Z]+-[a-z0-9]+\\b" ;
    # Truth is membership: a SPARQL SELECT naming the authoritative set.
    aegis:groundingQuery "SELECT ?id WHERE { ?w a aegis:WorkItem ; aegis:identifier ?id }" ;
    aegis:matchType "must-not-ground" ;
    aegis:tier "tree-sitter+graph" .
```

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

## Design B — semantic-tier predicates (model judgment, honestly tiered)

A comment can reference tracked work with no id-shaped token at all.
Deciding that is a model judgment, and the stack's own rules for model
judgments apply:

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
| Quarantine of model-written facts | camayoc `sourceKind` + label lattice |
| Read-time open-ness of a cited ticket | bitemporal store, liveness-by-absence |

## Sequencing

1. Vocabulary: `aegis:candidateSource`, `aegis:groundingQuery`,
   `must-ground`/`must-not-ground` match types; shapes + placement rules
   for them. (quipu)
2. Grounding-set projection into the hot plane; three-outcome
   evaluation; `unresolvable` violation class. (yupana)
3. `todo-needs-ticket` v2 requiring an open, existing item. (catalog)
4. Semantic tier: vocabulary value, OperatingPoint requirement,
   PAA/escalate placement, quarantined fact output. (quipu + yupana,
   after 1–3 prove the grounding loop)

Related: `shapes/policies/treesitter.ttl` (the v1 pair this supersedes),
`docs/design/policy-edit-hooks.md` (the hook seam), camayoc
`docs/design/ingress.md` (the quarantine the semantic tier lands in),
camayoc `docs/patents/provisional-grounding-cluster.md` § 9 (the
edit-boundary enforcement disclosure this embodiment extends).

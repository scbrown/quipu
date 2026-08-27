# Flagship Reasoning Use Cases — Plan

> Created: 2026-08-27
> Status: PLANNED — no implementation yet; ordering and scope recorded
> Related: [semantic-reasoning-gaps.md](./semantic-reasoning-gaps.md) (U1–U6),
> [entailment-regime.md](./entailment-regime.md),
> [reasoning-engine-fixes.md](./reasoning-engine-fixes.md)

## One-Line

Two consumer-visible wins driven end-to-end — subclass entailment retiring the
stack's hand-rolled workarounds, and the provenance cone making trajectory
pruning mechanical — plus one cheap live demonstration to de-risk both.

## Why these two

A reasoning capability that no consumer reads is shelfware. These flagships
were chosen because the consumers are already paying for their absence:
subclass entailment has a production incident behind it (aegis-368cu.4) and
three recorded workarounds; the provenance cone is the mechanism camayoc's
golden-paths design explicitly delegates to an unbuilt quipu command.

## Flagship 1 — Subclass entailment for catalogue queries

**Goal:** `?s a <Class>` against the base+inferred dataset matches instances
of subclasses, so consumers stop hand-rolling `a/rdfs:subClassOf*` and stop
dual-typing.

### End-to-end slices

1. **Ontology** (camayoc-governed): mint the minimal `rdfs:subClassOf` edges a
   competency question owns — no speculative hierarchy. Current candidates
   from camayoc's own analysis: `aegis:adversariallyProvenBy` ⊑
   `aegis:verifiedBy` (verification Q4 distinguishes exactly this), and a
   `hasVerification` superproperty over the three sibling
   "points at a falsifier-gated Verification" edges. Note these are
   *subproperty* cases; the class-side candidates live in bobbin/yupana's
   vocabulary (`TextRule` subtypes, `Chunk`/`CodeSymbol`/`Section`). Each edge
   arrives with its question, per the house rule.
2. **Store** (quipu): subclass/subproperty closure via the existing OWL
   materializer, placed per
   [entailment-regime.md](./entailment-regime.md) — companion inferred graph,
   `sourceKind "inferred"`, freshness note. Requires engine-fixes Phase 2
   (liveness) for the closure to stay current as members arrive.
3. **Consumers**:
   - yupana simplifies `POLICY_QUERY`/`TEXT_POLICY_QUERY`
     (`src/project_queries.rs`) to plain `a aegis:TextRule` against the
     base+inferred dataset — keeping the explicit-path form as a documented
     fallback, because the load-bearing comment's lesson (silent empty
     catalogue) must not be re-learnable;
   - bobbin retires dual-typing in `src/knowledge/chunks.rs` (assert the
     specific type, read the supertype from closure);
   - yupana's five `# TIGHTEN LATER: sh:class` constraints in
     `shapes/code-edges.ttl` are enabled, validating against the union;
   - `bobbin-567` type-level policy targeting (`aegis:targets "CodeModule"`)
     gains its type→instance binding.

### Acceptance

- The aegis-368cu.4 shape is untriggerable: a concrete-class catalogue query
  against the base+inferred dataset returns the full catalogue, and a test at
  the consumer pins it.
- Dual-typing removal is diff-visible in bobbin's emitted Turtle and the
  chunk-graph snapshot round-trips unchanged semantically.
- No consumer regresses when the inferred dataset is absent — every consumer
  keeps a correct (if wider) explicit-path fallback.

## Flagship 2 — The provenance cone (`quipu path cone`)

**Goal:** answer camayoc golden-paths Q5 — *which steps of a trajectory are
inside the provenance cone of its verified result* — as a quipu capability,
making pruning mechanical instead of human.

### Shape of the computation

The transitive derivation closure from a trajectory's terminal Verifications
backward through step outputs (camayoc `docs/design/golden-paths.md:129-135`).
A step outside the cone contributed nothing the verified result depends on:
mechanically prunable.

### Design decisions to settle first (in the design slice of this work)

1. **Engine**: three candidates —
   - a Datalog ruleset (transitive `derivesFrom` closure; needs engine-fixes
     Phase 1/5 depending on rule arity),
   - a stored SPARQL query with property paths (works today, but the cone is
     per-trajectory and join-heavy),
   - a dedicated graph command like `impact`/PageRank (petgraph over the fact
     log, `src/graph/` precedent).
   Lean **dedicated command** (`quipu path cone <trajectory>` + MCP + REST):
   the cone is a bounded reverse-reachability computation with a seed set,
   exactly the `impact` shape, and it keeps rule-engine limits off the
   critical path.
2. **Placement of cone membership**: camayoc deliberately left the per-step
   membership term unminted (*"minting the term first would be modelling a
   mechanism's internals before the mechanism"*). Follow the regime: computed
   at read time by default; if materialized for pruning workflows, it lands
   quarantined with `sourceKind "inferred"` and is promoted only when a
   pruning authority accepts it. The term gets minted (camayoc-side, with its
   competency question) only when the mechanism exists.
3. **Edge vocabulary**: the cone walks step-output/derivation edges that
   camayoc's trajectory slice defines; confirm the closed set of predicates
   with the camayoc fixtures (`tests/fixtures/golden-paths.ttl`) before
   implementation, so the cone's meaning is the fixture's meaning.

### Acceptance

- Against the golden-paths fixture, the cone of the verified result matches
  the hand-derivable answer, and a step edit that severs a derivation edge
  moves that step out of the cone.
- Camayoc's Q5 flips from GAP to answerable in its coverage tables, citing
  the quipu command.
- Bead quipu-gp2's design-only status is superseded by the implementation.

## The cheap live demonstration (do first)

**Section-containment ancestry** (camayoc doc-structure Q7/Q8): UNWRITTEN,
not GAP — the data is live (three repos ingested hourly via bobbin) and the
vocabulary exists (`bobbin:contains` over Sections). Ship two stored queries
using `contains+` / `^contains` (parent/children, depth-N ancestry) — pure
property paths, no engine work, no new terms. This validates the explicit-path
route against real data, gives the flagships a baseline to beat, and closes
two open competency questions for the cost of two stored queries.

Do the same for **supersession chains** (`supersededBy+` to find the current
end) and **transitive blocked-on** — one stored query each, camayoc-side.

## Ordering

1. Live demonstration (stored queries, camayoc) — days, no store changes.
2. Flagship 1 (subclass entailment) — after engine-fixes Phases 1–2 and the
   regime's inferred-plane mechanics land.
3. Flagship 2 (provenance cone) — design slice can start immediately
   (decisions 1–3 above); implementation follows the `impact`-command
   precedent and does not block on Flagship 1.

## Suggested bead breakdown

- One camayoc-side bead for the three stored-query demonstrations.
- Flagship 1: one bead per slice (ontology edges, closure placement, each
  consumer change).
- Flagship 2: a design bead (settle decisions 1–3), then an implementation
  bead for the command and its surfaces.

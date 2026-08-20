# Golden-path blessing: from a verified trajectory to a governed path

> **Implementation status (2026-08-20, quipu-gp2/gp3/gp4):** 🟨 **The
> analysis pipeline is built; promotion still rides the existing gates.**
> `src/path/` implements the cone (`quipu path cone`, `quipu_path_cone` MCP,
> `POST /path/cone`), the trajectory backtest (`quipu path backtest`,
> `quipu_path_backtest`, `POST /path/backtest`) under the versioned grammar
> (`src/path/grammar.rs`, [conformance-grammar.md](./conformance-grammar.md)),
> and the Turtle-emitting draft (`quipu path draft`). One design correction
> found while building is recorded inline in §3: the cone is forward
> reachability, not speculative removal. Trajectory ingestion needed no code
> (§1 — it is the episode path with camayoc's vocabulary), and promotion
> deliberately adds none (§5). This is the quipu-owned mechanism half of the
> golden-paths design; the ontology and its competency suite live in camayoc
> (`docs/design/golden-paths.md`, `competency/golden-paths.md` there), and the
> enforcement half lives in yupana (`docs/golden-path-guard.md`), still
> design-only.
>
> Created: 2026-08-20
> Status: DESIGN
> Related: [policy-by-example.md](./policy-by-example.md) (the point-exemplar
> ancestor of this pipeline), [reasoner.md](./reasoner.md) (speculative
> transactions this design reuses).

## One-Line

Store the trajectories completed work items actually took; deterministically
establish which steps the verified result depended on; backtest a pruned
candidate against recorded history; and promote it through the *existing*
advisory→enforcing gates — policy-by-example generalized from one observed
edit to one observed success.

## The pipeline, and what each stage reuses

A golden path is born the same way an exemplar policy is born, one level up:

1. **Record** — trajectories arrive as episodes.
2. **Admit** — deterministic gate: outcome `done` + falsifier-gated
   verification of the result.
3. **Prune** — provenance-cone analysis marks what the result depended on;
   humans rule on the rest.
4. **Backtest** — replay the candidate over bitemporal history before it
   exists as a rule.
5. **Promote** — born advisory, promoted by evidence, by a human, through the
   gates that already exist.

### 1. Trajectory ingestion — the `/episode` path, unchanged

A trajectory is an ordered set of steps hanging off a work item: actor,
action, target, the Decision enacted, the Verification produced. This is
exactly what the episode path already ingests — typed nodes and edges with
automatic PROV provenance — so recording trajectories is a *vocabulary*
addition (camayoc's slice), not a store capability. Trajectories land in the
observed plane and are never edited; pruning writes new entities.

What is new: step **order** and step **input/output artifacts** must be
carried explicitly, because the provenance cone (below) is computed over
them. An episode that records steps without their derivation edges produces a
trajectory that can be replayed (competency Q1) but not pruned mechanically
(Q5) — admissible, but everything inside it needs a human ruling. The
pipeline degrades toward more human involvement, never toward guessing.

### 2. Admissibility — a query, not a mechanism

"Completed with verified results" is decidable from facts the store already
holds: `aegis:outcome "done"` and terminal `aegis:Verification` records
carrying `aegis:falsifier`. Admissibility is a named stored query (the
quipu #79 pattern), not new machinery — competency Q3 *is* the
implementation. A success whose checks cannot name their failing result never
enters the pipeline.

### 3. Provenance-cone pruning — `speculate` pointed backwards

The deterministic pruning aid: compute the transitive derivation closure from
the terminal Verifications back through step outputs. Steps outside the cone
contributed nothing the verified result depends on — mechanically prunable.
Steps inside are load-bearing; omitting one requires a recorded human
Decision with rationale.

Reuse: the impact machinery already answers "what depends on X" —
`Store::speculate` (`src/store/ops.rs`) and `speculate_remove`
(`src/impact.rs`) evaluate a hypothetical removal without committing it. The
original sketch here was: *speculatively remove step S; is the terminal
Verification's derivation chain still intact?*

> **As built (quipu-gp2), a correction:** the speculative-removal test is
> unsound for this question. Retraction removes only S's OWN facts, so a
> chain that continues through edges owned by S's output artifacts survives
> S's removal — and a load-bearing step reads as prunable, which is the one
> direction this analysis must never fail in. What shipped is **forward
> reachability** on the same bounded BFS as `quipu impact`
> (`src/path/cone.rs`): S is in-cone iff something it produced flows, along
> the derivation predicates, into a falsifier-gated verification. A step
> with no recorded derivation edges is CANNOT-EVALUATE — never prunable by
> silence — and a trajectory with no falsifier-gated verification is refused
> outright.

`quipu path cone <trajectory>` (CLI + MCP + `POST /path/cone`) wraps this
per-trajectory. Its verdicts become stored facts at DRAFT time (§5): each
omission carries its authority (`cone-analysis` vs a Decision IRI), so
competency Q6 is answerable forever after.

### 4. Backtest — `governance/backtest.rs`, generalized

Policy-by-example backtests a candidate rule by replaying it over the store's
bitemporal history before the rule exists. The trajectory version replays a
candidate *path*: over the recorded past, which work items' trajectories
would have conformed, and how did they close? The report a human promotes on:

- N work items would have matched the path's applicability (similarity over
  work-item type and topic entities);
- of those, the conformers closed `done` at rate X, non-conformers at rate Y;
- the divergence points where non-conformers left the path.

Same discipline as `src/governance/backtest.rs`: "0 hits" is distinguished
from "cannot evaluate", and a backtest that cannot evaluate is reported as
itself, never as a clean result. Step matching starts coarse (action kind +
target type per step, in order, gaps allowed) — the exact conformance grammar
is deliberately the guard's contract, shared with yupana, and versioned so a
backtest and a live verdict can never silently disagree about what
"conforming" means.

### 5. Draft and promote — the existing gates, no parallel machinery

A candidate that survives backtest is drafted the way exemplar policies are
drafted (`src/governance/draft.rs`): born advisory, `effect "warn"`,
hard-coded — never enforcing on day one. The path cites its exemplar
trajectories via `aegis:exemplar` exactly as an exemplar policy cites its
motivating edit, so a warn (and later a deny) can always answer "because this
concrete work succeeded this way."

Promotion up the blessing ladder (L3 advisory → L4 blessed → far-horizon L5
constraint-backing) rides the existing advisory→enforcing promotion gates —
liveness, two-sidedness, new-blocks, recoverability, blast radius, measured
over real traffic — with conformance events as the traffic. Every promotion
is a human act stored as a fact of the event (who, when, citing which
backtest). Demotion is `aegis:supersededBy`, never deletion. L5 (a path
backing an `aegis:Policy` that *authorizes* action) additionally waits on
verdict signing, per the yupana addendum's Phase-4 caveat; the ladder
honestly stops at L4 until then.

## What quipu serves to whom

| Consumer | Surface | Content |
|---|---|---|
| agents (understanding a work item) | SPARQL / named queries | paths for similar work, their exemplars, dead-end hazards |
| yupana (enforcement) | the existing policy-projection path | blessed paths as compiled rules, with projection freshness |
| humans (promotion) | CLI (`quipu path cone`, `quipu path backtest`) | cone reports, backtest reports, the promotion queue |
| audit (competency Q14–15) | SPARQL | constraint ← path ← exemplar chains |

## Open questions

- **Step granularity.** Tool call? Bead state transition? Both, with
  trajectory-level declared granularity? The competency suite's Q1 fixes the
  answer shape but not the grain; the first fixture graph decides.
- **Similarity's plane.** "Work items like this one" is model-inferred at the
  margins; suggestions land in the inferred plane, tagged, and only the
  deterministic core (type + topic-entity match) feeds the backtest
  denominator.
- **Cone soundness under missing derivation edges.** The cone is only as
  complete as recorded step outputs. The rule above (no edges → no mechanical
  pruning) keeps missing data expensive rather than dangerous, but the
  incentive design — making derivation edges the cheap default at recording
  time — belongs to camayoc's ingress discipline.

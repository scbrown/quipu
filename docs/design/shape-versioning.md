# Design: Shape Versioning — a bitemporal registry for shapes and ontologies

> **Implementation status (2026-08-06):** ⬜ **Designed, not built.** The
> `shapes` and `ontologies` tables are latest-only today; nothing here is
> implemented.

**Status:** The store's *data* is bitemporal; the *rules that validate it* are
not. That asymmetry is currently invisible and it should not be: governance
policies (ordinary `aegis:Policy` facts) can be time-traveled, but the SHACL
that constrained them, and the shapes that gate every validated write, exist
only in their latest revision. "Which shapes were in force at time T" is
unanswerable, and two consumers already need the answer.

## 1. The current state is worse than "no versioning" — it is *discarded* versioning

- `shapes(name PRIMARY KEY, turtle, loaded_at)` and `load_shapes`'
  `INSERT OR REPLACE` obliterate the previous turtle with no trace; even
  `loaded_at` is overwritten. `ontologies` has the identical schema and the
  identical problem. `remove_shapes` is a hard `DELETE` — contrast with facts,
  where retraction is logical (`valid_to` close) and history survives.
- **No event is emitted on shape load.** `load_shapes` is a bare
  `conn.execute` — no `transact`, no tx id, nothing appended to `events`. The
  append-only audit spine has no record that the rules changed.
- Meanwhile **`proposals` already holds the lineage** for every shape that
  arrived via the proposal path: monotonic `id`, the full turtle in `diff`,
  `decided_by` / `decided_at` / `decision_note`. `accept_proposal` ends in
  `load_shapes`, which flattens that history to latest-only. And direct loads
  (MCP `quipu_shapes load`, `POST /shapes`, `seed-fixtures`) bypass proposals
  entirely, so the lineage is partial.
- **Replay evaluates against current Σ, by design** — `audit_spec::load` runs
  plain SPARQL with no `AsOf`, even though the SPARQL layer supports it end to
  end. That design intent is correct for drift-catching (a checker reading its
  own snapshot would agree with itself about a policy that has since been
  re-classed). But it makes replay's `would_block` silently wrong: a policy
  re-classed `hard`→`soft`, or retired, after the trace window falsifies every
  replay number for that window, undetectably — and the class-placement audit
  pass would report the re-classing as a *trace* violation, unable to
  distinguish "the runtime got it wrong" from "the spec moved."
- Verdicts deliberately do not hash graph state (no stable serialisation;
  hashing one that moved with unrelated facts would make every verdict
  spuriously stale — [policy-edit-hooks.md](policy-edit-hooks.md)). A
  *declared shape version* is exactly the stable reference that reasoning
  permits.

## 2. Design: version the registry, never touch the validator

### 2.1 Bitemporal `shapes` and `ontologies`

Both tables gain `valid_from`, `valid_to`, `tx` (additive migration; existing
rows get `valid_from = loaded_at`, open `valid_to`). Loading a name **closes
the prior row** instead of overwriting — the retraction-never-deletes
discipline `facts` already has. `remove_shapes` becomes a close, not a
`DELETE`. Loads emit a `shapes.loaded` event carrying the tx, closing the
audit-spine hole.

### 2.2 As-of reads

`get_combined_shapes` gains an `as_of: Option<AsOf>` twin; `POST /validate`
and the MCP tool gain optional `valid_at` / `as_of_tx` parameters defaulting
to now — zero behaviour change for every existing caller.

`Validator::validate` takes serialized Turtle and is version-agnostic, so the
rudof integration is untouched: versioning lives entirely in *which turtle
gets selected*. The content-hashed validator cache means two versions of a
shape set are two cache keys and coexist correctly — no cache work.

### 2.3 As-of Σ in audit and replay — an additional mode, not a replacement

Reading live Σ stays the default; it is the drift check and its rationale
stands. Replay gains **both** comparisons, reported as separate columns:

- **Fidelity** — the trace vs Σ as of the trace window: was enforcement right
  *then*?
- **Drift** — Σ-then vs Σ-now: what has moved since, stated as spec movement
  rather than misreported as trace violation?

`audit_spec::load` is one `AsOf` parameter away from the fidelity half for
*policy* facts, which are already bitemporal — that half needs nothing from
§2.1 and could land first. Shape-level fidelity (was this policy *definition*
valid under the shapes in force when written) needs §2.1.

### 2.4 Reports and lineage

- Validation feedback references `(shape_name, shape_tx)`, so "why was this
  write refused" traces to the exact shape version — validation reports as
  queryable data, landing as a consequence rather than a feature.
- `proposals` remains the richer record for *who and why*; `decided_at` is a
  natural effective date. Backfilling pre-versioning history from
  proposal-routed loads is possible but not promised. Direct loads are
  recorded going forward by the versioned write path.

## 3. Prior art

Thin — which is the point. Temporal-validity *constraints inside* shapes
appear in the literature; a versioned shape *set* with effective dates
("validate as of the policy in force at date Y") has no established
implementation. Quipu is unusually well-placed because the substrate is
already bitemporal: this design adds three columns and an event, not a
temporal engine.

## 4. Scope boundaries (honest)

- **No shape diffing or migration tooling.** Versions are whole turtle
  payloads; comparing two versions is the reader's job.
- **No per-shape effective dates within a set** — the unit of versioning is
  the named shape set (the `name` column), matching the unit of loading.
- **The reactive reasoner's startup snapshot behaviour is unchanged** (rules
  loaded after startup still need a restart); this design records history, it
  does not add hot-reload.
- **Live-Σ remains the audit default.** As-of is a mode; removing the drift
  check would reintroduce the failure it exists to catch.

## 5. Build order

1. This document.
2. Bitemporal `shapes` + `ontologies` registry, close-don't-overwrite,
   `shapes.loaded` events, `get_combined_shapes(as_of)`, `valid_at` on
   `/validate`. Independent of the graph-labels track; can land any time.
3. As-of Σ in audit/replay — fidelity and drift as separate columns. Depends
   on step 2 only for shape-level fidelity; the policy-fact half could land
   first.

## 6. Related

- [graph-labels.md](graph-labels.md) — the label lattice; shape versioning is
  the policy layer's time axis, labels are its trust axis.
- [policy-edit-hooks.md](policy-edit-hooks.md) — the write-time policy gate
  whose verdicts gain a citable shape version; also the multi-value
  refuse-don't-resolve precedent.
- [named-graphs.md](named-graphs.md) §7.2 — shapes live in ROOT; unchanged
  here.
- `src/proposal.rs` — the lineage that already exists and stops being thrown
  away.

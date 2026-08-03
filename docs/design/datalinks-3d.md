# Datalinks: a spatial explorer for Quipu graphs

> **Implementation status (2026-08-02):** 🟡 **Spike.** One of four designed
> modes is built and shipping: **Datalinks** (`ui/datalinks.js`, served at
> `/#datalinks`), a 3D rank layout over the prerequisite DAG with personalized
> PageRank re-lighting. The Grid, the Stacks, and the Archives are design only.
> The repo ingest that feeds the unbuilt modes ships as `scripts/ingest-repos.py`
> and is SHACL-validated. Measurements in this doc were taken on 2026-08-02
> against `NeuralAmplifier/datalinks/thinker/alphax.ttl` and the four sibling
> repos.

## Status

- **Date**: 2026-08-02
- **Status**: Spike — one mode built, three designed
- **Predecessor**: [`quipu-ui.md`](./quipu-ui.md), whose Graph Explorer section
  specs the 2D Sigma.js view this sits beside. This is a successor to that
  section, not a replacement for it.

## The thesis

3D graph explorers usually fail because they take a force-directed layout and
add a Z axis. The result is a hairball you can now get lost *inside*.

> **Position is derived from the data, and it is stable.**

A node's location is its call number — a deterministic path through a
hierarchy. Nothing floats, nothing re-anneals when a fact lands, and two loads
of the same data agree exactly. That buys what a force layout never can:
**spatial memory**. You learn the building.

Everything below depends on that property. Anything that would destabilise
position has to justify itself against it. It is also what makes the view
*testable*: a force layout can only be checked by eye, but derived placement
supports structural assertions (see [Verification](#verification)).

## The four modes

One camera, one dataset, four spatial grammars. The transitions between them
are the interface. Only Datalinks is built.

### 1. Datalinks — built

Hypertext-first, keyboard-driven, no walking. A focused entry, its
cross-links, its prerequisites. A 3D app needs a fast non-3D path, and this is
it.

- **Height = longest-path depth** through the dominant relation
- **Bearing = the aiWeight compass** where present (below)
- Entry panel renders `effectText` prose with Requires / Unlocks navigation
- `[` and `]` walk the selected node's links without touching the mouse

### 2. The Grid — designed

FSN / *Jurassic Park*: directories as platforms, files as blocks on top, arcs
for cross-references, fly-through navigation. The mode whose data is deepest —
`repo → directory → file → symbol` is a genuine hierarchy, unlike the aegis
class tree. `scripts/ingest-repos.py` already produces its data.

### 3. The Stacks — designed

Class-shelved books you can pull down and read, entered through a card
catalogue: type-ahead, drawer slides, cards riffle, pick one and *fly* to the
shelf. The flight is not a teleport — it is what teaches you the building.
Three lookups behind one drawer: substring (`labeled_like` in the `quipu_ask`
catalogue), semantic (`/hybrid_search`, `/spotlight`), and fuzzy-name
(`/reconcile`). `semweb::fetch_labeled_entities` is already a catalogue index.

### 4. The Archives — designed

A rotatable projection of the whole graph, for orientation and anomaly
spotting. Its distinctive view is **absence**: classes declared in `shapes/`
with zero instances, `minCount` violations, dangling references — an empty
plinth where a book should be. Quipu computes all of it via `POST /validate`
today.

## What was measured

Numbers taken 2026-08-02, not estimated.

| Source | Volume | Shape |
|---|---|---|
| `alphax.ttl` (NeuralAmplifier) | 374 entities, 3 987 facts, 12 classes | Prerequisite DAG: 88 technologies, 329 `requiresTech` edges, 17 ranks |
| Sibling repos (quipu, hank, NeuralAmplifier, thinker) | 6 160 entities, 30 668 facts | Path tree: 276 modules, 4 184 symbols, 132 documents, 1 568 sections |
| `shapes/` | 95 shapes, 76 target classes | **Nearly flat** — 14 `rdfs:subClassOf` edges total |
| `examples/demo-graph/demo.ttl` | 38 entities, 169 facts, 8 classes | Small, cyclic, non-DAG |

Two of these reshape the design.

**The aegis ontology is nearly flat.** 76 target classes joined by 14 subclass
edges — `Host` has three subclasses, `Service` four, `Directive` and `Script`
one each, and `code-entities.ttl` has none. Shelving purely by declared
hierarchy yields one floor with ~70 doors. So shelving must be **composed**:
declared `subClassOf` where it exists, then the file-path hierarchy for
code/docs, then predicate affinity and Louvain (`quipu_project
{"algorithm":"louvain"}`) for the rest.

Worth stating plainly: flatness is invisible in a force graph, where every
layout looks equally like a hairball. A building makes it obvious. The explorer
is a **forcing function for ontology quality**, and Quipu already ships the fix
path (`quipu_propose_schema_change`).

**Shipped demo data is tiny.** 38 entities. `MAX_LIMIT = 2000` in
`src/graph_view.rs` is nowhere near binding; the repo ingest is what makes this
a library rather than a diorama.

## Layout

Implemented in `ui/datalinks.js`.

**Rank.** An edge `a → b` reads "a requires b", so `rank(a) = 1 + max(rank(b))`
over outgoing edges. Computed by memoised DFS with an on-stack guard: a general
graph is not a DAG, and a cycle would otherwise recurse forever. A node on a
cycle contributes 0 rather than aborting — a cyclic graph still renders, it
just renders flat where it is cyclic. This is what lets the same view serve
`demo.ttl` (maxRank 8, cyclic) and `alphax.ttl` (maxRank 16, acyclic).

**Bearing.** Every SMAC technology carries `aiWeightGrowth`, `aiWeightTech`,
`aiWeightWealth` and `aiWeightPower`. Treated as compass bearings at 0°, 90°,
180° and 270°, a node's angle is the weighted circular mean — so the lattice
spirals up with wealth techs on one bearing and power techs on another, and
position carries meaning. Absent weights, bearing is an FNV hash of the IRI:
arbitrary but deterministic, so the layout is still identical across loads.
Dense rings fall back to even angular spacing so nodes cannot overlap.

**Encoding.** Colour and geometry come from the eight-slot palette exported by
`ui/graph-canvas.js` — validated for CVD separation against this exact surface,
assigned by type prevalence, never cycled, with a ninth type folding into a
neutral `Other`. Identity is (colour, shape); in 3D the shape channel becomes
geometry. Size is degree, so a hub is physically bigger.

## Lighting is importance

Four channels, all already shipping in `src/graph.rs` and exposed via
`quipu_project` / `POST /project`.

| Channel | Source | Renders as |
|---|---|---|
| Global importance | `page_rank` | Size and base luminosity — static |
| **Local importance** | **`ppr`**, seeded | **The scene re-lights as you move** |
| Neighbourhood | `impact` BFS | Concentric falloff; `via_predicate` lets you draw the path, not just the set |
| Community | `louvain` | Wing colour; already persisted as bitemporal `quipu:memberOfCommunity` triples |

**Personalized PageRank is the interaction.** Global PageRank is static
prettiness; PPR is a verb. Select a node and the lattice re-illuminates by
relevance-to-here. Two things had to be right for it to read:

**Restrict the projection to the spine.** PPR runs over a projection built from
*all* reference-valued facts, which here includes `rdf:type` and `sourcedFrom`.
Those are not drawn, so an unfiltered PPR both scores invisible nodes and leaks
rank through invisible edges — seeded on a root technology it returned mostly
zeros plus the class node. The UI therefore derives the dominant predicate from
the rendered `/graph` payload, resolves its full IRI with one aggregate query,
and passes it as `predicate`. Generic, not hardcoded: on `demo.ttl` it resolves
to `runsOn`, on `alphax.ttl` to `requiresTech`.

**Rank, don't normalise.** PPR mass concentrates hard on the seed, so
`score / max` leaves the entire reachable set within a few percent of black —
the picture reads as "everything went dark". Percentile ranking spreads the
reachable set across the full brightness range. Unreached nodes go *grey*
rather than merely dark: hue-vs-no-hue separates far more cleanly on this
surface than bright-vs-dim, and it keeps the untouched lattice legible as
structure instead of erasing it.

One related trap: `emissive` is a material property and does **not** respond to
per-instance colour, so a high emissive floors every node at a fraction of its
hue and flattens the re-light into mud. It is deliberately low (0.16), with
ambient light raised to compensate.

## Art and assets

**SMAC has no 3D models.** It is a 1999 2D isometric sprite game — unit art is
`.pcx` sheets composited from chassis + weapon + armour, plus `.avi`
secret-project movies in the expansion. There is nothing to extract.

NeuralAmplifier already has the settled discipline, and it is inherited rather
than relitigated. From its `fixtures/smac/PROVENANCE.md`: *"No game data lives
here — only paths and checksums. The bytes live in `$SMAC_DIR`, outside the
tree."* `/datalinks/*` is gitignored for the same reason. Therefore:

1. **Procedural geometry in the Datalinks visual language** — dark panel,
   cyan/amber monospace, bevelled entries. A style, not an asset. This is what
   ships.
2. **Optional sprite billboards from `$SMAC_DIR` at runtime** — facility and
   unit icons read from the user's own install, never committed, degrading to
   procedural when unset. Not built.
3. GLSMAC (`GLSMAC_DIR`) is worth a look for openly-licensed art, but it also
   sources original game data for most assets.

No game art is vendored under any of these.

## Air-gap

`ui/graph-canvas.js` states the rule: *"Local module, not a CDN: the graph must
render on an air-gapped deploy."* three.js r169 (MIT) is therefore **vendored**
at `ui/vendor/three.module.min.js` and `include_str!`'d into the binary like
every other UI asset, with its licence beside it. Nothing is fetched at runtime.

Note for anyone re-vendoring: unpkg and jsDelivr are both blocked by the agent
proxy (403 on CONNECT); `registry.npmjs.org` is in the `noProxy` set and serves
the tarball directly.

## Files

| Path | Role |
|---|---|
| `ui/datalinks.js` | Rank layout, instanced meshes, orbit camera, picking, importance re-light |
| `ui/vendor/three.module.min.js` | Vendored three.js r169 (MIT), 687 KB |
| `ui/index.html` | `#datalinks` route, entry panel, PPR wiring, spine resolution |
| `src/server.rs` | `include_str!` + `/datalinks.js`, `/vendor/three.module.min.js` |
| `src/server/base.rs` | The two asset handlers |
| `src/http_auth.rs` | Both paths on the unauthenticated read allowlist |
| `scripts/ingest-repos.py` | Repo → Turtle for CodeModule / CodeSymbol / Document / Section |
| `justfile` | `just datalinks`, `just ingest-repos` |

## The ingest

`scripts/ingest-repos.py` emits Turtle against the **existing**
`shapes/code-entities.ttl` vocabulary — no new classes. Predicate choice is
constrained by shapes that fire on *any* subject: `aegis:contains` is bound to
`sh:class aegis:Bead` by `shapes/provenance.ttl`, and `bobbin:` is the same IRI
namespace as `aegis:`, so a Document-contains-Section edge would violate it.
Sections therefore use `bobbin:inDocument`, which nothing targets; symbols use
`bobbin:definedIn`, constrained only inside `CodeSymbolShape`.

Symbol IRIs carry their line number, because two symbols may share a name in
one file (methods across `impl` blocks) and a collision would assert two values
for the `maxCount 1` name and symbolKind properties.

Verified clean against `code-entities.ttl`, `provenance.ttl`, and the full
1 615-line `aegis-ontology.shapes.ttl`.

## Verification

```bash
just check && just test           # the mandatory pre-push gate
just datalinks                    # alphax.ttl -> http://localhost:3030/#datalinks
just ingest-repos                 # the 6 160-entity code + docs graph
just demo                         # small, cyclic, non-DAG fallback
```

Assert **structurally**, not by pixel. Stable derived placement is exactly what
makes this possible — `quipu-ui.md` §Layer 4 argues the same point in reverse
about force layouts. The invariants that matter:

- every spine edge points strictly downward in rank (checked: 0 violations of
  329 `requiresTech` edges)
- node count matches the payload, rank count matches the DAG depth
- two loads produce identical positions
- a node's rendered prerequisites match the source Turtle

## Open questions

1. **Binary size.** Vendored three.js adds 687 KB to a binary that already
   links SQLite, ONNX and tokenizers. Marginal in context, but a hand-rolled
   WebGL renderer (~400 lines) would avoid the dependency entirely. Deferred
   deliberately: the point of the spike was to test whether the metaphor works,
   and three.js got there faster.
2. **`POST /graph` has no `valid_at` / `as_of_tx`.** It is current-state only,
   which blocks every temporal idea below. `TemporalContext` already exists in
   `src/sparql/mod.rs`; wiring it into `tool_graph_view` is plumbing, and it
   would help the existing 2D explorer too. This is the highest-value next
   change.
3. **PageRank has no write-back.** No `quipu:pageRank` persistence, so it
   recomputes per call — fine at 374 nodes, worth measuring at 6 160. See
   [`pagerank.md`](./pagerank.md).
4. **Labels.** The spike has no in-scene text; identity is (colour, shape) plus
   the entry panel. An SDF atlas is the obvious next step and the expensive one.
5. **Mode transitions.** Animated morph or hard cut between the four modes?
   The morph is the delight, but it is also where the work is.
6. **WebXR.** Nearly free with three.js, and a walkable lattice is well suited
   to room-scale — but desktop-first, or navigation and text legibility eat the
   project.

## Later: what the modes unlock

Not built; recorded so the shape of the thing is on paper.

**Bitemporal** — the elevator is transaction time: ride to March and the
building reshelves to what you believed then; valid time is the edition on the
shelf. **Graph diff as ghosts and glows** — removed things are red translucent
ghosts still in place, added things glow. Walking a diff beats reading one.
Backed by `POST /entity_history` and `GET /transactions`, and gated on open
question 2.

**Provenance** — bookplates naming episode and actor; an acquisitions room
where opening an episode lights up every entity it touched; derived facts
shelved apart and translucent, since they self-identify via
`source = "reasoner:<id>"` and `"owl:materialize"`.

**Governance** — SHACL violations as damaged books on a repair cart; policy
effects as signage; ed25519 verdicts as wax seals; pending schema proposals as
translucent scaffolding, a wing under construction. `aegis:tier`
{live, lsp, tree-sitter, committed, attested} maps onto binding quality —
signed leather down to cheap paperback — which is Hank's tier discipline made
visible across the room. It pairs exactly with `smac:ruleTier`
{canonical, house-rule}: the whole stack tiers its facts by trust, so one
channel serves every dataset.

**Two geometries** — morph between call-number shelving and a UMAP of the
384-dim embeddings. The things that fly furthest are where declared class and
semantic content disagree: a schema-smell detector you can watch run.

**Guided entry** — `POST /report` already returns PageRank hubs, *surprising
connections* (low-prior edges bridging two Louvain communities), and suggested
questions. An anomaly light, server-side, today.

## References

- Sigma.js / Graphology — the 2D lineage: <https://www.sigmajs.org>
- FSN, the SGI 3D file browser behind the *Jurassic Park* scene:
  <https://en.wikipedia.org/wiki/Fsn_(file_manager)>
- three.js: <https://threejs.org>
- Personalized PageRank: [`pagerank.md`](./pagerank.md)
- The 2D UI design this succeeds: [`quipu-ui.md`](./quipu-ui.md)

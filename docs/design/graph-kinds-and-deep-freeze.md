# Design: Graph kinds and deep freeze — a data-kind axis on the label lattice, and cold storage that stays composable

> **Implementation status (2026-08-24):** ✅ **Built on the sanctioned
> surfaces.** The `quipu:dataKind` axis: `src/lattice_kind.rs` (`DataKind`,
> `KindSet`), label plumbing in `src/store/labels.rs`, cache columns
> `graphs.data_kind` / `graphs.lifecycle` (guarded in
> `migrate_graph_labels`, classified in `respace_map`), pack/attach travel,
> `kind` on `/graph/label`, `GET /graphs` + `quipu_graph_list`
> (`src/store/registry_list.rs`), `include_kinds` on `/query`
> (`mcp::query_context`), and the `deny_data_kinds` floor (unset by default —
> zero behavior change). Deep freeze: `src/store/freeze.rs` +
> `src/store/freeze_io.rs`, the `frozen_packs` registry, the write guard in
> `Store::assert_graph_is_writable`, auto-attach on open, CLI
> `quipu graph freeze|thaw|list` (`src/cli_graph.rs`), REST
> `POST /graph/freeze|thaw`, MCP `quipu_graph_freeze|thaw`. Acceptance is
> pinned by `src/store/freeze_tests.rs`: the three cold-composition opt-ins
> return identical rows post-freeze, reopen auto-attaches, a missing pack
> refuses the open, a frozen graph refuses writes naming thaw, and
> freeze→thaw round-trips the canonical history byte-for-byte.
> Deliberately NOT built (beads filed): write-gate signature verification
> (quipu-8cc), vector travel for frozen graphs (quipu-0v4), a
> `[quipu] attachments` config surface.

**Status:** ticket and workflow engines (shuttle first) map high-volume
operational data into quipu. Left in ROOT or in ever-growing named graphs it
swamps the hot store; deleted, it violates the store's first commitment —
nothing in quipu deletes facts. This design adds the two missing pieces: a
declared **kind** dimension saying what sort of data a graph holds, and a
**freeze** operation that relocates a whole graph's history into a read-only
archive pack while keeping it addressable and composable at query time.

## 1. The kind axis

`quipu:dataKind` is a fifth declared label axis beside freshness, durability,
trust and policy. It is **categorical, not ordered** — there is no "weaker"
of `knowledge` vs `operational`, and inventing an order would be the sign
error `lattice.rs`'s module docs warn about. A graph declares one kind; a
dataset composes to the **union** of its members' kinds (`KindSet`, a `Join`
like `PolicyClass`), each with `Coverage`. A query whose dataset touched an
archive graph reports `archive` among its kinds — the information
accumulates, never averages away.

The value space is lexical (`[a-z][a-z0-9-]*`), not an enum: a fifth kind
must not require a quipu release. The conventioned set:

| kind | holds | freeze posture |
|---|---|---|
| `knowledge` | durable semantic content (camayoc planes) | rarely frozen |
| `operational` | high-churn workflow/run/ticket state | the freeze candidate |
| `identity` | principals, verifier registrations, keys | **never frozen** — freezing a window must not strand the keys that verify its signatures |
| `archive` | frozen read-only history | set by freeze, not by hand |

`deny_data_kinds` in `[quipu.labels]` is a **blocklist**, unlike the minimum
floors: an undeclared kind passes. Failing every unlabelled graph the moment
the key is set would break every existing store; the freshness/trust floors
stay the fail-safe minimums they were.

## 2. Deep freeze — durability-declared relocation

"A thing that existed is a fact about the past" is a claim about the
**composed store**, not about which file holds the bytes. Freeze deletes
`main.facts` rows for graph *g* only after:

1. a **full-history** export — every row including retracted ones, their
   transactions, and `retracted_tx` — lands in a pack
   (`freeze_io::export_graph_history`; the current-facts `pack()` is NOT
   sufficient and is not used);
2. the pack is respaced into its own term space (`respace_file`) so it
   attaches without id collisions;
3. the copy is **verified**: the canonical history form
   (`freeze_io::history_canonical` — IRI-resolved, sorted, term-id-free) is
   hashed on the main store before anything is written and recomputed from
   the pack's own rows after;
4. the registry records the relocation: `graphs.lifecycle = 'frozen'`, the
   relabel to `dataKind=archive` + `durability=backed` (durability genuinely
   IS backed now), meta-graph facts `quipu:lifecycleState` /
   `quipu:frozenInto` (the content hash) / `quipu:frozenAt`, and a
   `frozen_packs` row — all in one savepoint with the row delete;
5. the pack re-attaches read-only, in-process and on every subsequent open.

This is camayoc's *durability-declared relocation*
(`what-belongs-in-the-graph.md` §4b) and deliberately not a write-time
importance filter (§5): freeze is offered post-hoc, for whole graphs, on the
producer's own declaration that the graph is `operational`.

**What is genuinely lost, stated plainly:** `as_of_tx` time travel across
the composition — pre-existing and honestly refused (`sparql/mod.rs`
refuses `as_of_tx` with attachments), not newly silent. Valid-time queries
survive: rows carry their windows verbatim. Local embeddings are dropped in
v1 (quipu-0v4). Trust labels do not travel into the pack: a rank is anchored
to a chain in the origin store's meta-graph and does not survive relocation;
the consumer's floors decide.

**Windows.** Freeze operates on whole graphs, so producers write operational
data into time-windowed graphs (`{base}graph/shuttle/runs/2026-08`, monthly)
and freeze completed windows. Intra-graph `valid_to`-based freezing was
rejected: the graph is the unit of pack, attach, label and authority, and
splitting below it forfeits all four.

**The bitemporal move rule** (shared with camayoc-913, plane promotion):
assert in the target, close in the source, record the move. For freeze the
"close" is the whole-graph relocation with the pack as the surviving
history; thaw closes the `lifecycleState` fact with a retraction, never a
deletion, and the `frozen_packs` row keeps `thawed_at` rather than
disappearing (the fork-status precedent).

## 3. Cold composition — the three opt-ins

A frozen graph stays addressable **without thawing**. Silence never widens:
the default scope stays ROOT-alone, and each path below is explicit.

1. **By IRI** — `GRAPH <iri>` / `FROM <iri>`: works the moment the pack
   attaches; `lookup_all` resolves the IRI across term spaces, so the local
   (now empty) row and the archive's row both contribute.
2. **By dataset** — `FROM <urn:quipu:dataset:frozen>` or the `graph`
   request param: freeze and thaw auto-maintain the membership.
   `dataset_member_ids` resolves members via `lookup_all` for exactly this
   case.
3. **By kind** — `include_kinds: ["archive"]` on `/query`: widens the
   default graph set with every graph declaring one of the named kinds,
   resolved from the registry so new frozen windows join automatically.
   Composed labels then honestly report `kind: {…, "archive"}`.

A configured `deny_data_kinds: ["archive"]` floor refuses the implicit
widening paths while an explicit `FROM <specific-frozen-iri>` still reads —
floors are not access control, and the warning on the trust floor applies
verbatim here.

## 4. Write guard

`Store::assert_graph_is_writable` checks `lifecycle = 'frozen'` before the
attached-source check, unconditionally (not gated on attachments), because a
local write tagged with a frozen graph's id would land beside the archive
and be read as if the archive supplied it — the exact defect the source
check exists to stop, one lifecycle earlier. The refusal names
`quipu graph thaw <iri>`. Label writes stay possible while frozen (they
target the meta-graph, and thaw itself relabels).

## 5. Surfaces

CLI `quipu graph freeze <iri> [--out dir]` / `thaw <iri>` /
`list [--kind k] [--frozen]`; REST `POST /graph/freeze`, `POST /graph/thaw`,
`GET /graphs` (also the consumer capability probe — a 404 means the store
predates graph kinds, to be read as "cannot tell", never "no graphs"); MCP
`quipu_graph_freeze`, `quipu_graph_thaw`, `quipu_graph_list`.

## 6. Related

- `docs/design/graph-labels.md` — the lattice this extends.
- `docs/design/multi-db-composition.md` — the attach substrate; its
  "small hot DB for churn, large cold DB for stable knowledge" motivation is
  this feature.
- `docs/design/knowledge-packs.md` — the artifact format `pack_format: "2"`
  extends with full history.
- `docs/design/named-graphs.md` — graph axis orthogonal to both time axes.
- camayoc `docs/design/what-belongs-in-the-graph.md` §4b/§5 — the
  durability lattice and the no-write-time-filtering rule this design obeys.
- shuttle (`scbrown/shuttle`) — the first windowed producer.

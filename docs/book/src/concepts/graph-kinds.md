# Graph Kinds & Deep Freeze

> **Implementation status:** ✅ **Built** on the sanctioned surfaces — the
> `quipu:dataKind` label axis, `GET /graphs`, `include_kinds` on `/query`,
> and `quipu graph freeze|thaw|list`.
> See `docs/design/graph-kinds-and-deep-freeze.md` for the full design.

## The kind axis

`quipu:dataKind` declares what sort of data a graph holds — a fifth label
axis beside freshness, durability, trust and policy. Categorical, not
ordered: a dataset composes to the **union** of its members' kinds, so an
answer that touched cold data says so in its `labels.kind`.

Conventioned values (the space is lexically open — `[a-z][a-z0-9-]*`):

- `knowledge` — durable semantic content;
- `operational` — high-churn workflow/run/ticket state, written into
  time-windowed graphs (e.g. `…/shuttle/runs/2026-08`);
- `identity` — principals and verifier registrations, split out so freezing
  a window never strands the keys that verify its signatures;
- `archive` — frozen read-only history; set by the freeze operation.

Declare it like any label:

```bash
curl -s localhost:3030/graph/label -X POST -H "Content-Type: application/json" \
  -d '{"graph": "urn:app:runs/2026-08", "kind": "operational",
       "timestamp": "2026-08-24T00:00:00Z"}'
```

`[quipu.labels] deny_data_kinds = ["archive"]` refuses queries that
implicitly compose the named kinds — a blocklist (undeclared passes), not a
minimum, and like every floor it is **not access control**.

## Deep freeze

`quipu graph freeze <iri>` relocates a whole graph's **full history** —
retracted rows and transactions included — into a read-only archive pack,
verifies the copy by content hash, deletes the local rows, and re-attaches
the pack. The graph keeps its IRI and stays queryable; its durability
genuinely becomes `backed`. Writes to it are refused, naming
`quipu graph thaw`.

Compose frozen graphs back in, explicitly (silence never widens):

1. `GRAPH <iri>` or `FROM <iri>` — by name;
2. `FROM <urn:quipu:dataset:frozen>` — the auto-maintained dataset of every
   frozen graph;
3. `"include_kinds": ["archive"]` on `POST /query` — by kind, so new frozen
   windows join automatically.

Known cost: `as_of_tx` time travel is refused while archives are attached
(the pre-existing rule for any attachment); valid-time queries survive.
`quipu graph thaw <iri>` restores the history byte-for-byte and reopens the
graph for writes — the pack file stays on disk, and the freeze registry row
is closed, never deleted.

### Freezing and semantic search

**Freezing costs the freezing store nothing here.** Freeze deletes the graph's
fact rows; it never touches the `vectors` table, so the embeddings of a frozen
graph's entities stay in place and semantic search answers as it did before.

**The archive carries them too.** A freeze pack holds embeddings for the
graph's own subjects, re-keyed by IRI, and both `quipu graph thaw` and
`quipu graph import` restore them — so a window handed to another store, or
thawed into a store rebuilt from packs, arrives with its semantic index rather
than needing a re-embed. Freeze and thaw report the count; the restore is
idempotent.

Two limits worth knowing:

- **A delegated or LanceDB vector backend cannot be enumerated**, so nothing
  can be re-keyed out of it. The freeze still succeeds — relocating history is
  not a vector operation — but it warns on stderr and stamps
  `vectors_omitted` into the pack manifest, so an incomplete archive never
  reads as "this graph had no embeddings". A `thaw` in the other direction
  *refuses*: restoring rows into a store whose live backend is not the built-in
  one would put them where nothing reads them. Run `quipu migrate-vectors`
  first.
- **An attached archive's embeddings are not searched from the pack.** Vector
  search reads the local store only, deliberately — one index per question.
  Nothing is lost by it, because the local rows were never removed.

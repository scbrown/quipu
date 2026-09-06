# Standard share artifacts

Status: implementation boundary for `aegis-iv3df7.5`.

Quipu has one portable graph artifact: a **share**. A repository `.qpack` is a
share packaged for release, not a SQLite database. SQLite may be constructed as
an internal cache after verification, but it is never a published interchange
format. Bobbin indexes remain separate binary artifacts.

## Repository content selection

Repository shares are built from the repository checkout, not from a live
homelab database. The release job runs Bobbin's normal knowledge-enabled index
path with `quipu_push_chunks = true` and no remote endpoint, producing the same
replaceable `bobbin-chunks:{repo}` snapshot in an embedded Quipu store. It then
loads the repository's shipped shapes and adds the repository, governing
Directive, and release WorkItem context before sharing.

Bobbin publishes that snapshot into ROOT: its snapshot key is provenance and
replacement identity, not a named graph or `group_id`. Therefore the Quipu
release uses the checked-in
[`queries/repository-share-quipu.rq`](../../queries/repository-share-quipu.rq)
CONSTRUCT selector. The query explicitly permits Bobbin's structural predicates
for Quipu code/document IRIs and unions the repository context nodes. It must
not select every outgoing ROOT fact: other producers enrich the same subjects
with private operational references, and a public share must neither leak those
facts nor depend on the homelab store.

## Wire form and identity

A full share contains `manifest.ttl`, `payload.nq`, and `shapes.ttl`. A release
asset is those files in a deterministic POSIX tar archive named `*.qpack`; a
served directory exposes the same files individually. Archive entry order,
paths, modes, owners, and timestamps are normalized, but archive bytes do not
define identity.

`payload.nq` is an RDF dataset serialized as canonical N-Quads by
[RDFC-1.0](https://www.w3.org/TR/rdf-canon/). Its SHA-256 digest defines the
payload identity. This replaces Quipu's ordering-dependent hash of `export.nt`.
N-Triples remains a derived convenience view only when the scope has no named
graphs. `shapes.ttl` remains SHACL; its exact UTF-8 bytes have a separate
SHA-256 checksum because shapes are source text, not part of the shared data
dataset.

The normative manifest is RDF in Turtle. It describes the share as a
`dcat:Dataset`, each file as a `dcat:Distribution`, and records media type,
download URL when known, byte size, and `spdx:checksum`. The share is also a
`prov:Entity`, linked to its producer and `prov:generatedAtTime`. A child uses
`prov:wasRevisionOf` for its immediate parent and `prov:wasDerivedFrom` for
non-immediate ancestry. Quipu-specific scope and transaction-anchor predicates
remain in its namespace because PROV-O and DCAT have no equivalent. JSON-LD is
an optional negotiated representation of the same manifest graph, never a
second manifest.

The share identifier is `urn:sha256:<digest>` of canonicalized manifest RDF
after omitting the identifier itself and transport-location facts. Consequently
moving a release asset does not change its identity, while changing payload,
shapes, lineage, scope, or producer does.

## Import by reference

`quipu import <URL>` accepts an HTTP(S) directory manifest or a `.qpack` release
asset. It follows bounded redirects, enforces response and expanded-size limits,
rejects unsafe archive paths, and keeps fetched bytes in bounded memory; it does
not create a user-visible download. Before opening a destination store it:

1. parses the manifest with an RDF parser and permits only declared files;
2. verifies every checksum and the self-derived share identifier;
3. canonicalizes `payload.nq` with RDFC-1.0 and verifies its dataset digest; and
4. parses SHACL and payload syntax completely.

Only then does it open `Store::open_in_memory()`, import through the existing
staging/quarantine policy, and return a handle whose lifetime is the command,
server session, or WASM object that owns it. Dropping the owner drops the store.
Promotion remains explicit; network origin never grants ROOT admission.

For a graph URL, Quipu adopts the
[SPARQL 1.1 Graph Store HTTP Protocol](https://www.w3.org/TR/sparql11-http-rdf-update/)
and HTTP content negotiation: a manifest distribution may identify a graph with
`dcat:accessURL`, and the importer GETs an RDF representation from that graph
resource. Graph Store Protocol is not stretched into a multi-file packaging
protocol. An LDP container may advertise directory members, but LDP is optional
discovery rather than a required import dependency: GitHub release assets and
ordinary static hosts do not expose LDP containers, while the manifest already
provides a closed, checksum-bound inventory.

## Delta shares

A delta share substitutes `delta.ru` for `payload.nq`. `delta.ru` is a
[SPARQL 1.1 Update](https://www.w3.org/TR/sparql11-update/) request containing
only ground `DELETE DATA` followed by ground `INSERT DATA`; variables, `WHERE`,
`LOAD`, `SERVICE`, graph-management operations, and relative IRIs are rejected.
The manifest declares media type `application/sparql-update`, the immediate
parent share identifier and parent payload digest, plus the resulting payload
digest. Exact normalized UTF-8 update bytes are checksummed; the delete and
insert RDF datasets are independently RDFC-canonicalized so semantically equal
deltas have stable component identities even though SPARQL Update has no
canonical concrete syntax.

`quipu share --since <parent URL-or-id>` diffs canonical parent and current
datasets and emits the restricted update. Import resolves the full ancestor,
verifies every link before mutation, then applies each delta in order inside one
store transaction. Every link must name the currently materialized parent digest
and must produce its declared result digest; missing, cyclic, ambiguous, or
divergent chains fail closed. A modified in-memory store writes a delta by
default and a full share when explicitly requested.

LD Patch is not the primary delta syntax: it is a W3C Note rather than the
SPARQL standard Quipu already parses, and its path expressions add an unrelated
execution model. Jena RDF Patch is useful implementation prior art but is not a
W3C interchange standard. Both may be accepted later through media-type
negotiation without changing the manifest or lineage model.

## WASM and interoperability gates

Native and WASM use the same parser, canonicalizer, verifier, manifest model,
and delta applier. WASM fetches through the host-provided `fetch` capability and
owns an in-memory store; it never receives SQLite bytes. URL policy and size
limits are supplied explicitly by the host so browser and server behavior cannot
silently diverge.

The feature is not complete until these independent checks pass:

- the official RDFC-1.0 test suite agrees with Quipu's canonical N-Quads and
  digest, including blank-node datasets;
- Apache Jena parses the Turtle/JSON-LD PROV-O + DCAT manifest and independently
  applies `delta.ru`, producing Quipu's declared result digest;
- a Graph Store Protocol test server serves a distribution under negotiated
  N-Quads, and Quipu imports it without bundle-specific server behavior;
- an ordinary static directory and a GitHub-style tar asset import to identical
  in-memory datasets, while checksum, traversal, oversize, cycle, and wrong-parent
  fixtures fail before mutation; and
- native and WASM materialize the same full-plus-delta golden chain and emit the
  same resulting RDFC digest.

## Reconstruction completeness (aegis-9f899e)

The sections above settle the **form**: a `.qpack` is a text share, not a SQLite
blob. This section settles the **content**, and it is a widening: the share
described above carries CURRENT FACTS ONLY, which is not enough to reconstruct
the store it came from.

### The gap, measured

Three ordinary commands — `knot`, `retract`, `knot` — leave a store holding 5
`facts` rows across 3 transactions and 2 entities. `quipu share` of that store
emits **3 triples, 1 entity, 0 transactions**. `export_rdf_subset` calls
`current_facts_in_graph` and serializes `(e, a, v)`; the rest of each row — `g`,
`tx`, `valid_from`, `valid_to`, `op`, `retracted_tx` — and the whole
`transactions` table stop at the boundary.

Three questions the share cannot answer, and the first is why this matters:

1. **Was there ever a `bob`?** No. A retracted entity leaves no trace at all, so
   the share is not merely a lossy view — it is one whose loss falls exactly on
   what somebody decided to remove.
2. **When did `alice` become `principal`?** Unknowable. Both values are current
   (legal multi-value RDF), so the share cannot even order them.
3. **Who wrote any of it?** `transactions` carries timestamp/actor/source per
   transaction and none of it appears.

### Lossless with respect to a DECLARED set

"Losslessly reconstructs the store" cannot mean *every table*, so the manifest
names the set it carries. A consumer then reads what was excluded instead of
inferring it, and adding a table later is a visible format change.

The declared set is not a convenience. For the excluded group below it is a
security requirement, and the manifest saying so is what stops a later
contributor "completing" the format.

| Group | Tables | Disposition |
|---|---|---|
| Content | `facts` (whole row), `transactions`, `terms`, `graphs`, `shapes`, `ontologies`, `queries`, `query_params`, `datasets`, `dataset_members`, `forks`, `proposals`, `term_spaces`, `schema_terms` | serialized |
| Derived | `vectors` | regenerated under a pinned model, never serialized |
| Log | `events` | serialized; `consumers` and `subscriptions` excluded |
| **Excluded** | `attestation_bindings`, `attestation_nonces`, `frozen_packs.path`, `snapshot_uploads`, `snapshot_upload_parts` | never carried |

### Why the excluded group is excluded

**`attestation_bindings` is a trust registry, and carrying it would undo
`aegis-tadzdf` by the back door.** It records which producer sessions *this*
store was told to trust. `share_attestation.rs` consults it first and says why in
its own comment: *"A share that carries a binding does NOT get it registered; if
it did, `attested` would mean 'came with a self-signed claim' and a whole-bundle
substitution would swap the key along with the data and still go green … quipu
never self-registers."*

So the format already refuses to let a bundle carry trust in through the front
door. A "lossless" pack that serialized this table would carry it in through the
back one — the same escalation, arriving labelled as completeness. Two rules each
correct alone and wrong together; neither document anticipated the other.

**`attestation_nonces` is replay state and is wrong in both directions.** Carry
spent nonces and a legitimate re-import is refused as a replay; omit them
silently and a replay the origin had already spent is accepted on the copy.
Excluding it deliberately, and saying so, is the only honest option.

`frozen_packs.path` is a producer-local filesystem path; carried verbatim it
points the reconstruction at files that do not exist. `consumers` is a *reader's*
cursor and `subscriptions` holds webhook URLs — reconstructing either would
resume someone else's position or aim a new store at another store's endpoints.

### Store identity

An unpacked reconstruction **mints a new `store_id`** and records the source as
explicit lineage. Identity is load-bearing in attestation and must not be
transported, for the same reason as the trust registry.

Recording lineage by *reusing* the source `store_id` would also be inert:
measured, no code path compares `store_id` at all — not `share_merge`, not
`share_import`, not `share_delta`. It is minted once per store
(`store/open.rs:261`) and read in exactly one place, to stamp the manifest. What
merge and status actually key on is `parent_share`, so lineage needs its own
declared field rather than a field nothing consults.

Consequence, stated so it is not discovered later: an unpacked reconstruction is
a **different store that agrees with the original**. `quipu merge` against the
origin still works, through `parent_share`. Divergence after unpack is a fork.

### Size

Sizes are the measured ones, and they are why the format is text-and-diffable
rather than a single artifact re-added nightly: a lossless text pack is large on
first import and its nightly cost is the git **delta**, not the whole file. The
LFS decision is therefore made against a measured night-2-minus-night-1 diff, not
against the full size. Vectors are excluded from that calculation entirely — they
are regenerated, which is also why the pinned embedding model and config are part
of the declared set rather than an implementation detail.

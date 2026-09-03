# Standard share artifacts

Status: implementation boundary for `aegis-iv3df7.5`.

Quipu has one portable graph artifact: a **share**. A repository `.qpack` is a
share packaged for release, not a SQLite database. SQLite may be constructed as
an internal cache after verification, but it is never a published interchange
format. Bobbin indexes remain separate binary artifacts.

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

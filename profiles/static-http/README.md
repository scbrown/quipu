# Static HTTP contract candidates (proposed profile v1)

This opt-in profile compares **committed static evidence**. A bound producer in
`static_http_matches` means one candidate has the same explicit service, HTTP
method and full normalized path. It does not establish a runtime dependency,
network reachability, authorization or compatibility of request/response bodies.

The profile consists of an independently loadable SHACL proposal (`shapes.ttl`)
and a stored-query definition (`matches.json`). Neither is registered by default.
Load the existing `code-entities` shapes alongside this profile for manifest
`Document` validation. New vocabulary requires the normal proposal, independent review and accepted
source-files path before enrollment. The source and private fixtures do not grant
production schema, graph, ingestion or query activation authority.

## Evidence envelope

Each candidate has its own IRI and the `HttpContractCandidate` type in the existing
code ontology namespace. Its identity must include repository, commit, source-file
hash and extraction identity, plus the original DTO candidate ID: role, occurrence
byte range, ordered mount/prefix byte ranges, method, full path and authority.
Hash that canonical identity tuple; never identify candidates only by endpoint or
repository/revision/hash. Distinct mounts/registrations must survive collisions.

| Property | Meaning |
| --- | --- |
| `sourceManifest` | Required IRI of the retained DTO `Document` in the same derived scope |
| `candidateTier`, `candidateConfidence` | Required `treesitter` and `candidate`; never promoted by a match |
| `candidateRole` | `consumer` or `producer` |
| `service` | Optional explicit target/owning service IRI; absent when unknown |
| `repo`, `revision` | Repository identity and full committed Git object ID |
| `sourceHash` | `sha256:` plus the digest of the exact source-file bytes |
| `extractorVersion` | Exact extractor/profile build identity |
| `filePath`, `occurrenceSpan` | Module and `start:end` UTF-8 byte range, zero-based/exclusive end |
| `callerSpan`, `handlerSpan` | Optional separate caller and handler byte ranges; never substituted |
| `httpMethod` | Exact uppercase method, or `UNKNOWN` |
| `httpPath` | Full normalized path including all router prefixes |
| `resolution` | `exact` static input or `unresolved` |
| `generatedClient` | Tri-state: true/false, or omitted when unknown; only explicit false can match |

The exporter must read the committed tree and verify source hashes. Shapes check
form/cardinality; they cannot inspect Git, prove digest correspondence or certify
extractor accuracy. Service is optional and single-valued: an unknown/ambiguous service is omitted
and remains unresolved. Required source evidence missing is a validation error,
not a successfully measured empty graph. An input manifest must declare selected
repositories/revisions, extractor coverage, omitted frameworks and content hashes.
The query pins revisions and exact source-file hashes supplied by the caller; it does not discover the latest
revision or prove that the caller's pin is current.

## Query contract

Load only after validating the complete input against the profile. Invoke the
stored query with all eight required parameters: `input_graph`, `service`,
`consumer_repository`, `producer_repository`, `consumer_revision` and
`producer_revision`, `consumer_source_hash` and `producer_source_hash`. The definition's explicit `FROM` names the derived input
scope. There is no all-graph or ROOT fallback. The caller must select a dedicated,
reviewed derived scope containing a complete bounded comparison manifest; an
arbitrary graph IRI does not prove those properties.

Every consumer with a complete validated manifest remains visible. Missing
manifest/tier/confidence is an invalid input, never a successful empty result.
The query requires a same-scope DTO Document with its contract, profile, full
`manifestBody` and `manifestHash`, and returns both manifest IRIs/hashes. The
producer match cannot bypass that join. The mapper must verify the manifest
digest and correspondence to indexed fields before admission; SHACL checks
cardinality/form and the query does not parse JSON or recompute its digest.

Every valid consumer row remains visible. An unbound `producer` means unresolved;
it must not be converted into a missing dependency, a negative runtime claim or a
successful match. A bound row retains both candidates' repository, revision,
source hash and extractor version. Input IRIs retain the module and span.

Both candidates must be exact, nongenerated and at the supplied repository and
revision/source-hash pins. Service, method and full path must match exactly. `UNKNOWN` methods,
wildcards, template paths, query strings and fragments never resolve. There is no
GET, prefix, first-method or first-service fallback. Two producer IRIs with the
same key cause unresolved output, even across repositories or stale snapshots;
this conservative collision rule prevents choosing one from a partial inventory.
The query reports evidence, never writes a dependency edge or promotes to ROOT.

Limits are caller-governed: validate input coverage and bound fixture/graph size
before activation. The query deliberately has no silent row limit; if a future
transport truncates output, treat it as incomplete. Unknown frameworks, dynamic
URLs and base services stay unresolved in the exporter, and cannot be repaired by
this matcher. The Yupana exporter is a separate workstream; its exact profile
mapping must be reviewed jointly before any enrollment.

## Private acceptance

`just test --test static_http_profile` exercises real in-memory stores, governed
shape load, named-graph input, stored-query load and invocation. A populated exact
match is the positive control. Collisions, wrong methods/prefixes, stale revisions,
missing producer evidence, unknown methods, wildcard/template paths, generated
clients and ambiguous service declarations are checked. Identity distinguishes
role, method and mount; missing manifests, missing bodies and authoritative
confidence substitutions are refused. A ROOT collision proves
scope isolation; missing provenance is refused by the shape.

The idea was informed by [Infigraph at the pinned revision](https://github.com/intuit/infigraph/tree/cf82f5d7440d9be158e219df1ef256d6359ed1f6),
particularly `multi/cross_service.rs`. Its path matching illustrates useful
cross-repository discovery, while its wildcard/method fallback is unsuitable for
this profile's evidence threshold. No upstream code was copied.

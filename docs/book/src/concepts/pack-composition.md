# Compose knowledge packs

`quipu compose` loads verified local share directories or archives into one
store, retaining each pack as a named graph and naming their union as a dataset.
ROOT remains unchanged, including when validation passes. Composition is an
inspection operation; it does not grant foreign content trust.

```bash
quipu compose ./operations-pack ./repository-pack \
  --shapes-from ./operations-pack --db composed.db > composition.json
```

The command returns JSON and exits 0 for a conforming union, 2 for a retained,
nonconforming union, or 1 for a refusal. Exit 2 means the composition is available
for inspection in quarantine, not that no data was written. Integrity and
shape-selection errors refuse the whole operation. Internal shares require
explicit `--destination internal`, as on the import path.

## Identity and provenance

Exact IRIs denote one entity across packs. Matching labels never merge entities.
An explicit `owl:sameAs` assertion supplies an auditable identity link; consumers
can follow it while the original names remain in the source graphs. Blank node
identifiers are scoped to the pack even when independent exports both use `c14n0`.

The result names the dataset and each source graph. Query the dataset explicitly:

```sparql
SELECT ?item ?name FROM <urn:quipu:composition:HASH>
WHERE { ?item a <https://example.org/Item>; <https://example.org/name> ?name }
```

For a fact's source membership, name the source graphs from the result:

```sparql
SELECT ?pack
FROM NAMED <urn:quipu:composition:pack:FIRST_SHARE_HASH>
FROM NAMED <urn:quipu:composition:pack:SECOND_SHARE_HASH>
WHERE { GRAPH ?pack { <https://example.org/item> <https://example.org/name> "Item" } }
```

A fact supplied by both packs has two source memberships. The dataset's RDF union
suppresses duplicate triples. `urn:quipu:composition:metadata` records JSON manifests
under `urn:quipu:composition:manifest`, including source references, share and store
identities, transaction anchors, original manifests, attestation status, and validation
results. Records describe observed
compositions; they are not an automatically maintained validation certificate.

## Shape authority and partial snapshots

Without `--shapes-from`, every input must carry an identical shape bundle.
Different bundles are refused with their share IDs and hashes. Explicit selection
chooses that pack's complete bundle for this composition. It never silently unions
conflicting constraints or installs foreign shapes as global policy.

Validation examines the union: another pack can supply a required field. A
nonconforming union stays in the named inspection dataset and is never promoted to
ROOT. The report preserves violation counts and counts by source shape, with at
most 40 individual diagnostics. Types outside the selected authority are reported
separately as `off_vocabulary` and also quarantine the union.

The snapshot vector remains visible. Different source stores' transaction anchors
are incomparable. Mixed dates are permitted and reported; composition does not
claim that snapshots were taken together, every referenced entity has arrived, or
missing facts represent upstream deletion. Pack selection is explicit. Loading a
different snapshot names another composition without replacing an earlier dataset.

## Retractions and replay

Reloading a share reuses its source graph rather than refilling it. Attested shares
still obey the ordinary signature and nonce-replay checks. A local
retraction in that graph survives reload of the same snapshot. Validation on replay
examines the current local union, so removing a required field changes the outcome
to quarantine. Source graph edits affect every dataset that includes that graph.

A distinct historical snapshot is a separate inspection graph, not a restoration
of ROOT or an automatic replacement of the selected dataset. Historical source
membership records what a snapshot contained, not whether its facts remain current.
Retract in the graph whose view you intend to change.

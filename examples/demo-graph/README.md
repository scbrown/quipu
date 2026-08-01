# Demo graph

A small, entirely fictional platform graph: 37 entities and 165 facts across
eight types — services, datastores, clusters, teams, endpoints, libraries,
runbooks and alerts.

```bash
just demo          # load it and serve the explorer on localhost:3030
```

Or by hand:

```bash
quipu knot examples/demo-graph/demo.ttl --db /tmp/quipu-demo.db
quipu-server --db /tmp/quipu-demo.db
```

## Why this exists

Two reasons, and the second is the important one.

**It is a 30-second tour.** The graph is deliberately *layered* rather than
random — endpoints route to services, services depend on services, persist to
datastores, run on clusters, are owned by teams, and are documented by runbooks
and watched by alerts. A graph view is for seeing structure; a randomly wired
one just looks like a hairball. It also has exactly eight types, which is the
size of the explorer's categorical palette, so every colour/shape slot is
exercised.

**It is what screenshots must be taken from.** Everything here lives under
`example.org`, the domain [RFC 2606](https://www.rfc-editor.org/rfc/rfc2606)
reserves for documentation, so nothing in it can be mistaken for a real
deployment.

That second point is a real hazard, not a formality. This repo runs a ratchet
(`tests/no_internal_identifiers.rs`) that reads **every tracked file as raw
bytes** to keep internal hostnames out of a public repo — it was written because
a live graph database was committed here and the text-based sweep missed it. But
a screenshot bakes its text into *pixels*, and no byte scanner can read those.
An image is the one artifact that can carry an internal hostname straight past
the guard.

So: screenshots in this repository come from this dataset, never from
`test-fixtures/test-store.db` (which mirrors a real environment) or from a
live store.

## Try these

```sparql
# Which services would a checkout outage take with it?
SELECT ?s WHERE { ?s <http://example.org/demo/dependsOn>+ <http://example.org/demo/checkout> }

# Everything one team owns, with where it runs
SELECT ?svc ?cluster WHERE {
  ?svc <http://example.org/demo/ownedBy> <http://example.org/demo/team-payments> ;
       <http://example.org/demo/runsOn>  ?cluster .
}

# Alerts with no runbook — the gap worth knowing about
SELECT ?a WHERE {
  ?a a <http://example.org/demo/Alert> .
  FILTER NOT EXISTS { ?a <http://example.org/demo/resolvedBy> ?rb }
}
```

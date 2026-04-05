# Triples and the Knowledge Graph

Everything in Quipu is a **triple**: a subject, a predicate, and an object.

```text
<koror>  <runs>  <traefik>
 subject  predicate  object
```

This triple says "koror runs traefik." Three triples can encode a complete
service dependency:

```turtle
@prefix hw: <http://example.org/homelab/> .

hw:koror    a               hw:Host .
hw:koror    hw:runs         hw:traefik .
hw:traefik  hw:dependsOn    hw:pihole .
```

That's it. No tables to design, no schema migrations. You add facts
incrementally and query them with SPARQL.

## IRIs: Naming Things

Every entity and predicate is identified by an **IRI** (Internationalized
Resource Identifier) — a globally unique name like a URL:

```text
http://example.org/homelab/koror
```

Prefixes keep things readable. Instead of writing the full IRI every time:

```turtle
@prefix hw: <http://example.org/homelab/> .

hw:koror  hw:runs  hw:traefik .
```

`hw:koror` expands to `http://example.org/homelab/koror`.

## Objects: References vs Literals

The object of a triple can be either:

- **A reference** to another entity (another IRI)
- **A literal** value (a string, number, boolean, or date)

```turtle
hw:koror  hw:runs      hw:traefik .          # reference → another entity
hw:koror  hw:hostname  "koror.lan" .          # literal → a string
hw:koror  hw:cpuCores  "4"^^xsd:integer .     # literal → a typed number
```

## How Quipu Stores Triples

Under the hood, Quipu stores triples as **EAVT facts** in an immutable log:

| Field | Meaning | Example |
|-------|---------|---------|
| **E** (entity) | The subject | `hw:koror` |
| **A** (attribute) | The predicate | `hw:runs` |
| **V** (value) | The object | `hw:traefik` |
| **T** (transaction) | When it was written | `tx:42` |

Every fact also carries a **valid-time window** (`valid_from`, `valid_to`),
so you can model when facts were true in the real world — not just when
they were recorded. See [The Temporal Model](temporal-model.md) for details.

## The RDF Data Model

Quipu uses the [RDF](https://www.w3.org/RDF/) data model, which means:

- Facts are interoperable with any RDF tool
- You can ingest data in Turtle, N-Triples, JSON-LD, RDF/XML, or TriG
- You query with SPARQL — the standard RDF query language
- You validate with SHACL — the standard RDF constraint language

You don't need to know RDF theory to use Quipu. If you can read
`subject predicate object .` you're ready to go.

## Loading Triples

From a Turtle file:

```bash
quipu knot homelab.ttl --db homelab.db
```

From the REST API:

```bash
curl -s localhost:3030/knot -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "turtle": "@prefix hw: <http://example.org/homelab/> .\nhw:koror a hw:Host ; hw:hostname \"koror.lan\" ."
  }'
```

From an episode (structured agent input):

```bash
curl -s localhost:3030/episode -X POST \
  -H "Content-Type: application/json" \
  -d '{
    "name": "homelab-inventory",
    "nodes": [
      {"name": "koror", "type": "Host", "properties": {"hostname": "koror.lan"}},
      {"name": "traefik", "type": "WebApp"}
    ],
    "edges": [
      {"source": "koror", "target": "traefik", "relation": "runs"}
    ]
  }'
```

## What's Next

- [The Temporal Model](temporal-model.md) — how time-travel works
- [SPARQL from Zero](../tutorials/sparql.md) — querying your triples
- [SHACL Validation](shacl-validation.md) — enforcing structure

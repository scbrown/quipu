# Bounded dictionary Turtle transport

GET /rdf-graph-store?graph=IRI&transport=compact-v1 opts into a complete
named-graph transfer. Ordinary requests keep their existing RDF negotiation.
The optional transport refuses default scope and unsupported versions.

The body is standard Turtle: each distinct IRI is defined as an empty-local-name
prefix, followed by one complete assertion per physical line. Literals retain
their lexical encoding, datatype and language tag; blank-node labels remain.
No properties or facts are removed. The response is limited to 256 MiB of the
actual compact RDF artifact, rather than a compressed/expanded budget exception.

| Header | Meaning |
| --- | --- |
| X-Quipu-RDF-Transport | compact-v1 |
| X-Quipu-Graph-SHA256 | SHA-256 of the requested graph IRI in UTF-8 |
| X-Quipu-Body-SHA256 | SHA-256 of every response body byte |
| X-Quipu-Triples | Unique assertion rows in this response |
| X-Quipu-Actions | Direct schema:Action assertion rows in this response |

Prefix declarations precede assertions. A dictionary key and IRI occur once.
All named terms, including explicit literal datatypes, use dictionary references.
Assertion lines end with a space, period and newline. Counts exclude declarations.

The complete scoped fact collection precedes serialization. Output accumulation
refuses excess bytes; errors do not return a partial successful body. This is an
artifact bound, not a new limit on SQLite collection or whole-process memory.
Unsupported RDF term kinds fail instead of being omitted.

These headers bind the artifact and scope. They do not promise a shared read
transaction across subsequent HTTP requests and do not waive an application's
independent census, shape/profile, build, memory or seeded-control checks.
Release and consumer activation remain separate from this optional capability.

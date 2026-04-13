# OWL Ontology Layer

Quipu supports OWL 2 RL reasoning through a built-in ontology engine. OWL
ontologies define class hierarchies, property characteristics, and constraints
that Quipu enforces at write time and uses to materialize inferred facts.

## Loading an Ontology

Ontologies are OWL axioms expressed in Turtle format. Load one via the CLI or
MCP tool:

```bash
quipu ontology load aegis-ontology ontology.ttl --db quipu.db
```

Or via the MCP `quipu_load_ontology` tool:

```json
{
  "action": "load",
  "name": "aegis-ontology",
  "turtle": "@prefix owl: <http://www.w3.org/2002/07/owl#> ...",
  "timestamp": "2026-04-13T00:00:00Z"
}
```

On load, Quipu:

1. Parses the Turtle and extracts OWL/RDFS axioms
2. Persists the ontology in SQLite (like SHACL shapes)
3. Materializes entailments into the fact log

## Supported Axioms

| Axiom | Effect |
|---|---|
| `rdfs:subClassOf` | Transitive closure: instances of a subclass are also instances of all superclasses |
| `owl:disjointWith` | Write-time validation: rejects an entity typed with two disjoint classes |
| `owl:inverseOf` | Materialization: `(a P b)` produces `(b Q a)` |
| `owl:FunctionalProperty` | Write-time validation: rejects a second value on a functional property |
| `owl:SymmetricProperty` | Materialization: `(a P b)` produces `(b P a)` |
| `owl:equivalentClass` | Materialization: instances of A become instances of B and vice versa |
| `rdfs:domain` / `rdfs:range` | Materialization: infers type from property usage |

## Materialization

Materialized facts are written with `source = "owl:materialize"` so they can
be identified in the transaction log. When an ontology changes, derived facts
can be re-materialized.

```turtle
ex:fido a ex:Dog .
ex:Dog rdfs:subClassOf ex:Mammal .
ex:Mammal rdfs:subClassOf ex:Animal .
```

After materialization, `ASK { ex:fido a ex:Animal }` returns true.

## Write-Time Validation

Two OWL constraints are enforced at write time:

**Disjoint classes**: If `ex:Person owl:disjointWith ex:Robot`, then an entity
cannot be typed as both. Attempting to assert `ex:alice a ex:Robot` when
`ex:alice a ex:Person` already exists returns a structured error.

**Functional properties**: If `ex:ssn a owl:FunctionalProperty`, an entity can
have at most one value. A second assertion is rejected.

## Feature Flag

OWL support is behind the `owl` feature flag:

```bash
cargo build --features owl
cargo test --features owl
```

The `shacl` feature continues to work independently.

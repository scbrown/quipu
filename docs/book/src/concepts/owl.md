# OWL Ontology Layer

Quipu supports OWL 2 RL reasoning through a built-in ontology engine. OWL
ontologies define class hierarchies, property characteristics, and constraints
that Quipu uses to materialize inferred facts, and — when
`owl.validate_on_write` is enabled — enforces at write time.

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
| `owl:disjointWith` | Write-time validation (opt-in): rejects an entity typed with two disjoint classes |
| `rdfs:subPropertyOf` | Materialization: a fact under a subproperty is restated under every superproperty (transitive) |
| `owl:inverseOf` | Materialization: `(a P b)` produces `(b Q a)` |
| `owl:FunctionalProperty` | Write-time validation (opt-in): rejects a second value on a functional property |
| `owl:SymmetricProperty` | Materialization: `(a P b)` produces `(b P a)` |
| `owl:equivalentClass` | Materialization: instances of A become instances of B and vice versa |
| `owl:TransitiveProperty` | Materialization: full closure — `(a P b)`, `(b P c)` produce `(a P c)`, chained to fixpoint |
| `owl:equivalentProperty` | Materialization: facts under either property are restated under the other |
| `rdfs:domain` / `rdfs:range` | Materialization: infers type from property usage |

> `owl:TransitiveProperty` and `owl:equivalentProperty` were parsed and counted
> but **not materialized** before 2026-08-27 — the same silently-dropped shape
> `rdfs:subPropertyOf` had before aegis-qfncf. Loading one reported success and
> derived nothing.

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

> **Enforcement is OPT-IN, and was not wired at all before 2026-08-04.**
> This section previously stated flatly that the two constraints below "are
> enforced at write time". That was FALSE for the shipped server:
> `Ontology::validate()` implemented both and had **no caller** — nothing on the
> write path invoked it, so an ontology could declare a disjointness and every
> violating write was accepted. The caller landed on 2026-08-04.
>
> It is recorded here rather than quietly corrected because the failure mode is
> the doc, not the code: a capability claim in a manual is not tested, it is
> BELIEVED, so it stops the reader checking the very thing that is broken.

Two OWL constraints can be enforced at write time. They are **off by default** —
set `owl.validate_on_write = true` (mirroring `shacl.validate_on_write`), and
build with the `owl` feature.

The default is off on purpose. Axioms may have accumulated in a store while
nothing enforced them, so enabling this can start rejecting writes against a
population that was never checked. **Load the axioms, measure the existing
violations, then enable**. In one real store a single functional-property
candidate had 205 live violations at the moment it would have been declared.

**Disjoint classes**: If `ex:Person owl:disjointWith ex:Robot`, then an entity
cannot be typed as both. Attempting to assert `ex:alice a ex:Robot` when
`ex:alice a ex:Person` already exists returns a structured error and the write
is rolled back.

**Functional properties**: If `ex:ssn a owl:FunctionalProperty`, an entity can
have at most one value. A second, different value is rejected.

Both reject the whole transaction: the constraint runs inside the write's
savepoint, so a violating batch commits nothing. Violations are reported
together rather than one per round-trip.

Constraints are evaluated against the **union of all loaded ontologies**, and a
`load` or `remove` through `POST /ontology` takes effect on the next write.

## Feature Flag

OWL support is behind the `owl` feature flag:

```bash
cargo build --features owl
cargo test --features owl
```

The `shacl` feature continues to work independently.

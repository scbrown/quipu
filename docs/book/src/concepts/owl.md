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
| `owl:sameAs` | Materialization: identity closure (symmetric + transitive), and every fact about one individual is restated about its co-referents. **Subjects and objects only — predicates are not rewritten.** See below |

> `owl:TransitiveProperty` and `owl:equivalentProperty` were parsed and counted
> but **not materialized** before 2026-08-27 — the same silently-dropped shape
> `rdfs:subPropertyOf` had before aegis-qfncf. Loading one reported success and
> derived nothing.

### `owl:sameAs`: identity comes from your DATA, not from the ontology

Every other axiom in the table is read from the ontology document you load.
`owl:sameAs` is **not** — it is read from the graph itself, because identity
between individuals is asserted as ordinary data (the `quipu align` verbs and
`/knot` both write it). You do not declare `owl:sameAs` in an ontology; you
assert it about two things, and materialization picks it up:

```turtle
ex:dolt owl:sameAs ex:doltLan .
ex:dolt ex:hosts   ex:beads .
```

After materialization `ex:doltLan ex:hosts ex:beads` is entailed, and the
identity itself is closed both ways and through chains: with `a sameAs b` and
`b sameAs c`, facts about `a` reach `c`.

> ⚠️ **Predicates are not rewritten.** If you assert `owl:sameAs` between two
> PROPERTIES, the identity itself is closed, but facts are **not** restated
> under the co-referent property — `ex:box ex:hosts ex:svc` with
> `ex:hosts owl:sameAs ex:runs` does **not** entail `ex:box ex:runs ex:svc`.
> This is OWL 2 RL's `eq-rep-p`, and it is not implemented: the rule language
> (`reasoner/ast.rs`) can only put variables in argument position, so a rule
> quantifying over the predicate is not expressible. Use `owl:equivalentProperty`
> instead, which IS materialized and is the right axiom for saying two
> properties mean the same thing. Tracked as the named gap on aegis-yro9m.

Before 2026-09-06 `owl:sameAs` was not implemented at all: assertions were
accepted and stayed completely inert, so a reader landing on one twin never saw
the other's facts (aegis-yro9m, filed after the identity had been asserted 191
times on a live store).

## Materialization

Materialized facts are written with `source = "owl:materialize"` into ROOT's
**companion inferred graph** (`urn:quipu:graph:root#inferred`, quipu-0b6) —
quarantined by placement, composed back in with
`FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred>`. When an
ontology changes, derived facts can be re-materialized.

Materialization runs to **fixpoint across axiom families**: a type introduced
by `rdfs:range` feeds the subclass closure of the next pass, and passes repeat
until one derives nothing new. (Before 2026-08-27 it was one-shot — each
family ran once over base facts, so composed entailments were silently
missing and the recorded workaround was re-encoding OWL axioms as Datalog
rules.) Each pass derives only facts not already present, so re-running
materialization at fixpoint is a no-op and the report counts stay honest.

Materialization can also stay **live**: with `[quipu.owl]
reactive_materialize = true` (requires the `owl` and `reactive-reasoner`
features — release `full` builds have both), the server re-runs
materialization whenever a committed write touches vocabulary the loaded
ontologies mention, so the closure extends as members arrive instead of going
stale after load. Default off: it is a per-write cost a deployment should
choose.

```turtle
ex:fido a ex:Dog .
ex:Dog rdfs:subClassOf ex:Mammal .
ex:Mammal rdfs:subClassOf ex:Animal .
```

After materialization,
`ASK FROM <urn:quipu:graph:root> FROM <urn:quipu:graph:root#inferred>
{ ex:fido a ex:Animal }` returns true — a plain `ASK` does not, because the
entailment lives in the companion, not beside its premises.

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

Two OWL constraints are enforced at write time by default when built with the
`owl` feature. Set `owl.validate_on_write = false` only for an explicitly
informal deployment. Before adopting new axioms, measure the existing graph:
turning an incompatible declaration into live policy can reject future writes
that touch historical drift.

**Disjoint classes**: If `ex:Person owl:disjointWith ex:Robot`, then an entity
cannot be typed as both. Attempting to assert `ex:alice a ex:Robot` when
`ex:alice a ex:Person` already exists returns a structured error and the write
is rolled back.

**Functional properties**: If `ex:ssn a owl:FunctionalProperty`, an entity can
have at most one current value. A later value supersedes the earlier one while
preserving history; two competing values in one batch are rejected because the
write provides no ordering.

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

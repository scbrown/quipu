# Equivalence-aware namespace reads

Status: proposed contract; no implementation or default change in this document.
This specifies a possible read mode after the namespace migration. That migration
continues to use explicit dual reads and is not blocked on this proposal.

## Problem and measured boundary

An isolated restored-store rehearsal on revision `6c35fde2` loaded an explicit
class/property equivalence bridge through the **SHACL shapes registry** with
reactive materialization disabled. Same-namespace class controls returned one
entity in both ROOT and a named graph. Four opposite-namespace class reads
returned zero. Explicit `UNION` plus `DISTINCT` returned one in all four arms.
This establishes behavior under that configuration, not an absence of OWL
support. It does not establish property-query behavior; that needs its own tests.

The current implementation has separate mechanisms:

- `src/store/registry.rs` versions shapes and ontologies separately. A shape
  registration is not an ontology registration.
- `src/sparql/rdfs.rs::collect_class_and_subclasses` follows ROOT
  `rdfs:subClassOf` assertions for the existing constant-type expansion.
- `src/mcp/mod.rs` accepts the existing `entailment: "rdfs"` regime and reports
  constant-type subclass expansion separately through `inference.expandedTypes`.
- [OWL materialization](../book/src/concepts/owl.md) can derive equivalent-class
  and equivalent-property facts. Its companion-graph placement, graph selection,
  load-time writes and reactive settings are different from a read-only view.

A bridge that validates both spellings therefore does not, by itself, establish
that every reader can find facts under the other spelling. The specification
below is deliberately separate from those existing mechanisms.

## Proposed request and compatibility boundary

Reserve a versioned, explicit `alias_read` request object for a future
implementation. This is proposed syntax, not a currently supported API:

```json
{
  "query": "SELECT ?s WHERE { ?s a <https://example.org/new/Directive> }",
  "alias_read": {
    "version": 1,
    "scopes": [{
      "graphs": ["urn:quipu:graph:root", "urn:example:records"],
      "sets": [{
        "name": "namespace-transition",
        "digest": "<sha256 of the selected ontology Turtle bytes>"
      }]
    }]
  }
}
```

Omission preserves existing query behavior byte-for-byte, including existing
inference settings. It does not become an alias-aware default. The CLI, HTTP,
MCP and Wasm surfaces must either honor this contract or explicitly reject the
option; silently ignoring it is a compatibility failure. Capability discovery
must identify version 1 support before a client relies on it.

Version 1 answers from a read-only virtual dataset defined below. It does not
invoke existing implicit subclass expansion or materialization. Combining
`alias_read` with `entailment` or another inference option is rejected until a
composition contract is implemented and tested. A client replacing an existing
subclass-aware query must measure that difference; this mode is not a promised
superset of every existing inferred answer.

Initially support SELECT, ASK and CONSTRUCT with ordinary triple patterns,
FILTER, VALUES, joins, OPTIONAL, UNION, aggregates, DISTINCT, ORDER BY and slices.
Reject DESCRIBE, property-path operators and SERVICE in this mode until their
semantics and resource controls have dedicated tests. Unsupported algebra must
fail before execution, not fall back halfway through a query.

## Which declarations have authority

Only explicitly selected, authorized **ontology registry versions** supply
`owl:equivalentClass` and `owl:equivalentProperty` declarations. Each request
pins its selected set by name and content digest and grants it an explicit list
of data graphs. No implicit union of all loaded ontologies is permitted.

The following do not grant alias-read authority:

- Equivalence-looking triples present only in the SHACL registry.
- Declarations embedded in arbitrary data graphs or imported quarantine data.
- Matching local names, labels, prefixes, suffixes or string similarity.
- `owl:sameAs`, subclass/subproperty axioms, inverse properties, domain/range,
  or SHACL alternative paths.

A registered set is eligible only if the caller may use it and read its declared
application scope. Authorization is checked before parsing or expanding it.
The alias view neither grants access to another graph nor changes graph trust
labels. Unrequested sets cannot influence the answer.

Registration and querying are separate operations. The existing ontology-load
path can materialize facts; registering a bridge through it must not be called
read-only. Before implementation rollout, provide a separately reviewed
registration path that versions the ontology **without materializing it**, or
prove a supported existing path has that property. No caller may simulate this
by writing to the registry database directly. The read mode itself never loads,
removes, materializes or rewrites an ontology.

Named-IRI endpoints only are supported in version 1. Reject selected sets with
anonymous class expressions or malformed equivalence endpoints rather than
silently dropping unsupported members. Other axiom families may coexist in the
set but are ignored by this mode and are named as excluded in its response.

## Virtual dataset semantics

For each graph independently, build two finite equivalence relations from its
selected sets: one for classes and one for properties. Each is reflexive,
symmetric and transitively closed. Cycles terminate; repeating a declaration
or selecting overlapping sets does not multiply proofs. Class and property
roles stay separate even if an IRI is used in both roles.

For each visible base triple `(s, p, o)` in graph `g`:

1. Expose `(s, p2, o)` for every property `p2` equivalent to `p` in `g`.
2. If `p` is exactly `rdf:type` and `o` is a named class IRI, expose
   `(s, rdf:type, c)` for every class `c` equivalent to `o` in `g`.
3. Retain the original triple. Never rewrite subjects, literal values, unrelated
   IRI objects, graph names or stored provenance.

Reject an alias set placing `rdf:type` itself in a nontrivial property
component; otherwise type aliasing and property aliasing would compose through
an unintended route. Expanding a class membership does not invent an OWL
metamodel triple such as `C rdf:type owl:Class`.

The virtual triples are a **set per graph**, keyed by the complete RDF triple,
not by a proof path. Two asserted namespace spellings that yield the same
virtual triple produce one graph match. This is not a global DISTINCT over
result rows: normal SPARQL multiset semantics still apply after graph matching,
projection, UNION and joins. Two different values remain two values. The mode
must not choose a winning value when legacy and replacement properties disagree.
Authorization consumers must still apply their explicit conflict policy.

Variable patterns see this same virtual dataset; optimizers must not make
semantics depend on whether a term was a literal in the query or bound by VALUES.
For example, `?s a ?t` exposes both equivalent type IRIs. Filtering that variable
is **not an asserted-only census in this mode**. Such a census requires a
separate query without alias expansion and with an appropriately selected base
dataset; even then, persisted inferred triples are not original assertions.

CONSTRUCT instantiates its template from these bindings. Its output is a derived
answer, never silently persisted. Projection does not canonicalize namespace
spellings: a bound type or property is the virtual triple's IRI, while subjects
and ordinary object IRIs preserve their stored identity.

## Dataset and time boundaries

ROOT remains the default dataset. A request's `graph`, FROM and FROM NAMED
selection keeps its existing meaning; alias expansion adds no data graph.
`GRAPH <g>` expands only `g`, and `GRAPH ?g` preserves each named graph binding.
Do not consult ROOT facts while evaluating a named graph.

For a default graph made from several FROM graphs, expand each authorized graph
with **its own** selected aliases first, then merge their virtual triples using
normal RDF default-graph union semantics. An axiom authorized for graph A must
not affect a fact whose only source is graph B. If the user explicitly selects
an inferred companion graph, its persisted facts are input like any other
selected facts; this mode neither refreshes it nor certifies its derivations.

Every selected data graph must have an explicit alias scope entry, even if that
entry intentionally selects no sets. An out-of-scope `GRAPH` evaluation is an
error, not a hidden literal fallback. This makes mixed literal/alias answers
visible to the caller. Authorization filtering of accessible graph names remains
upstream of this check.

Capture data and ontology versions at one request snapshot. Access is always
subject to the caller's current authorization; historical grants cannot revive
access that has since been revoked. For transaction-time or valid-time reads,
resolve the versioned
registry at the corresponding historical horizon as well. Reject a current
bridge that was not visible at the requested transaction, even if its asserted
validity is backdated. Refuse ambiguous historical versions, unavailable
registry history, digest mismatch and missing sets; never silently substitute
today's aliases for yesterday's vocabulary.

A pinned digest identifies the parsed bytes, not permission to use retired
policy in a current query. Current requests cannot select a retired version;
authorized historical requests may select it only at a horizon where it was
active. Version replacement or removal affects new requests atomically.
An already-running query completes against its captured snapshot or fails
explicitly; it cannot mix pre-change and post-change alias components.

## Explainability and errors

A successful alias request always includes a separate `aliasRead` envelope on
SELECT, ASK and CONSTRUCT responses. It must name:

- Contract version, `requested: true`, and the normalized data graph scope.
- Selected set names, byte digests and resolved registry version identifiers.
- Effective transaction/valid-time horizon and graph-specific alias scopes.
- Supported axiom families and counts of nontrivial class/property components.
- Whether any selected graph has a nontrivial expansion relation.
- `answerContribution: "not-measured"` unless a separate, bounded comparison
  against the literal dataset actually measured the difference.

The expansion flag says the view was expanded, not that any returned row needed
it. In particular, ASK true may already have been true on stored facts. A query
with no nontrivial components still reports its requested mode, preventing a
silent no-op from being mistaken for implemented inference. Existing
`inference.expandedTypes` must not be reused for property aliases or changed to
imply this contract; it describes a different mechanism.

W3C result formats need equivalent response metadata through a bounded header
and a request-scoped explain identifier, without injecting synthetic result
variables. CLI and Wasm output must expose the same distinctions. Metadata size
limits must return an explicit explain reference or error, never truncate the
list of applied sets without saying so.

Reject unknown versions, malformed scopes, unavailable capability, unsupported
algebra, digest mismatch and unauthorized set/graph access before returning an
answer. Expansion overflow, deadline and intermediate-row limits are errors;
never return a successful partial closure or zero rows that mean exhaustion.

## Resource limits and cache identity

Charge alias parsing, component construction, candidate scans and joins to the
existing query deadline and row budget. Add explicit per-request limits for
selected-set bytes, total terms, component size and generated virtual triples.
Numeric defaults require measured benchmarks before implementation approval.

Cache immutable closure plans by store identity, exact set digests/versions,
contract version, graph scope, temporal horizon and authorization context.
Current-version changes invalidate discovery caches. Never reuse a ROOT-only
or privileged caller's plan for a differently scoped request. The cache is an
optimization: cold and warm answers, metadata and refusals must agree.

## Acceptance matrix

Use separate old, new and unrelated namespaces, unique literal values and two
named graphs in addition to ROOT. All counts below use isolated fixture arms;
no production writes are needed.

| Case | Required result |
|---|---|
| No option | Exact existing behavior and response schema |
| Bridge only in shapes registry | Cannot select it as ontology authority |
| Unselected ontology or data-graph bridge | No alias contribution |
| Old-only/new-only class assertion | Either constant type query finds one subject |
| Old-only/new-only property assertion | Either constant predicate query finds one value |
| Mixed class/property spellings | All four class/property query combinations agree |
| Both spellings, same subject/value | Constant predicate/type match once; COUNT is one |
| Distinct values under the two properties | Both values remain; no silent winner |
| Variable type/property and VALUES binding | Same virtual triples as constant lookup |
| Projection omits variable alias predicate | Preserve normal duplicate projected rows |
| Explicit UNION of two matching branches | Two bag rows; DISTINCT yields one |
| Equivalent class used as ordinary object | Object is unchanged |
| Same suffix, unrelated IRI | Zero accidental matches |
| Chain, cycle, repeated/overlapping sets | Complete finite closure; no proof duplicates |
| Class/property punning | No cross-role closure |
| `rdf:type` property equivalence | Reject unsupported selected set |
| Unsupported anonymous endpoint | Reject; no partial set acceptance |
| ROOT fact, named-graph query | No ROOT leak |
| Named A alias authorization, B facts | No B expansion through A's bridge |
| FROM A + FROM B | Expand per source, then set-union the default graph |
| GRAPH variable, equal triples in A/B | Preserve two distinct graph bindings |
| Unauthorized graph/set | Fail under access contract, no expanded disclosure |
| Absent scope or changed digest | Error, never silent literal fallback |
| Historical query before bridge knowledge | No use of later or backdated bridge |
| Retired/replaced set | Current pin refuses; eligible historical pin works |
| Replacement during query | One snapshot or explicit error |
| ASK true on directly stored fact | Expansion marked; contribution not claimed |
| CONSTRUCT alias output | Correct derived triple set; database unchanged |
| Unsupported path/SERVICE/inference combination | Explicit pre-execution refusal |
| Deadline/component/row overflow | Error; no successful partial answer |
| Warm/cold plan and restart | Identical answers and scope metadata |

For each mode-enabled arm, verify no new transaction, fact, registry version or
materialization event is produced by the read. Compare the explicit dual-read
baseline as a control, and report raw row counts as well as DISTINCT counts.
Benchmark old-only, new-only and mixed populations at increasing component and
graph counts; report parsing time, execution time, peak memory and failures.

## Retirement, rollback and delivery decision

Disabling the request option returns callers to existing behavior and leaves no
alias-read-derived facts to retract. It does **not** undo materialization that
some other path previously performed. Removing a bridge can remove virtual
answers from later reads; retain version history for audit and do not infer
that disappearance authorizes deletion of original facts.

Retiring a namespace requires an inventory of every reader, writer and stored
reference, plus a measured period with no required legacy-only reads. This mode
is an optional compatibility tool, not evidence that the inventory is complete.
Keep the reviewed dual-read migration path until a separate rollout decision
proves all relevant consumers support and request the mode.

Implementation needs separate review of the registry registration path,
virtual dataset evaluator, query metadata/adapters, authorization boundaries
and acceptance/performance results. Do not flip inference defaults, change
writers or seed production aliases as part of landing this specification.

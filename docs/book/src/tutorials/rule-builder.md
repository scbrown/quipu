# The Rule Builder

> "I don't want to write the same SPARQL property path in every query.
> I want the graph to *know* that traefik transitively runs on koror."

You have a knowledge graph full of infrastructure facts. You've been writing
SPARQL queries with `dependsOn+` and `runsOn+` property paths to chase
transitive chains, and it works — but every query re-derives the same
relationships from scratch. You want the graph to materialise those
relationships once, keep them current, and let you query them directly.

This tutorial walks you through writing Datalog rules, running the reasoner,
enabling reactive evaluation, and asking counterfactual "what if?" questions.

## Prerequisites

You need a Quipu store with some infrastructure data. If you followed
[The Homelab Operator](homelab-operator.md), you already have one. If not,
create a quick test store:

```bash
quipu knot - --db lab.db <<'EOF'
@prefix hw: <http://example.org/homelab/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

hw:koror a hw:Host ;
    rdfs:label "koror" .

hw:webproxy a hw:Container ;
    rdfs:label "webproxy" ;
    hw:runsOn hw:koror .

hw:traefik a hw:WebApp ;
    rdfs:label "traefik" ;
    hw:runsOn hw:webproxy ;
    hw:dependsOn hw:pihole .

hw:pihole a hw:Service ;
    rdfs:label "pihole" ;
    hw:runsOn hw:koror .

hw:grafana a hw:WebApp ;
    rdfs:label "grafana" ;
    hw:runsOn hw:webproxy ;
    hw:dependsOn hw:prometheus .

hw:prometheus a hw:Service ;
    rdfs:label "prometheus" ;
    hw:runsOn hw:koror ;
    hw:dependsOn hw:postgres .

hw:postgres a hw:Database ;
    rdfs:label "postgres" ;
    hw:runsOn hw:koror .
EOF
```

## Step 1: Your First Rule — Transitive `runsOn`

Traefik runs on webproxy, and webproxy runs on koror. You want to
materialise the fact that traefik runs on koror without writing a property
path query every time.

Create `my-rules.ttl`:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://example.org/rules/> .

ex:homelab a rule:RuleSet ;
    rule:defaultPrefix "http://example.org/homelab/" .

ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?mid), runsOn(?mid, ?host)" .
```

Let's unpack this:

- **`rule:RuleSet`** with `rule:defaultPrefix` tells the parser that bare
  names like `runsOn` expand to `http://example.org/homelab/runsOn`.
- **`rule:id`** is the provenance tag — every derived fact will carry
  `source = "reasoner:runs_on_transitive"` so you know where it came from.
- **`rule:head`** is what gets derived: `runsOn(?svc, ?host)`.
- **`rule:body`** is the condition: if `?svc` runs on `?mid`, and `?mid`
  runs on `?host`, then `?svc` runs on `?host`.

The shared variable `?mid` is the join key — it's the container in the
middle of the chain.

## Step 2: Run the Reasoner

```bash
quipu reason --rules my-rules.ttl --db lab.db
```

Output:

```text
reasoner: 1 rules across 1 strata — asserted 3, retracted 0

per-rule contributions:
  runs_on_transitive    3
```

Three new facts were derived. The reasoner found that traefik, grafana,
and prometheus all transitively run on koror (via webproxy or directly).

Query the derived facts:

```bash
quipu read "PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?svc ?host WHERE {
  ?svc hw:runsOn ?host .
  ?host a hw:Host .
  ?svc rdfs:label ?label .
}" --db lab.db
```

The transitive `runsOn` edges are now first-class facts — no property paths
needed. Every SPARQL query, every API call, and every agent context lookup
benefits.

## Step 3: Add a Join Rule — Cross-Predicate Derivation

A single-predicate transitive closure is useful, but the real power is
joining across predicates. Add a second rule that derives "this service
is affected by this host going down":

```turtle
ex:affected_by_host a rule:Rule ;
    rule:id "affected_by_host" ;
    rule:head "affectedByHost(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?container), runsOn(?container, ?host)" .
```

Wait — that's the same rule as `runs_on_transitive`. Let's do something
more interesting. Suppose you also track package dependencies:

```bash
quipu knot - --db lab.db <<'EOF'
@prefix hw: <http://example.org/homelab/> .

hw:nginx_pkg a hw:Package ;
    hw:installedIn hw:webproxy .

hw:traefik hw:usesPackage hw:nginx_pkg .
EOF
```

Now add a rule that derives "service S is affected by package P":

```turtle
ex:affected_by_package a rule:Rule ;
    rule:id "affected_by_package" ;
    rule:head "affectedByPackage(?svc, ?pkg)" ;
    rule:body "usesPackage(?svc, ?pkg), installedIn(?pkg, ?container)" .
```

This joins across `usesPackage` and `installedIn` via the shared variable
`?pkg`. The head projects the service and package, dropping the container
(which was only needed for the join condition).

Add both rules to `my-rules.ttl` and run again:

```bash
quipu reason --rules my-rules.ttl --db lab.db
```

## Step 4: Transitive Dependencies

The most common pattern is transitive closure over `dependsOn`. Add:

```turtle
ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .
```

This rule is **positively recursive** — it reads and writes the same
predicate (`dependsOn`). The reasoner handles this correctly: it keeps
applying the rule until no new facts are derived (fixpoint), using
semi-naive evaluation to avoid redundant work.

After running the reasoner, you can find the full transitive blast radius
of any service with a simple flat query:

```bash
quipu read "PREFIX hw: <http://example.org/homelab/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?affected ?label WHERE {
  ?affected hw:dependsOn <http://example.org/homelab/postgres> .
  ?affected rdfs:label ?label .
}" --db lab.db
```

No property paths, no `dependsOn+` — the transitive closure is already
in the store.

## Step 5: Understanding Stratification

Let's look at what happens when you have multiple rules that depend on each
other. Your complete ruleset now has:

1. `depends_on_transitive` — reads and writes `dependsOn`
2. `runs_on_transitive` — reads and writes `runsOn`
3. `affected_by_package` — reads `usesPackage` and `installedIn`, writes
   `affectedByPackage`

The reasoner stratifies these automatically:

- **Stratum 0**: Base facts (`usesPackage`, `installedIn`) — no rules
  produce these, so they're treated as ground truth.
- **Stratum 1**: All three rules. Rules 1 and 2 are self-recursive (they
  read their own output), but there are no cross-rule dependencies, so
  the stratifier groups them together.

The important principle: **positive recursion is fine within a stratum**.
The evaluator runs all rules in a stratum to fixpoint together. What
*isn't* allowed is negation within a cycle — but since none of these
rules use negation, stratification is straightforward.

If you had a rule like:

```turtle
# NOT YET SUPPORTED at eval time, but parsed and stratified
ex:orphan a rule:Rule ;
    rule:id "orphan" ;
    rule:head "orphan(?svc)" ;
    rule:body "service(?svc), not dependsOn(?other, ?svc)" .
```

The stratifier would place `orphan` in a higher stratum than
`depends_on_transitive`, because it needs the complete `dependsOn`
relation (including derived transitive edges) before it can evaluate the
negation. This is stratification at work — it ensures negation only
reads "finished" relations.

## Step 6: Go Reactive

Running `quipu reason` manually is fine for batch processing, but in a
live system you want derived facts to update automatically when base facts
change.

Enable reactive evaluation:

```bash
quipu reason --reactive --rules my-rules.ttl --db lab.db
```

Now the reasoner registers as an observer on the store. Any subsequent
`transact()` call — whether from the CLI, the REST API, an episode
ingestion, or an MCP tool — triggers automatic re-derivation of affected
rules.

Add a new dependency:

```bash
quipu knot - --db lab.db <<'EOF'
@prefix hw: <http://example.org/homelab/> .
hw:grafana hw:dependsOn hw:traefik .
EOF
```

The reactive reasoner:

1. Sees that `dependsOn` changed
2. Finds `depends_on_transitive` uses `dependsOn` in its body
3. Re-evaluates that rule
4. Derives new transitive edges (grafana now transitively depends on
   pihole, via traefik)

All of this happens in the same transaction boundary — by the time the
`knot` command returns, the derived facts are already updated.

### How the Reactive Reasoner Avoids Loops

When the reactive reasoner writes derived facts, those writes are also
transactions. To prevent infinite recursion, the observer checks the
`source` field of every incoming transaction. If it starts with
`"reasoner:"`, the observer skips it. This is simple, correct, and
zero-cost.

## Step 7: Ask "What If?" with Speculate

The most powerful feature of the reasoner is counterfactual reasoning.
Instead of actually removing a host from your graph, you can ask "what
would happen if I removed it?" and get a precise answer.

From the Rust API:

```rust
use quipu::reasoner::{evaluate, parse_rules};
use quipu::store::Store;
use quipu::types::{Datum, Op, Value};

let mut store = Store::open("lab.db")?;
let ruleset = parse_rules(&std::fs::read_to_string("my-rules.ttl")?, None)?;

// Hypothetical: retract all runsOn edges to koror
let koror_id = store.lookup("http://example.org/homelab/koror")?.unwrap();
let runs_on_id = store.lookup("http://example.org/homelab/runsOn")?.unwrap();

let retractions: Vec<Datum> = store
    .current_facts()?
    .iter()
    .filter(|f| f.attribute == runs_on_id && f.value == Value::Ref(koror_id))
    .map(|f| Datum {
        entity: f.entity,
        attribute: f.attribute,
        value: f.value.clone(),
        valid_from: "2026-04-04T00:00:00Z".into(),
        valid_to: None,
        op: Op::Retract,
    })
    .collect();

// Ask "what if?"
let report = store.speculate(&retractions, "2026-04-04T00:00:00Z", |s| {
    evaluate(s, &ruleset, "2026-04-04T00:00:00Z")
})?;

println!("If koror went down: {} facts retracted", report.retracted);
```

The store is **unchanged** after `speculate()` returns. The hypothetical
facts were applied inside a SQLite savepoint and rolled back. You get the
evaluation report without any side effects.

This is ideal for:

- **Pre-change impact assessment**: "What breaks if I decommission this host?"
- **Capacity planning**: "What if I move these containers to a new host?"
- **Incident simulation**: "What's the blast radius of this failure?"

## Step 8: Debugging Rules

### Common Errors and Fixes

**"head variable ?z is not bound in the body"**

Every variable in the head must appear in at least one positive body atom.
If your head mentions `?z`, make sure `?z` appears in a body atom (not
just under negation).

```turtle
# Bad: ?host not in body
rule:head "runsOn(?svc, ?host)" ;
rule:body "service(?svc)" .

# Good: ?host bound by second body atom
rule:head "runsOn(?svc, ?host)" ;
rule:body "service(?svc), assignedTo(?svc, ?host)" .
```

**"two-atom body must share exactly one variable"**

Two-atom join rules need a shared variable for the join key. If your body
atoms have no common variables, there's nothing to join on. If they share
two variables, the evaluator can't determine the join plan.

```turtle
# Bad: no shared variable
rule:body "p(?a, ?b), q(?c, ?d)" .

# Bad: two shared variables
rule:body "p(?a, ?b), q(?a, ?b)" .

# Good: one shared variable (?b is the join key)
rule:body "p(?a, ?b), q(?b, ?c)" .
```

**"body with more than 2 atoms"**

Break the rule into a chain. Instead of:

```turtle
# Not yet supported
rule:head "result(?a, ?d)" ;
rule:body "p(?a, ?b), q(?b, ?c), r(?c, ?d)" .
```

Create an intermediate predicate:

```turtle
ex:step1 a rule:Rule ;
    rule:id "step1" ;
    rule:head "pq(?a, ?c)" ;
    rule:body "p(?a, ?b), q(?b, ?c)" .

ex:step2 a rule:Rule ;
    rule:id "step2" ;
    rule:head "result(?a, ?d)" ;
    rule:body "pq(?a, ?c), r(?c, ?d)" .
```

The stratifier handles the dependency automatically — `step2` evaluates
after `step1` because it reads `pq` which `step1` produces.

**"rule set is not stratifiable: negation cycle through \[...\]"**

Your rules have a cycle through negation. Rule A negates something rule B
produces, and rule B negates something rule A produces. The fix is to
restructure so negation only flows in one direction (from higher strata
to lower ones).

### Inspecting Derived Facts

To see what a specific rule derived, filter by source:

```bash
quipu read "SELECT ?e ?a ?v WHERE { ?e ?a ?v }" --db lab.db \
  | grep "reasoner:runs_on_transitive"
```

To re-run the reasoner and see what changed:

```bash
quipu reason --rules my-rules.ttl --db lab.db
```

If the report shows `asserted 0, retracted 0`, the derived facts are
already up to date — nothing changed since the last run.

## Your Complete Ruleset

Here's the full `my-rules.ttl` from this tutorial:

```turtle
@prefix rule: <http://quipu.local/rule#> .
@prefix ex:   <http://example.org/rules/> .

ex:homelab a rule:RuleSet ;
    rule:defaultPrefix "http://example.org/homelab/" .

# Transitive dependency closure
ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .

# Transitive runsOn closure
ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runsOn(?svc, ?host)" ;
    rule:body "runsOn(?svc, ?mid), runsOn(?mid, ?host)" .

# Package impact: which services are affected by a package?
ex:affected_by_package a rule:Rule ;
    rule:id "affected_by_package" ;
    rule:head "affectedByPackage(?svc, ?pkg)" ;
    rule:body "usesPackage(?svc, ?pkg), installedIn(?pkg, ?container)" .
```

## What's Next

- [The Reasoner](../concepts/reasoning.md) — how it works under the hood
- [Reasoner Reference](../reference/reasoner.md) — complete rule syntax and API
- [Impact Analysis](../recipes/impact-analysis.md) — more patterns for blast radius queries
- [The Homelab Operator](homelab-operator.md) — model your infrastructure from scratch

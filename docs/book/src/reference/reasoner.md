# Reasoner Reference

Complete reference for the Quipu reasoner: rule syntax, CLI, Rust API, error
catalogue, and current limitations.

## Rule Syntax

Rules are written in standard Turtle files using the `rule:` vocabulary. The
reasoner reads any resource typed `rule:Rule` and ignores everything else, so
rules can live alongside SHACL shapes in the same file.

### Namespace

```turtle
@prefix rule: <http://quipu.local/rule#> .
```

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `a rule:Rule` | type | yes | Marks this resource as a rule |
| `rule:id` | string | yes | Stable identifier used in provenance (`source = "reasoner:<id>"`) |
| `rule:head` | string | yes | Head atom: `predicate(?var1, ?var2)` |
| `rule:body` | string | yes | Body atoms: `p(?x, ?y), q(?y, ?z)` |
| `rule:prefix` | string | no | Per-rule IRI prefix for bare predicate names |

### RuleSet Container

An optional `rule:RuleSet` resource sets defaults for all rules in the file:

```turtle
ex:my_rules a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .
```

| Property | Type | Description |
|----------|------|-------------|
| `a rule:RuleSet` | type | Marks this resource as a ruleset |
| `rule:defaultPrefix` | string | Default IRI prefix for all rules in this file |

### Prefix Resolution Order

When the reasoner encounters a bare predicate name like `dependsOn` inside a
head or body string, it resolves the full IRI using this precedence:

1. **Per-rule** `rule:prefix` property (highest priority)
2. **Ruleset** `rule:defaultPrefix` property
3. **Fallback** `http://quipu.local/default/` (lowest priority)

### Atoms and Terms

An atom is `predicate(arg1, arg2)`. Arguments can be:

| Term | Syntax | Example | Notes |
|------|--------|---------|-------|
| Variable | `?name` | `?svc` | Bound by body atoms, projected into head |
| Bare name | `name` | `dependsOn` | Expanded with prefix resolution |
| Full IRI | `<http://...>` | `<http://ex.org/p>` | Used as-is, no expansion |
| String | `"value"` | `"active"` | Allowed in head only (constants) |

### Body Syntax

Body atoms are comma-separated. Whitespace is flexible:

```text
dependsOn(?a, ?b), dependsOn(?b, ?c)
```

Negation uses the `not` keyword:

```text
reachable(?x, ?y), not blocked(?y)
```

Negation is parsed and stratified but **not yet evaluated** — the evaluator
will reject rules with negated body atoms with a clear error message. This
is reserved for a future phase.

### Supported Rule Shapes

The evaluator currently supports two rule shapes:

#### Single-atom projection

One body atom. The head projects a subset of its variables.

```turtle
rule:head "output(?x, ?y)" ;
rule:body "input(?x, ?y)" .
```

Constraints:

- Head must have exactly 2 arguments
- Body atom must have exactly 2 arguments
- No repeated variables within a body atom

#### Two-atom join

Two body atoms sharing exactly one variable (the join key).

```turtle
rule:head "affects(?pkg, ?svc)" ;
rule:body "installedIn(?pkg, ?c), runsService(?c, ?svc)" .
```

Constraints:

- Head must have exactly 2 arguments
- Each body atom must have exactly 2 arguments
- Body atoms must share exactly one variable
- No repeated variables within a single body atom
- No constants in body atoms

#### What's not yet supported

- Body with 3+ atoms
- Negation-as-failure in body
- Non-binary atoms (1 or 3+ arguments)
- Constants in body atom positions
- Repeated variables within a body atom (e.g., `p(?x, ?x)`)

These shapes are parsed and stratified correctly — only evaluation rejects
them. Future phases will extend compilation without changing the rule DSL.

### Complete Example

```turtle
@prefix rule:  <http://quipu.local/rule#> .
@prefix ex:    <http://aegis.gastown.local/rules/> .
@prefix aegis: <http://aegis.gastown.local/ontology/> .

ex:aegis a rule:RuleSet ;
    rule:defaultPrefix "http://aegis.gastown.local/ontology/" .

# Transitive dependency closure (positive recursion)
ex:depends_on_transitive a rule:Rule ;
    rule:id "depends_on_transitive" ;
    rule:head "depends_on(?a, ?c)" ;
    rule:body "depends_on(?a, ?b), depends_on(?b, ?c)" .

# Host-level runs_on closure
ex:runs_on_transitive a rule:Rule ;
    rule:id "runs_on_transitive" ;
    rule:head "runs_on(?svc, ?host)" ;
    rule:body "runs_on(?svc, ?container), runs_on(?container, ?host)" .
```

### Safety Checks

The parser enforces these safety properties at load time:

- **Range restriction**: every variable in the head must appear in at least
  one positive body atom. A head variable that only appears under negation
  is rejected.
- **Non-empty body**: rules with empty bodies are rejected.
- **Valid syntax**: malformed head or body strings produce errors that name
  the specific rule and the parse location.

---

## CLI: `quipu reason`

Run the reasoner against a store.

```bash
quipu reason [--rules <file.ttl>] [--reactive] [--db <path>]
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--rules <file>` | `shapes/aegis-rules.ttl` | Path to Turtle file containing rules |
| `--reactive` | off | Register a `ReactiveReasoner` observer after evaluation (requires `reactive-reasoner` feature) |
| `--db <path>` | config default | Store database path |

### Output

```text
reasoner: 2 rules across 1 strata — asserted 5, retracted 0

per-rule contributions:
  depends_on_transitive    3
  runs_on_transitive       2
```

The report shows:

- Total rules and strata evaluated
- Aggregate asserted/retracted counts
- Per-rule breakdown of new assertions

### Examples

Run with the default aegis rules:

```bash
quipu reason --db homelab.db
```

Run with custom rules:

```bash
quipu reason --rules my-rules.ttl --db homelab.db
```

Run and keep derived facts fresh going forward:

```bash
quipu reason --reactive --db homelab.db
```

---

## Rust API

### Parsing

```rust
use quipu::reasoner::{parse_rules, RuleSet};

let turtle = std::fs::read_to_string("rules.ttl")?;
let ruleset: RuleSet = parse_rules(&turtle, None)?;
// Or with a fallback prefix:
let ruleset = parse_rules(&turtle, Some("http://my.org/"))?;
```

`RuleSet` contains:

- `rules: Vec<Rule>` — rules in source order
- `default_prefix: String` — resolved default prefix

### Evaluation

```rust
use quipu::reasoner::{evaluate, EvalReport};
use quipu::store::Store;

let mut store = Store::open("homelab.db")?;
let report: EvalReport = evaluate(&mut store, &ruleset, "2026-04-04T12:00:00Z")?;

println!("asserted: {}, retracted: {}", report.asserted, report.retracted);
for (rule_id, count) in &report.per_rule {
    println!("  {}: {}", rule_id, count);
}
```

`EvalReport` fields:

| Field | Type | Description |
|-------|------|-------------|
| `asserted` | `usize` | Total new derived facts |
| `retracted` | `usize` | Total retracted derived facts |
| `strata_run` | `usize` | Number of non-empty strata executed |
| `per_rule` | `Vec<(String, usize)>` | Per-rule assertion counts |

### Reactive Evaluation

Requires the `reactive-reasoner` feature.

```rust
use quipu::reasoner::reactive::ReactiveReasoner;
use std::sync::Arc;

let observer = Arc::new(ReactiveReasoner::new(ruleset));
store.add_observer(observer.clone());

// Now any transact() call triggers automatic re-derivation.
store.transact(&new_facts, timestamp, Some("agent"), Some("discovery"))?;
// Derived facts are already updated.

// Check stats:
let stats = observer.stats();
println!("triggers: {}, asserted: {}", stats.triggers, stats.total_asserted);
```

`ReactiveStats` fields:

| Field | Type | Description |
|-------|------|-------------|
| `triggers` | `usize` | Number of times the observer fired |
| `total_asserted` | `usize` | Cumulative assertions across all triggers |
| `total_retracted` | `usize` | Cumulative retractions across all triggers |

The reactive reasoner:

- Skips transactions with `source` starting with `"reasoner:"` (prevents loops)
- Computes the transitive closure of affected rules via dependency analysis
- Re-evaluates only the affected strata, not the entire ruleset

### Speculate

```rust
let result = store.speculate(&hypothetical_datums, timestamp, |store| {
    evaluate(store, &ruleset, timestamp)
})?;
// result is the EvalReport from inside the closure
// store is unchanged — the hypothetical was rolled back
```

The closure receives a `&Store` with the hypothetical facts applied. When
the closure returns, all changes are rolled back via `ROLLBACK TO SAVEPOINT`.

### Core Types

```rust
// A term in an atom's argument list
pub enum Term {
    Var(String),    // Variable (without leading ?)
    Iri(String),    // Full IRI
    Str(String),    // String literal
}

// A predicate application: pred(arg1, arg2)
pub struct Atom {
    pub predicate: String,  // Full IRI after expansion
    pub args: Vec<Term>,
}

// A body literal
pub enum BodyAtom {
    Positive(Atom),
    Negative(Atom),  // Parsed but not yet evaluated
}

// A Horn clause rule
pub struct Rule {
    pub id: String,         // Provenance identifier
    pub head: Atom,
    pub body: Vec<BodyAtom>,
}
```

---

## Error Reference

All errors are variants of `ReasonerError`.

### `Turtle`

```text
rule Turtle parse error: <details>
```

The Turtle file itself failed to parse as valid RDF. Check for unclosed
strings, missing prefixes, or invalid IRIs.

### `MissingProperty`

```text
rule "R1" is missing required property head
```

A resource typed `rule:Rule` lacks a required property. Every rule needs
`rule:id`, `rule:head`, and `rule:body`.

### `BadSyntax`

```text
rule "R1" head: expected 'predicate(args)' but got 'foo bar'
```

A head or body string couldn't be parsed as atoms. Check for missing
parentheses, unbalanced commas, or invalid variable syntax.

### `UnboundHeadVariable`

```text
rule "R1" head variable ?z is not bound in the body
```

The head references a variable that doesn't appear in any positive body
atom. Every head variable must be range-restricted — it must appear in at
least one positive body literal so the reasoner knows what values to bind.

### `UnstratifiableCycle`

```text
rule set is not stratifiable: negation cycle through ["p", "q"]
```

The ruleset contains a cycle through negation: rule A negates predicate P
which rule B produces, and rule B negates predicate Q which rule A produces
(or a self-negation like `p :- not p`). Break the cycle by restructuring
your rules so negation only flows "downward" between strata.

### `Unsupported`

```text
rule "R1" uses unsupported feature: body with more than 2 atoms
```

The rule parsed and stratified successfully, but uses a shape the evaluator
doesn't yet handle. Current unsupported features:

| Feature | Message |
|---------|---------|
| 3+ body atoms | `body with more than 2 atoms` |
| Negation in body | `negation-as-failure` |
| Non-binary atoms | `non-binary head/body atom` |
| Constants in body | `constant argument in body atom` |
| Self-join variable | `repeated variable in a body atom` |
| Missing join key | `two-atom body must share exactly one variable` |
| Unknown head IRI | `head references an IRI that has never been interned` |
| String in head | `string constant in head atom` |

### `Store`

```text
store error: <sqlite error details>
```

A store operation failed during evaluation (reading facts, writing
derivations). This typically indicates a database problem, not a rule
problem.

---

## Provenance Tags

Every derived fact is written with structured provenance:

| Field | Value |
|-------|-------|
| `source` | `reasoner:<rule-id>` (e.g., `reasoner:depends_on_transitive`) |
| `actor` | `reasoner` |

You can query or filter by these tags. To find all facts derived by a
specific rule:

```bash
quipu read "SELECT ?e ?a ?v WHERE {
  ?e ?a ?v .
}" --db homelab.db | grep "reasoner:depends_on_transitive"
```

Or from Rust, filter the `source` field on returned `Fact` structs.

---

## Limitations

**Binary predicates only.** Head and body atoms must have exactly 2
arguments. This covers the vast majority of RDF-style relations
(`subject predicate object`) but can't express ternary or higher-arity
relations directly.

**Two-atom body maximum.** Join rules can combine at most two body atoms.
Longer joins can be decomposed into chains of two-atom rules with
intermediate predicates.

**No negation at evaluation time.** Negated body atoms are parsed and
stratified correctly, but the evaluator rejects them. Stratification
analysis is complete — when negation support lands, it will work without
changes to existing rules.

**Full re-derivation.** Each evaluation pass re-derives all facts for every
rule in the affected strata, then diffs against the old state. This is
correct and fast at the target scale (~50K facts), but would need
incremental truth maintenance for much larger workloads.

**No aggregation.** There's no COUNT, SUM, MIN/MAX in rules. Use SPARQL
for aggregation over derived facts.

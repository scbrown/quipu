# SHACL Validation

Quipu enforces strict schema at write time via
[SHACL](https://www.w3.org/TR/shacl/) (Shapes Constraint Language),
powered by [rudof](https://github.com/rudof-project/rudof).

## How It Works

1. Define SHACL shapes in Turtle format
2. Create a `Validator` from those shapes
3. Validate proposed data before writing to the fact log
4. Get structured feedback on failures

```rust
use quipu::{Validator, validate_shapes};

let shapes = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
    ] .
"#;

let data = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;

let feedback = validate_shapes(shapes, data).unwrap();
assert!(feedback.conforms);  // true -- data is valid
```

## Agent-Friendly Feedback

When validation fails, the `ValidationFeedback` struct provides structured
details that agents can act on:

```rust
if !feedback.conforms {
    for issue in &feedback.results {
        println!("Severity: {}", issue.severity);
        println!("Focus node: {}", issue.focus_node);
        println!("Component: {}", issue.component);
        if let Some(path) = &issue.path {
            println!("Path: {}", path);
        }
        if let Some(msg) = &issue.message {
            println!("Message: {}", msg);
        }
    }
}
```

This is the core of Quipu's "strict but helpful" philosophy -- validation
doesn't just reject, it tells the agent exactly what's wrong and where.

## Supported Constraints

Through rudof, Quipu supports the full SHACL Core specification:

- **Cardinality**: `sh:minCount`, `sh:maxCount`
- **Value type**: `sh:datatype`, `sh:class`, `sh:nodeKind`
- **Value range**: `sh:minInclusive`, `sh:maxInclusive`, `sh:minExclusive`, `sh:maxExclusive`
- **String**: `sh:minLength`, `sh:maxLength`, `sh:pattern`
- **Property pair**: `sh:equals`, `sh:disjoint`, `sh:lessThan`
- **Logical**: `sh:and`, `sh:or`, `sh:not`, `sh:xone`
- **Shape-based**: `sh:node`, `sh:property`, `sh:qualifiedValueShape`
- **Other**: `sh:closed`, `sh:ignoredProperties`, `sh:hasValue`, `sh:in`

## Reusable Validators

Create a `Validator` once and validate multiple data payloads:

```rust
let validator = Validator::from_turtle(shapes)?;

// Validate multiple payloads
let result1 = validator.validate(data1.as_bytes())?;
let result2 = validator.validate(data2.as_bytes())?;

// Or use the convenience reject method
validator.validate_or_reject(data.as_bytes())?;
```

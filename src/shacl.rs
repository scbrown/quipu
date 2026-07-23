//! SHACL write-time validation via rudof.
//!
//! Loads SHACL shapes from Turtle and validates proposed RDF data against them
//! before allowing it into the fact log. Returns structured feedback on failures
//! so agents can fix and retry.

use std::io::Read;

use rudof_lib::{
    RDFFormat, ReaderMode, Rudof, RudofConfig, ShaclFormat, ShaclValidationMode, ShapesGraphSource,
};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::resolution::EntityCandidate;

/// Structured feedback from SHACL validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationFeedback {
    /// Whether the data conforms to the shapes.
    pub conforms: bool,
    /// Number of violations found.
    pub violations: usize,
    /// Number of warnings found.
    pub warnings: usize,
    /// Individual violation/warning details.
    pub results: Vec<ValidationIssue>,
    /// Entity resolution candidates — present when resolution is enabled and
    /// near-duplicate entities were detected during write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_candidates: Option<Vec<EntityCandidate>>,
}

/// A single validation issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Severity: "violation", "warning", or "info".
    pub severity: String,
    /// The focus node that failed validation.
    pub focus_node: String,
    /// The SHACL component that triggered the issue.
    pub component: String,
    /// The property path involved (if any).
    pub path: Option<String>,
    /// The offending value (if any).
    pub value: Option<String>,
    /// The source shape (if any).
    pub source_shape: Option<String>,
    /// Human-readable message.
    pub message: Option<String>,
}

/// A SHACL validator that holds loaded shapes and validates data against them.
pub struct Validator {
    shapes_turtle: String,
}

impl Validator {
    /// Create a new validator from SHACL shapes in Turtle format.
    pub fn from_turtle(shapes: &str) -> Result<Self> {
        // Verify the shapes parse by loading them into rudof.
        let mut rudof = RudofConfig::default_config()
            .map_err(|e| Error::InvalidValue(format!("rudof config error: {e}")))
            .and_then(|cfg| {
                Rudof::new(&cfg).map_err(|e| Error::InvalidValue(format!("rudof init error: {e}")))
            })?;
        let mut reader = shapes.as_bytes();
        rudof
            .read_shacl(
                &mut reader,
                "shapes",
                Some(&ShaclFormat::Turtle),
                None,
                None,
            )
            .map_err(|e| Error::InvalidValue(format!("SHACL parse error: {e}")))?;
        Ok(Self {
            shapes_turtle: shapes.to_string(),
        })
    }

    /// Load shapes from a reader.
    pub fn from_reader(mut reader: impl Read) -> Result<Self> {
        let mut shapes = String::new();
        reader
            .read_to_string(&mut shapes)
            .map_err(|e| Error::InvalidValue(format!("read error: {e}")))?;
        Self::from_turtle(&shapes)
    }

    /// Validate RDF data (as Turtle bytes) against the loaded shapes.
    ///
    /// Returns structured feedback. If `conforms` is true, the data is valid.
    pub fn validate(&self, data: &[u8]) -> Result<ValidationFeedback> {
        let mut rudof = RudofConfig::default_config()
            .map_err(|e| Error::InvalidValue(format!("rudof config error: {e}")))
            .and_then(|cfg| {
                Rudof::new(&cfg).map_err(|e| Error::InvalidValue(format!("rudof init error: {e}")))
            })?;

        // Load shapes.
        let mut shapes_reader = self.shapes_turtle.as_bytes();
        rudof
            .read_shacl(
                &mut shapes_reader,
                "shapes",
                Some(&ShaclFormat::Turtle),
                None,
                None,
            )
            .map_err(|e| Error::InvalidValue(format!("SHACL load error: {e}")))?;

        // Load data.
        let mut data_reader = data;
        rudof
            .read_data(
                &mut data_reader,
                "data",
                Some(&RDFFormat::Turtle),
                None,
                Some(&ReaderMode::Lax),
                None,
            )
            .map_err(|e| Error::InvalidValue(format!("data load error: {e}")))?;

        // Validate.
        let report = rudof
            .validate_shacl(
                Some(&ShaclValidationMode::Native),
                Some(&ShapesGraphSource::CurrentSchema),
            )
            .map_err(|e| Error::InvalidValue(format!("SHACL validation error: {e}")))?;

        let mut issues = Vec::new();
        for result in report.results() {
            issues.push(ValidationIssue {
                severity: format!("{:?}", result.severity()),
                focus_node: format!("{}", result.focus_node()),
                component: format!("{}", result.component()),
                path: result.path().map(|p| format!("{p}")),
                value: result.value().map(|v| format!("{v}")),
                source_shape: result.source().map(|s| format!("{s}")),
                message: result.message().map(std::string::ToString::to_string),
            });
        }

        Ok(ValidationFeedback {
            conforms: report.conforms(),
            violations: report.count_violations(),
            warnings: report.count_warnings(),
            results: issues,
            resolution_candidates: None,
        })
    }

    /// Validate proposed Turtle data and return Ok(()) if valid, or Err with
    /// the first violation message if not.
    pub fn validate_or_reject(&self, data: &[u8]) -> Result<()> {
        let feedback = self.validate(data)?;
        if feedback.conforms {
            Ok(())
        } else {
            let msg = feedback.results.first().map_or_else(
                || "SHACL validation failed".to_string(),
                |r| {
                    format!(
                        "SHACL violation on {}: {} (component: {}{})",
                        r.focus_node,
                        r.message.as_deref().unwrap_or("constraint violated"),
                        r.component,
                        r.path
                            .as_ref()
                            .map(|p| format!(", path: {p}"))
                            .unwrap_or_default()
                    )
                },
            );
            Err(Error::InvalidValue(format!(
                "{msg}. Hint: propose a schema change via quipu_propose_schema_change"
            )))
        }
    }
}

/// Convenience: validate proposed data against shapes, both as Turtle strings.
///
/// Returns structured feedback for agent consumption.
pub fn validate_shapes(shapes_turtle: &str, data_turtle: &str) -> Result<ValidationFeedback> {
    let validator = Validator::from_turtle(shapes_turtle)?;
    validator.validate(data_turtle.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERSON_SHAPE: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:property [
        sh:path ex:name ;
        sh:datatype xsd:string ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
    ] ;
    sh:property [
        sh:path ex:age ;
        sh:datatype xsd:integer ;
        sh:minCount 0 ;
        sh:maxCount 1 ;
        sh:minInclusive 0 ;
        sh:maxInclusive 200 ;
    ] .
"#;

    #[test]
    fn valid_data_passes() {
        let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer .
"#;
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(feedback.conforms, "expected valid data to conform");
        assert_eq!(feedback.violations, 0);
    }

    #[test]
    fn missing_required_property_fails() {
        let data = r#"
@prefix ex: <http://example.org/> .

ex:alice a ex:Person .
"#;
        // Missing ex:name which has sh:minCount 1
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(!feedback.conforms, "expected missing name to fail");
        assert!(feedback.violations > 0);
        assert!(!feedback.results.is_empty());
    }

    #[test]
    fn wrong_datatype_fails() {
        let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "not-a-number" .
"#;
        // age should be xsd:integer, but "not-a-number" is xsd:string
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(!feedback.conforms, "expected wrong datatype to fail");
        assert!(feedback.violations > 0);
    }

    #[test]
    fn value_out_of_range_fails() {
        let data = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "300"^^xsd:integer .
"#;
        // age 300 exceeds sh:maxInclusive 200
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(!feedback.conforms, "expected out-of-range to fail");
    }

    #[test]
    fn too_many_values_fails() {
        let data = r#"
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:name "Also Alice" .
"#;
        // Two names but sh:maxCount is 1
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(!feedback.conforms, "expected too many names to fail");
    }

    #[test]
    fn validator_reuse() {
        let validator = Validator::from_turtle(PERSON_SHAPE).unwrap();

        let good = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person ; ex:name "Alice" .
"#;
        assert!(validator.validate(good.as_bytes()).unwrap().conforms);

        let bad = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
        assert!(!validator.validate(bad.as_bytes()).unwrap().conforms);
    }

    #[test]
    fn validate_or_reject_returns_error() {
        let validator = Validator::from_turtle(PERSON_SHAPE).unwrap();

        let bad = r#"
@prefix ex: <http://example.org/> .
ex:bob a ex:Person .
"#;
        let err = validator.validate_or_reject(bad.as_bytes()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("SHACL violation"), "got: {msg}");
    }

    #[test]
    fn feedback_has_structured_details() {
        let data = r#"
@prefix ex: <http://example.org/> .
ex:alice a ex:Person .
"#;
        let feedback = validate_shapes(PERSON_SHAPE, data).unwrap();
        assert!(!feedback.conforms);

        let issue = &feedback.results[0];
        assert!(!issue.focus_node.is_empty());
        assert!(!issue.component.is_empty());
        assert!(!issue.severity.is_empty());
    }
}

/// The result of routing a combined shapes document by `quipu:onViolation`.
///
/// Event-based P3 (design §5/§7): a shape annotated `quipu:onViolation
/// "emit"` observes violations as `shacl.violation` events WITHOUT gating the
/// write; every other shape (unannotated, or annotated `"reject"`) keeps the
/// hard-gate semantics. DEFAULT REJECT is decided (design §9.3) — a shape opts
/// INTO emit, never out of reject by omission.
#[derive(Debug, Clone)]
pub struct ShapesSplit {
    /// Shapes that gate the transaction (hard reject on violation).
    pub reject: String,
    /// Shapes whose violations become events; empty when nothing is annotated.
    pub emit: String,
    /// Whether any emit-annotated shape was found (spares a validator run).
    pub has_emit: bool,
}

/// Split a Turtle shapes document into reject-mode and emit-mode documents by
/// the `quipu:onViolation` annotation on each top-level shape block.
///
/// TEXTUAL, BY DESIGN, WITH ITS LIMITS STATED: this splits on the house style
/// used by every shape set this deployment loads — top-level statements of the
/// form `<subject> a sh:NodeShape ; … .` with inline `[ … ]` property shapes,
/// where a statement ends with `.` at bracket depth 0 at end-of-line. It does
/// not implement a full Turtle parser; a shapes document in a different style
/// routes conservatively (unrecognised content stays in the REJECT document,
/// which preserves the default-reject contract — never the other way around).
/// Prefix lines are replicated into both documents so each stands alone; the
/// emit document must declare the `quipu:` prefix itself, exactly as it must
/// declare `sh:`.
pub fn split_shapes_by_policy(shapes_turtle: &str) -> ShapesSplit {
    let mut prefixes = String::new();
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;

    for line in shapes_turtle.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("@prefix") || trimmed.to_lowercase().starts_with("prefix ") {
            prefixes.push_str(line);
            prefixes.push('\n');
            continue;
        }
        // Comments and blank lines between blocks belong to no block.
        if current.is_empty() && (trimmed.is_empty() || trimmed.starts_with('#')) {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        // Track bracket depth outside comments (house style has no strings
        // containing brackets; a `#` starts a comment for the rest of the line).
        let code = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        for ch in code.chars() {
            match ch {
                '[' | '(' => depth += 1,
                ']' | ')' => depth -= 1,
                _ => {}
            }
        }
        if depth == 0 && code.trim_end().ends_with('.') {
            blocks.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        // Unterminated trailing content: conservative — reject document.
        blocks.push(current);
    }

    let mut reject = prefixes.clone();
    let mut emit = prefixes;
    let mut has_emit = false;
    for block in blocks {
        let is_emit = block.contains("quipu:onViolation")
            && block
                .split("quipu:onViolation")
                .nth(1)
                .is_some_and(|rest| rest.trim_start().starts_with("\"emit\""));
        if is_emit {
            emit.push_str(&block);
            emit.push('\n');
            has_emit = true;
        } else {
            reject.push_str(&block);
            reject.push('\n');
        }
    }
    ShapesSplit {
        reject,
        emit,
        has_emit,
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    const SHAPES: &str = r#"@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix quipu: <http://quipu.dev/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

aegis:HardShape a sh:NodeShape ;
    sh:targetClass aegis:Thing ;
    sh:property [ sh:path rdfs:label ; sh:minCount 1 ] .

aegis:SoftShape a sh:NodeShape ;
    quipu:onViolation "emit" ;
    sh:targetClass aegis:Widget ;
    sh:property [ sh:path aegis:size ; sh:minCount 1 ] .
"#;

    #[test]
    fn routes_emit_annotated_block_and_keeps_default_reject() {
        let split = split_shapes_by_policy(SHAPES);
        assert!(split.has_emit);
        assert!(split.reject.contains("HardShape"));
        assert!(!split.reject.contains("SoftShape"));
        assert!(split.emit.contains("SoftShape"));
        assert!(!split.emit.contains("HardShape"));
        // Both documents carry the prefixes and PARSE as shapes.
        for doc in [&split.reject, &split.emit] {
            assert!(doc.contains("@prefix sh:"));
            validate_shapes(doc, "@prefix ex: <http://example.org/> .\n").unwrap();
        }
    }

    #[test]
    fn explicit_reject_annotation_stays_reject() {
        let shapes = SHAPES.replace("\"emit\"", "\"reject\"");
        let split = split_shapes_by_policy(&shapes);
        assert!(!split.has_emit);
        assert!(split.reject.contains("SoftShape"));
    }

    #[test]
    fn unannotated_document_is_pure_reject_and_byte_stable_semantics() {
        let shapes = SHAPES
            .lines()
            .filter(|l| !l.contains("quipu:onViolation"))
            .collect::<Vec<_>>()
            .join("\n");
        let split = split_shapes_by_policy(&shapes);
        assert!(!split.has_emit);
        assert!(split.reject.contains("HardShape") && split.reject.contains("SoftShape"));
        assert!(split.emit.trim_end().ends_with('.') || !split.emit.contains("Shape"));
    }
}

#[cfg(test)]
#[path = "code_entity_tests.rs"]
mod code_entity_tests;

#[cfg(test)]
#[path = "governance_tests.rs"]
mod governance_tests;

//! SHACL write-time validation via rudof.
//!
//! Loads SHACL shapes from Turtle and validates proposed RDF data against them
//! before allowing it into the fact log. Returns structured feedback on failures
//! so agents can fix and retry.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_core::term::Object;
use rudof_rdf::rdf_impl::{OxigraphInMemory, ReaderMode};
use shacl_engine::ir::IRSchema;
use shacl_engine::validator::ShaclValidationMode;
use shacl_engine::validator::processor::{GraphValidation, ShaclProcessor};
use shacl_engine::validator::store::{Graph, ShaclDataManager};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::resolution::EntityCandidate;

fn report_term(term: &Object) -> String {
    match term {
        Object::Literal(literal) => literal.lexical_form(),
        _ => term.to_string(),
    }
}

fn single_shape_message(shapes: &str) -> Option<String> {
    let pattern = regex::Regex::new(r#"sh:message\s+(\"(?:\\.|[^\"])*\")"#).ok()?;
    let mut matches = pattern.captures_iter(shapes);
    let first = matches.next()?.get(1)?.as_str();
    if matches.next().is_some() {
        return None;
    }
    serde_json::from_str(first).ok()
}

fn report_severity(severity: &shacl_engine::types::Severity) -> String {
    match severity {
        shacl_engine::types::Severity::Generic(iri) => iri.to_string(),
        other => other.to_string(),
    }
}

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
///
/// The parsed shapes graph is retained (behind a `Mutex`, because rudof needs
/// `&mut` to validate) and REUSED across `validate` calls. Previously this type
/// kept only the shapes *string*: `from_turtle` parsed the shapes and
/// threw the parse away, then every `validate` re-parsed them. With the real
/// aegis shape sets (86 KB) that was ~19 ms of parsing done twice on every
/// single write, and it dominated validation cost — see `examples/shacl_cost.rs`.
pub struct Validator {
    shapes_turtle: String,
    /// Compiled shapes schema, reused across validation calls.
    schema: Mutex<IRSchema>,
}

impl Validator {
    /// Create a new validator from SHACL shapes in Turtle format.
    pub fn from_turtle(shapes: &str) -> Result<Self> {
        let mut reader = std::io::BufReader::new(shapes.as_bytes());
        let schema = ShaclDataManager::load(
            &mut reader,
            "shapes",
            &RDFFormat::Turtle,
            Some("http://quipu.local/shapes"),
        )
        .map_err(|e| Error::InvalidValue(format!("SHACL parse error: {e}")))?;
        Ok(Self {
            shapes_turtle: shapes.to_string(),
            schema: Mutex::new(schema),
        })
    }

    /// The shapes this validator was built from.
    #[must_use]
    pub fn shapes_turtle(&self) -> &str {
        &self.shapes_turtle
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
        // Reuse the already-parsed shapes graph. `read_data` below is called
        // with merge=None (=false), which replaces the RDF data outright, so a
        // reused instance carries no data from a previous validation.
        let schema = self
            .schema
            .lock()
            .map_err(|e| Error::InvalidValue(format!("shapes lock poisoned: {e}")))?;

        let mut data_reader = std::io::BufReader::new(data);
        let data = OxigraphInMemory::from_reader(
            &mut data_reader,
            "data",
            &RDFFormat::Turtle,
            Some("http://quipu.local/data"),
            &ReaderMode::Lax,
        )
        .map_err(|e| Error::InvalidValue(format!("data load error: {e}")))?;

        let graph = Graph::try_from(data)
            .map_err(|e| Error::InvalidValue(format!("data graph error: {e}")))?;
        let mut validator = GraphValidation::new(graph);
        let report =
            ShaclProcessor::validate(&mut validator, &schema, &ShaclValidationMode::Native)
                .map_err(|e| Error::InvalidValue(format!("SHACL validation error: {e}")))?;

        let mut issues = Vec::new();
        let shape_message = single_shape_message(&self.shapes_turtle);
        for result in report.results() {
            let focus_node = report_term(result.focus_node());
            let component = report_term(result.constraint_component());
            let value = result.value().map(report_term).or_else(|| {
                component
                    .ends_with("#HasValueConstraintComponent")
                    .then(|| focus_node.clone())
            });
            issues.push(ValidationIssue {
                severity: report_severity(result.severity()),
                focus_node,
                component,
                path: result.path().map(|p| format!("{p}")),
                value,
                source_shape: result.source().map(report_term),
                message: shape_message
                    .clone()
                    .or_else(|| result.message().get(None).cloned()),
            });
        }

        Ok(ValidationFeedback {
            conforms: report.conforms(),
            violations: report.get_count_of(&shacl_engine::types::Severity::Violation),
            warnings: report.get_count_of(&shacl_engine::types::Severity::Warning),
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

/// Process-wide cache of parsed shape graphs, keyed by a hash of the shapes
/// Turtle. The stored shapes change rarely (a `/shapes` load) while writes are
/// continuous, so without this every write re-parses the same 86 KB of shapes.
///
/// Bounded: the key space is "distinct shape sets in use", normally 1. The cap
/// exists so a caller passing per-request inline shapes (`/knot` accepts them)
/// cannot grow this without limit.
type ValidatorCache = Mutex<HashMap<u64, Arc<Validator>>>;

const VALIDATOR_CACHE_CAP: usize = 16;

fn validator_cache() -> &'static ValidatorCache {
    static CACHE: OnceLock<ValidatorCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shapes_key(shapes_turtle: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    shapes_turtle.hash(&mut hasher);
    hasher.finish()
}

/// Get a validator for these shapes, parsing them only on a cache miss.
///
/// On a hash collision between two *different* shape sets the cached validator
/// would be wrong, so the stored shapes are compared before reuse.
pub fn cached_validator(shapes_turtle: &str) -> Result<Arc<Validator>> {
    cached_validator_in(validator_cache(), shapes_turtle)
}

/// The lookup behind `cached_validator`, with the cache as a parameter so
/// tests can exercise the caching contract against a private cache instead of
/// the process-global one (which parallel tests evict from at will).
fn cached_validator_in(cache: &ValidatorCache, shapes_turtle: &str) -> Result<Arc<Validator>> {
    let key = shapes_key(shapes_turtle);
    {
        let cache = cache
            .lock()
            .map_err(|e| Error::InvalidValue(format!("validator cache poisoned: {e}")))?;
        if let Some(v) = cache.get(&key)
            && v.shapes_turtle() == shapes_turtle
        {
            return Ok(Arc::clone(v));
        }
    }

    // Parse outside the lock: it is the expensive step, and holding the cache
    // lock across it would serialise every concurrent write behind one parse.
    let validator = Arc::new(Validator::from_turtle(shapes_turtle)?);

    let mut cache = cache
        .lock()
        .map_err(|e| Error::InvalidValue(format!("validator cache poisoned: {e}")))?;
    if cache.len() >= VALIDATOR_CACHE_CAP {
        cache.clear();
    }
    cache.insert(key, Arc::clone(&validator));
    Ok(validator)
}

/// Convenience: validate proposed data against shapes, both as Turtle strings.
///
/// Returns structured feedback for agent consumption. The parsed shapes graph
/// is cached across calls — see `cached_validator`.
pub fn validate_shapes(shapes_turtle: &str, data_turtle: &str) -> Result<ValidationFeedback> {
    let validator = cached_validator(shapes_turtle)?;
    validator.validate(data_turtle.as_bytes())
}

#[cfg(test)]
#[path = "shacl_tests.rs"]
mod tests;

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

//! Edition selection from a supplied catalogue snapshot.
//!
//! Parsing and interval selection do not certify where a snapshot came from
//! or whether its history is complete. Source admission must establish those
//! facts before the selected rules can be treated as a historical answer.

use std::collections::BTreeMap;

use oxrdf::Term;
use oxrdfio::{RdfFormat, RdfParser};
use sha2::{Digest, Sha256};

const TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const VERSION: &str = "http://purl.org/dc/terms/isVersionOf";
const SOURCE: &str = "http://purl.org/dc/terms/source";
const VALUE: &str = "http://www.w3.org/ns/prov#value";
const TIME: &str = "http://www.w3.org/2006/time#";
const SPDX: &str = "http://spdx.org/rdf/terms#";

type Properties = BTreeMap<String, Vec<Term>>;
type Document = BTreeMap<String, Properties>;

/// A selected executable file, not a certificate of source history.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SelectedEdition {
    /// Stable catalogue IRI.
    pub ruleset: String,
    /// Immutable executable edition IRI.
    pub edition: String,
    /// Digest over the exact UTF-8 Turtle bytes.
    pub content_sha256: String,
    /// Normalized requested applicability instant.
    pub valid_at: String,
    /// Inclusive lower applicability bound.
    pub begins: String,
    /// Exclusive upper applicability bound; absent means open-ended.
    pub ends: Option<String>,
    /// Pinned primary sources, separate from executable file identity.
    pub primary_sources: Vec<String>,
    /// Exact verified executable bytes, omitted from identity JSON.
    #[serde(skip_serializing)]
    pub turtle: String,
}

/// Explicit selection outcomes; none falls back to a current rules file.
#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    /// Input or candidate count exceeds the bounded catalogue profile.
    #[error("catalogue capacity exceeded")]
    Capacity,
    /// Malformed RDF, cardinality, datatype or structural relationship.
    #[error("malformed catalogue: {0}")]
    MalformedCatalogue(String),
    /// The stable catalogue identity is absent or not explicitly typed.
    #[error("missing catalogue: {0}")]
    MissingCatalogue(String),
    /// A present catalogue has no retained edition members.
    #[error("empty catalogue: {0}")]
    EmptyCatalogue(String),
    /// Date input cannot be represented by the whole-second profile.
    #[error("invalid or unsupported instant: {0}")]
    InvalidInstant(String),
    /// No edition applies at the requested date.
    #[error("no matching edition")]
    NoMatchingEdition,
    /// Overlapping members are an explicit ambiguity, not first-row-wins.
    #[error("ambiguous editions: {0:?}")]
    AmbiguousEditions(Vec<String>),
    /// Executable content and its declared SHA256 differ.
    #[error("invalid content digest for edition {0}")]
    InvalidDigest(String),
    /// Empty or invalid executable rules cannot be accepted as an edition.
    #[error("invalid executable rules for edition {0}")]
    InvalidRules(String),
}

/// Select one edition from an already supplied RDF catalogue snapshot.
///
/// This pure operation performs no graph access, inference or source write.
/// It validates every member before selecting, so an invalid overlapping or
/// out-of-range member cannot disappear through an inner join. The caller
/// must separately establish transaction lineage, scope and history coverage.
pub fn select_from_catalogue_snapshot(
    turtle: &str,
    ruleset: &str,
    valid_at: &str,
) -> Result<SelectedEdition, SelectionError> {
    let at = instant(valid_at, false)?;
    let doc = parse(turtle)?;
    let root = doc
        .get(ruleset)
        .filter(|p| has_type(p, super::RULE_SET_TYPE))
        .ok_or_else(|| SelectionError::MissingCatalogue(ruleset.into()))?;
    literal(root, "http://www.w3.org/2000/01/rdf-schema#label")?;
    let members: Vec<_> = doc
        .iter()
        .filter(|(_, p)| {
            p.get(VERSION).is_some_and(|values| {
                values
                    .iter()
                    .any(|v| matches!(v, Term::NamedNode(n) if n.as_str() == ruleset))
            })
        })
        .collect();
    if members.is_empty() {
        return Err(SelectionError::EmptyCatalogue(ruleset.into()));
    }
    if members.len() > 64 {
        return Err(SelectionError::Capacity);
    }
    let mut eligible = Vec::new();
    for (id, properties) in members {
        let edition = decode_edition(&doc, id, properties, ruleset, &at)?;
        if edition.begins <= at && edition.ends.as_ref().is_none_or(|end| at < *end) {
            eligible.push(edition);
        }
    }
    match eligible.len() {
        0 => Err(SelectionError::NoMatchingEdition),
        1 => Ok(eligible.remove(0)),
        _ => Err(SelectionError::AmbiguousEditions(
            eligible.into_iter().map(|e| e.edition).collect(),
        )),
    }
}

fn decode_edition(
    doc: &Document,
    id: &str,
    properties: &Properties,
    ruleset: &str,
    at: &str,
) -> Result<SelectedEdition, SelectionError> {
    if !has_type(properties, &format!("{}RuleEdition", super::RULE_NS))
        || iri(properties, VERSION)? != ruleset
    {
        return malformed("edition type or catalogue relationship");
    }
    let interval = resource(
        doc,
        properties,
        &format!("{TIME}hasTime"),
        &format!("{TIME}Interval"),
    )?;
    let begins = boundary(doc, interval, "hasBeginning")?;
    let ends = if interval.contains_key(&format!("{TIME}hasEnd")) {
        Some(boundary(doc, interval, "hasEnd")?)
    } else {
        None
    };
    if ends.as_ref().is_some_and(|end| begins >= *end) {
        return malformed("empty or reversed applicability interval");
    }
    let content = literal(properties, VALUE)?;
    let checksum = resource(
        doc,
        properties,
        &format!("{SPDX}checksum"),
        &format!("{SPDX}Checksum"),
    )?;
    let declared = literal(checksum, &format!("{SPDX}checksumValue"))?;
    let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
    if iri(checksum, &format!("{SPDX}algorithm"))? != format!("{SPDX}checksumAlgorithm_sha256")
        || declared != digest
    {
        return Err(SelectionError::InvalidDigest(id.into()));
    }
    let rules =
        super::parse_rules(&content, None).map_err(|_| SelectionError::InvalidRules(id.into()))?;
    if rules.is_empty() {
        return Err(SelectionError::InvalidRules(id.into()));
    }
    let sources = properties
        .get(SOURCE)
        .ok_or_else(|| SelectionError::MalformedCatalogue("missing primary source".into()))?;
    let mut primary_sources = Vec::new();
    for source in sources {
        let Term::NamedNode(source) = source else {
            return malformed("primary source must be an IRI");
        };
        primary_sources.push(source.as_str().to_string());
    }
    Ok(SelectedEdition {
        ruleset: ruleset.into(),
        edition: id.into(),
        content_sha256: digest,
        valid_at: at.into(),
        begins,
        ends,
        primary_sources,
        turtle: content,
    })
}

fn boundary(doc: &Document, interval: &Properties, name: &str) -> Result<String, SelectionError> {
    let point = resource(
        doc,
        interval,
        &format!("{TIME}{name}"),
        &format!("{TIME}Instant"),
    )?;
    let term = one(point, &format!("{TIME}inXSDDateTimeStamp"))?;
    let Term::Literal(value) = term else {
        return malformed("boundary is not a timestamp");
    };
    if value.datatype().as_str() != "http://www.w3.org/2001/XMLSchema#dateTimeStamp" {
        return malformed("boundary datatype is not xsd:dateTimeStamp");
    }
    instant(value.value(), true)
}

fn instant(value: &str, canonical: bool) -> Result<String, SelectionError> {
    if value.as_bytes().get(19) == Some(&b'.') || value.get(17..19) == Some("60") {
        return Err(SelectionError::InvalidInstant(value.into()));
    }
    let normalized = crate::time::normalize_rfc3339_utc(value)
        .ok_or_else(|| SelectionError::InvalidInstant(value.into()))?;
    if normalized.len() != 20
        || !normalized.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        || (canonical && normalized != value)
    {
        return Err(SelectionError::InvalidInstant(value.into()));
    }
    Ok(normalized)
}

fn parse(turtle: &str) -> Result<Document, SelectionError> {
    if turtle.len() > 1024 * 1024 {
        return Err(SelectionError::Capacity);
    }
    let mut doc = Document::new();
    for (count, quad) in RdfParser::from_format(RdfFormat::Turtle)
        .for_reader(turtle.as_bytes())
        .enumerate()
    {
        if count >= 4096 {
            return Err(SelectionError::Capacity);
        }
        let triple = oxrdf::Triple::from(
            quad.map_err(|e| SelectionError::MalformedCatalogue(e.to_string()))?,
        );
        let oxrdf::NamedOrBlankNode::NamedNode(subject) = triple.subject else {
            return malformed("catalogue resources must use stable IRIs");
        };
        let values = doc
            .entry(subject.as_str().into())
            .or_default()
            .entry(triple.predicate.as_str().into())
            .or_default();
        if !values.contains(&triple.object) {
            values.push(triple.object);
        }
    }
    Ok(doc)
}

fn has_type(properties: &Properties, class: &str) -> bool {
    properties.get(TYPE).is_some_and(|v| {
        v.iter()
            .any(|t| matches!(t, Term::NamedNode(n) if n.as_str() == class))
    })
}
fn one<'a>(properties: &'a Properties, predicate: &str) -> Result<&'a Term, SelectionError> {
    match properties.get(predicate).map(Vec::as_slice) {
        Some([term]) => Ok(term),
        _ => malformed(&format!("expected exactly one {predicate}")),
    }
}
fn iri(properties: &Properties, predicate: &str) -> Result<String, SelectionError> {
    match one(properties, predicate)? {
        Term::NamedNode(n) => Ok(n.as_str().into()),
        _ => malformed(&format!("expected IRI for {predicate}")),
    }
}
fn literal(properties: &Properties, predicate: &str) -> Result<String, SelectionError> {
    match one(properties, predicate)? {
        Term::Literal(l) if l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#string" => {
            Ok(l.value().into())
        }
        _ => malformed(&format!("expected string for {predicate}")),
    }
}
fn resource<'a>(
    doc: &'a Document,
    properties: &Properties,
    predicate: &str,
    class: &str,
) -> Result<&'a Properties, SelectionError> {
    let id = iri(properties, predicate)?;
    doc.get(&id)
        .filter(|p| has_type(p, class))
        .ok_or_else(|| SelectionError::MalformedCatalogue(format!("missing typed resource {id}")))
}
fn malformed<T>(message: &str) -> Result<T, SelectionError> {
    Err(SelectionError::MalformedCatalogue(message.into()))
}

//! RDF data model layer — bridges oxrdf types with the EAVT fact log.
//!
//! Converts between standard RDF terms (IRIs, blank nodes, literals) and the
//! integer-encoded term dictionary + typed `Value` used by the fact log.
//! Supports parsing Turtle/N-Triples/JSON-LD into the store and serializing
//! facts back to standard RDF formats.

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term as OxTerm, Triple};
use oxrdfio::{RdfFormat, RdfParser};
use std::io::Read;

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

// Prefix for blank node IRIs in the term dictionary.
pub(crate) const BLANK_PREFIX: &str = "_:";

/// Convert an oxrdf subject to a term dictionary id.
pub fn intern_subject(store: &Store, subject: &NamedOrBlankNode) -> Result<i64> {
    match subject {
        NamedOrBlankNode::NamedNode(n) => store.intern(n.as_str()),
        NamedOrBlankNode::BlankNode(b) => store.intern(&format!("{BLANK_PREFIX}{}", b.as_str())),
    }
}

/// Convert an oxrdf `Term` (object position) to a `Value`.
///
/// - Named nodes and blank nodes → `Value::Ref(term_id)`
/// - Literals → `Value::Str`, `Value::Int`, `Value::Float`, `Value::Bool`
///   depending on the XSD datatype.
pub fn term_to_value(store: &Store, term: &OxTerm) -> Result<Value> {
    match term {
        OxTerm::NamedNode(n) => {
            let id = store.intern(n.as_str())?;
            Ok(Value::Ref(id))
        }
        OxTerm::BlankNode(b) => {
            let id = store.intern(&format!("{BLANK_PREFIX}{}", b.as_str()))?;
            Ok(Value::Ref(id))
        }
        OxTerm::Literal(lit) => literal_to_value(lit),
        #[cfg(feature = "shacl")]
        OxTerm::Triple(_) => Err(Error::InvalidValue("unsupported RDF term type".into())),
    }
}

/// Map an RDF literal to a typed `Value` based on its XSD datatype.
fn literal_to_value(lit: &Literal) -> Result<Value> {
    // Language tags are checked first (their datatype is rdf:langString) and
    // are stored SEPARATELY from the lexical form. Concatenating them, as this
    // did, is irreversible corruption — aegis-fmyi.
    if let Some(lang) = lit.language() {
        return Ok(Value::Lang {
            lexical: lit.value().to_string(),
            lang: lang.to_string(),
        });
    }
    let dt = lit.datatype().as_str();
    match dt {
        namespace::XSD_INTEGER => {
            let n: i64 = lit
                .value()
                .parse()
                .map_err(|e| Error::InvalidValue(format!("bad integer literal: {e}")))?;
            Ok(Value::Int(n))
        }
        namespace::XSD_DOUBLE => {
            let value = lit
                .value()
                .parse::<f64>()
                .map_err(|e| Error::InvalidValue(format!("bad float literal: {e}")))?;
            Ok(Value::Typed {
                lexical: canonical_double(value),
                datatype: dt.to_string(),
            })
        }
        namespace::XSD_BOOLEAN => {
            let b = matches!(lit.value(), "true" | "1");
            Ok(Value::Bool(b))
        }
        namespace::XSD_STRING => Ok(Value::Str(lit.value().to_string())),
        _ => {
            // Numeric subtypes still have to parse — a malformed xsd:long is an
            // ingest error, not a string — but they keep their datatype IRI so
            // xsd:long/xsd:decimal/xsd:double stay distinguishable.
            if namespace::is_numeric_datatype(dt) {
                lit.value()
                    .parse::<f64>()
                    .map_err(|e| Error::InvalidValue(format!("bad numeric literal <{dt}>: {e}")))?;
            }
            Ok(Value::Typed {
                lexical: lit.value().to_string(),
                datatype: dt.to_string(),
            })
        }
    }
}

fn canonical_double(value: f64) -> String {
    let rendered = format!("{value:E}");
    let (mantissa, exponent) = rendered.split_once('E').unwrap_or((&rendered, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_string()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{}", exponent.parse::<i32>().unwrap_or(0))
}

/// Convert a `Value` back to an oxrdf `Term` for serialization.
pub fn value_to_term(store: &Store, value: &Value) -> Result<OxTerm> {
    match value {
        Value::Ref(id) => {
            let iri = store.resolve(*id)?;
            if let Some(bnode_id) = iri.strip_prefix(BLANK_PREFIX) {
                Ok(OxTerm::BlankNode(BlankNode::new(bnode_id).map_err(
                    |e| Error::InvalidValue(format!("bad blank node: {e}")),
                )?))
            } else {
                Ok(OxTerm::NamedNode(NamedNode::new(&iri).map_err(|e| {
                    Error::InvalidValue(format!("bad IRI: {e}"))
                })?))
            }
        }
        // A plain Str is a plain literal. It is NOT sniffed for a trailing
        // "@xx" — the string "hello@en" is a legitimate string, and guessing
        // would manufacture a language tag nobody asserted (aegis-fmyi).
        Value::Str(s) => Ok(OxTerm::Literal(Literal::new_simple_literal(s))),
        Value::Lang { lexical, lang } => Ok(OxTerm::Literal(
            Literal::new_language_tagged_literal(lexical, lang)
                .map_err(|e| Error::InvalidValue(format!("bad language tag: {e}")))?,
        )),
        Value::Typed { lexical, datatype } => Ok(OxTerm::Literal(Literal::new_typed_literal(
            lexical,
            NamedNode::new(datatype)
                .map_err(|e| Error::InvalidValue(format!("bad datatype IRI: {e}")))?,
        ))),
        Value::Int(n) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            n.to_string(),
            NamedNode::new_unchecked(namespace::XSD_INTEGER),
        ))),
        Value::Float(f) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            f.to_string(),
            NamedNode::new_unchecked(namespace::XSD_DOUBLE),
        ))),
        Value::Bool(b) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            b.to_string(),
            NamedNode::new_unchecked(namespace::XSD_BOOLEAN),
        ))),
        Value::Bytes(_) => Err(Error::InvalidValue(
            "cannot convert raw bytes to RDF term".into(),
        )),
    }
}

/// Resolve a fact's entity id back to an oxrdf subject.
fn id_to_subject(store: &Store, id: i64) -> Result<NamedOrBlankNode> {
    let iri = store.resolve(id)?;
    if let Some(bnode_id) = iri.strip_prefix(BLANK_PREFIX) {
        Ok(NamedOrBlankNode::BlankNode(
            BlankNode::new(bnode_id)
                .map_err(|e| Error::InvalidValue(format!("bad blank node: {e}")))?,
        ))
    } else {
        Ok(NamedOrBlankNode::NamedNode(NamedNode::new(&iri).map_err(
            |e| Error::InvalidValue(format!("bad IRI: {e}")),
        )?))
    }
}

/// Resolve a fact's attribute id back to an oxrdf `NamedNode`.
fn id_to_predicate(store: &Store, id: i64) -> Result<NamedNode> {
    let iri = store.resolve(id)?;
    NamedNode::new(&iri).map_err(|e| Error::InvalidValue(format!("bad predicate IRI: {e}")))
}

// ── Public API ──────────────────────────────────────────────────

/// Parse RDF from a reader and ingest all triples into the fact log.
///
/// Supported formats: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, `TriG`.
/// Returns the transaction id and the number of triples ingested.
pub fn ingest_rdf(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
) -> Result<(i64, usize)> {
    // Default target is ROOT (g=0). Named-graph writes use the _to_graph form.
    ingest_rdf_to_graph(store, reader, format, base_iri, timestamp, actor, source, 0)
}

/// Ingest RDF into a specific named graph `g` (aegis-g1al / #36). g=0 is ROOT.
/// All facts from this parse land in `graph`, via `transact_to_graph`, so an
/// overlay ingest extends ROOT without mutating it.
#[allow(clippy::too_many_arguments)]
pub fn ingest_rdf_to_graph(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
) -> Result<(i64, usize)> {
    let datums = parse_rdf(store, reader, format, base_iri, timestamp)?;
    let count = datums.len();
    let tx_id = store.transact_to_graph(&datums, timestamp, actor, source, graph)?;
    Ok((tx_id, count))
}

/// Parse RDF into fact-log datums without committing them.
///
/// This is used by callers that must combine RDF assertions with other changes
/// in one atomic transaction, such as replacing a producer-owned snapshot.
pub(crate) fn parse_rdf(
    store: &Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
) -> Result<Vec<Datum>> {
    let mut parser = RdfParser::from_format(format);
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .map_err(|e| Error::InvalidValue(format!("bad base IRI: {e}")))?;
    }

    let mut datums = Vec::new();
    for result in parser.for_reader(reader) {
        let quad = result.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
        let triple = Triple::from(quad);

        let e = intern_subject(store, &triple.subject)?;
        let a = store.intern(triple.predicate.as_str())?;
        let v = term_to_value(store, &triple.object)?;

        datums.push(Datum {
            entity: e,
            attribute: a,
            value: v,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }

    Ok(datums)
}

/// Serialize current facts as RDF in the specified format.
///
/// Supported output formats: Turtle, N-Triples, N-Quads, RDF/XML, `TriG`.
/// Serialize a set of facts to an RDF document (shared by the exporters).
/// `pub(crate)` since quipu-gp5: fork promotion serializes its asserted delta
/// for the SHACL gate through the same path the exporters use.
pub(crate) fn serialize_facts(
    store: &Store,
    facts: &[crate::types::Fact],
    format: RdfFormat,
) -> Result<Vec<u8>> {
    let mut triples = Vec::with_capacity(facts.len());
    for fact in facts {
        let subject = id_to_subject(store, fact.entity)?;
        let predicate = id_to_predicate(store, fact.attribute)?;
        let object = value_to_term(store, &fact.value)?;

        triples.push(Triple {
            subject,
            predicate,
            object,
        });
    }

    crate::rdf_export::serialize_triples_canonical(&triples, format)
}

/// Export the ROOT/default graph's current facts, flattened to triples.
///
/// Drive-by (quipu #81): this said "EVERY current fact across all graphs".
/// `current_facts` is ROOT-only, so the docstring promised a whole-store export
/// that has never happened — name a graph with [`export_rdf_subset`] instead.
pub fn export_rdf(store: &Store, format: RdfFormat) -> Result<Vec<u8>> {
    let facts = store.current_facts()?;
    serialize_facts(store, &facts, format)
}

/// Export a SUBSET — the current facts of one named graph, or the ROOT/default
/// graph (quipu #36). This is the "pull a scoped slice" primitive that
/// subset-export and, above it, federation build on. `graph` is the graph's IRI;
/// `None` exports the ROOT default graph (`g = 0`). An unknown graph IRI is an
/// error (a targeted export of a graph that does not exist is a caller mistake,
/// not an empty success). The slice is a graph's OWN facts — the same scope a
/// `GRAPH <iri> { … }` read sees.
pub fn export_rdf_subset(
    store: &Store,
    format: RdfFormat,
    graph: Option<&str>,
) -> Result<(Vec<u8>, usize)> {
    let g = match graph {
        None => 0,
        Some(iri) => store
            .lookup(iri)?
            .ok_or_else(|| Error::InvalidValue(format!("unknown graph: {iri}")))?,
    };
    let facts = store.current_facts_in_graph(g)?;
    let count = facts.len();
    Ok((serialize_facts(store, &facts, format)?, count))
}

#[cfg(test)]
include!("rdf_tests.rs");

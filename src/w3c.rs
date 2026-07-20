//! W3C-standard SPARQL 1.1 Query Results serialization for content-negotiated
//! `/query` responses (aegis-u7ag).
//!
//! The default `/query` response is a bespoke flat-rows JSON shape that is
//! *lossy*: a `Value::Ref` (an IRI) and a `Value::Str` (a string literal) both
//! serialize to a bare JSON string, so a consumer cannot tell a node from a
//! string, and no off-the-shelf SPARQL client (rdflib, `SPARQLWrapper`, Jena,
//! YASGUI) can parse it. When a caller sets an `Accept` header naming a standard
//! results/RDF media type we serialize the W3C shape instead — which carries a
//! `"type": "uri" | "literal"` tag on every binding, restoring the distinction
//! the default shape destroys.
//!
//! ## Scope (deliberately bounded — see the bead)
//!
//! Term *kind* (uri vs. literal) IS preserved, because `Value::Ref` and
//! `Value::Str` are distinct enum variants.
//!
//! Datatypes and language tags are now preserved too, because the `Value` model
//! carries them: `Value::Lang` and `Value::Typed` (aegis-fmyi). This paragraph
//! used to say the opposite — that they were destroyed at PARSE time and could
//! not be recovered here at any price. That was true, and it was the reason a
//! serializer-only change could never have fixed them.
//!
//! Every datatype emitted below comes from the value itself: the `Int` / `Float`
//! / `Bool` discriminants, or the IRI a `Typed` literal stored verbatim. We
//! NEVER sniff a datatype or a language tag back out of a `Value::Str` lexical
//! form — the plain string `"hello@en"` and the string `"2026-07-15"` are both
//! legitimate, and guessing would manufacture confident lies. A datatype
//! round-trip that "passes" via sniffing is evidence of the bug, not of a fix.

use oxrdf::{vocab::xsd, Literal, NamedNode, Term as OxTerm, Triple as OxTriple, Variable};
use oxrdfio::{RdfFormat, RdfSerializer};
use sparesults::{QueryResultsFormat, QueryResultsSerializer};

use crate::error::{Error, Result};
use crate::sparql::{QueryResult, Triple};
use crate::store::Store;
use crate::types::Value;

/// A negotiated standard output format for `/query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultFormat {
    /// `application/sparql-results+json` — SELECT / ASK.
    SparqlJson,
    /// `application/sparql-results+xml` — SELECT / ASK.
    SparqlXml,
    /// `text/turtle` — CONSTRUCT / DESCRIBE.
    Turtle,
    /// `application/n-triples` — CONSTRUCT / DESCRIBE.
    NTriples,
}

/// Pick a standard format from an HTTP `Accept` header value.
///
/// Returns `None` for an absent/empty header, `*/*`, `application/json`, or any
/// value that does not explicitly name a standard SPARQL/RDF media type — those
/// cases fall through to the default bespoke rows shape, so existing callers
/// (the web UI, `skills/graph-report`, `skills/quipu`) are byte-for-byte
/// unaffected. Only an explicit standard media type opts in to the W3C shape.
///
/// `q`-values are not honored; the first standard type found (in the priority
/// order below) wins. That is sufficient for real clients, which send a single
/// concrete results type.
#[must_use]
pub fn negotiate(accept: &str) -> Option<ResultFormat> {
    let a = accept.to_ascii_lowercase();
    if a.contains("application/sparql-results+json") {
        Some(ResultFormat::SparqlJson)
    } else if a.contains("application/sparql-results+xml") {
        Some(ResultFormat::SparqlXml)
    } else if a.contains("text/turtle") {
        Some(ResultFormat::Turtle)
    } else if a.contains("application/n-triples") {
        Some(ResultFormat::NTriples)
    } else {
        None
    }
}

/// Serialize a query result in a negotiated standard format.
///
/// Returns `Ok(Some((content_type, body)))` on success, or `Ok(None)` when the
/// result variant cannot be expressed in `fmt` (e.g. a CONSTRUCT graph requested
/// as `sparql-results+json`, or a SELECT requested as `text/turtle`). In the
/// `None` case the caller falls back to the default bespoke rows shape rather
/// than erroring, so a mismatched `Accept` degrades gracefully instead of 406-ing.
///
/// `result` is expected to have already had the server-side row ceiling applied
/// (see `mcp::query_result`); the standard shapes have no field for a `truncated`
/// flag, so the cap is silent here by design — but it is the same cap.
pub fn serialize(
    store: &Store,
    result: &QueryResult,
    fmt: ResultFormat,
) -> Result<Option<(&'static str, Vec<u8>)>> {
    match (result, fmt) {
        (
            QueryResult::Select { variables, rows },
            ResultFormat::SparqlJson | ResultFormat::SparqlXml,
        ) => {
            let vars: Vec<Variable> = variables
                .iter()
                .map(|v| Variable::new_unchecked(v.clone()))
                .collect();
            let mut ser = QueryResultsSerializer::from_format(sparql_format(fmt))
                .serialize_solutions_to_writer(Vec::new(), vars.clone())
                .map_err(|e| Error::Serialization(format!("W3C results init: {e}")))?;
            for row in rows {
                // Only bound variables appear in a row's map; unbound head vars
                // are omitted from the binding, which is exactly W3C-correct.
                let binding: Vec<(&Variable, OxTerm)> = vars
                    .iter()
                    .filter_map(|var| row.get(var.as_str()).map(|val| (var, value_to_term(store, val))))
                    .collect();
                ser.serialize(binding.iter().map(|(v, t)| (*v, t)))
                    .map_err(|e| Error::Serialization(format!("W3C results row: {e}")))?;
            }
            let body = ser
                .finish()
                .map_err(|e| Error::Serialization(format!("W3C results finish: {e}")))?;
            Ok(Some((content_type(fmt), body)))
        }
        (QueryResult::Ask(value), ResultFormat::SparqlJson | ResultFormat::SparqlXml) => {
            let body = QueryResultsSerializer::from_format(sparql_format(fmt))
                .serialize_boolean_to_writer(Vec::new(), *value)
                .map_err(|e| Error::Serialization(format!("W3C ASK: {e}")))?;
            Ok(Some((content_type(fmt), body)))
        }
        (QueryResult::Graph(triples), ResultFormat::Turtle | ResultFormat::NTriples) => {
            let rdf_format = if fmt == ResultFormat::Turtle {
                RdfFormat::Turtle
            } else {
                RdfFormat::NTriples
            };
            let mut ser = RdfSerializer::from_format(rdf_format).for_writer(Vec::new());
            for t in triples {
                ser.serialize_triple(&triple_to_ox(store, t))
                    .map_err(|e| Error::Serialization(format!("W3C triple: {e}")))?;
            }
            let body = ser
                .finish()
                .map_err(|e| Error::Serialization(format!("W3C graph finish: {e}")))?;
            Ok(Some((content_type(fmt), body)))
        }
        // Format does not fit the result variant — caller falls back to bespoke.
        _ => Ok(None),
    }
}

fn sparql_format(fmt: ResultFormat) -> QueryResultsFormat {
    match fmt {
        ResultFormat::SparqlXml => QueryResultsFormat::Xml,
        _ => QueryResultsFormat::Json,
    }
}

fn content_type(fmt: ResultFormat) -> &'static str {
    match fmt {
        ResultFormat::SparqlJson => "application/sparql-results+json",
        ResultFormat::SparqlXml => "application/sparql-results+xml",
        ResultFormat::Turtle => "text/turtle",
        ResultFormat::NTriples => "application/n-triples",
    }
}

/// Map a stored `Value` to an oxrdf `Term` for standard serialization.
///
/// `Ref` becomes an IRI (`"type": "uri"`); everything else is a literal
/// (`"type": "literal"`). This is the one distinction the bespoke shape loses.
fn value_to_term(store: &Store, val: &Value) -> OxTerm {
    match val {
        Value::Ref(id) => {
            let iri = store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}"));
            OxTerm::from(NamedNode::new_unchecked(iri))
        }
        Value::Str(s) => OxTerm::from(Literal::new_simple_literal(s.clone())),
        Value::Lang { lexical, lang } => Literal::new_language_tagged_literal(lexical, lang)
            .map_or_else(
                |_| OxTerm::from(Literal::new_simple_literal(lexical.clone())),
                OxTerm::from,
            ),
        Value::Typed { lexical, datatype } => NamedNode::new(datatype).map_or_else(
            |_| OxTerm::from(Literal::new_simple_literal(lexical.clone())),
            |dt| OxTerm::from(Literal::new_typed_literal(lexical.clone(), dt)),
        ),
        Value::Int(n) => OxTerm::from(Literal::new_typed_literal(n.to_string(), xsd::INTEGER)),
        Value::Float(f) => OxTerm::from(Literal::new_typed_literal(fmt_double(*f), xsd::DOUBLE)),
        Value::Bool(b) => OxTerm::from(Literal::new_typed_literal(b.to_string(), xsd::BOOLEAN)),
        // No honest W3C term for opaque bytes; keep the bespoke placeholder as a
        // plain literal rather than invent a datatype. Bytes do not appear in
        // SPARQL bindings in practice (they back vector blobs, not queried terms).
        Value::Bytes(b) => OxTerm::from(Literal::new_simple_literal(format!("<{} bytes>", b.len()))),
    }
}

/// Canonical-ish xsd:double lexical form (handles the XSD special values and
/// ensures a non-integer-looking lexeme so the datatype reads honestly).
fn fmt_double(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_string()
    } else if f.is_infinite() {
        if f > 0.0 { "INF".to_string() } else { "-INF".to_string() }
    } else {
        let s = f.to_string();
        if s.contains(['.', 'e', 'E']) { s } else { format!("{s}.0") }
    }
}

/// Build an oxrdf `Triple` from a quipu CONSTRUCT/DESCRIBE `Triple`. Subject and
/// predicate are already-resolved IRI strings; the object goes through
/// [`value_to_term`].
fn triple_to_ox(store: &Store, t: &Triple) -> OxTriple {
    OxTriple::new(
        NamedNode::new_unchecked(t.subject.clone()),
        NamedNode::new_unchecked(t.predicate.clone()),
        value_to_term(store, &t.object),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_maps_standard_types_and_ignores_the_rest() {
        assert_eq!(negotiate("application/sparql-results+json"), Some(ResultFormat::SparqlJson));
        assert_eq!(negotiate("application/sparql-results+xml"), Some(ResultFormat::SparqlXml));
        assert_eq!(negotiate("text/turtle"), Some(ResultFormat::Turtle));
        assert_eq!(negotiate("application/n-triples"), Some(ResultFormat::NTriples));
        // case-insensitive + parameters tolerated
        assert_eq!(
            negotiate("Application/SPARQL-Results+JSON; charset=utf-8"),
            Some(ResultFormat::SparqlJson)
        );
        // the fall-through cases that MUST stay on the bespoke shape
        assert_eq!(negotiate(""), None);
        assert_eq!(negotiate("*/*"), None);
        assert_eq!(negotiate("application/json"), None);
        assert_eq!(negotiate("text/html"), None);
    }

    #[test]
    fn fmt_double_is_honest() {
        assert_eq!(fmt_double(1.0), "1.0");
        assert_eq!(fmt_double(1.5), "1.5");
        assert_eq!(fmt_double(f64::INFINITY), "INF");
        assert_eq!(fmt_double(f64::NEG_INFINITY), "-INF");
        assert_eq!(fmt_double(f64::NAN), "NaN");
    }
}

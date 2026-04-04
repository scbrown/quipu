//! RDF data model layer — bridges oxrdf types with the EAVT fact log.
//!
//! Converts between standard RDF terms (IRIs, blank nodes, literals) and the
//! integer-encoded term dictionary + typed `Value` used by the fact log.
//! Supports parsing Turtle/N-Triples/JSON-LD into the store and serializing
//! facts back to standard RDF formats.

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term as OxTerm, Triple};
use oxrdfio::{RdfFormat, RdfParser, RdfSerializer};
use std::io::Read;

use crate::error::{Error, Result};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

// Prefix for blank node IRIs in the term dictionary.
const BLANK_PREFIX: &str = "_:";

/// Convert an oxrdf subject to a term dictionary id.
fn intern_subject(store: &Store, subject: &NamedOrBlankNode) -> Result<i64> {
    match subject {
        NamedOrBlankNode::NamedNode(n) => store.intern(n.as_str()),
        NamedOrBlankNode::BlankNode(b) => {
            store.intern(&format!("{BLANK_PREFIX}{}", b.as_str()))
        }
    }
}

/// Convert an oxrdf `Term` (object position) to a `Value`.
///
/// - Named nodes and blank nodes → `Value::Ref(term_id)`
/// - Literals → `Value::Str`, `Value::Int`, `Value::Float`, `Value::Bool`
///   depending on the XSD datatype.
fn term_to_value(store: &Store, term: &OxTerm) -> Result<Value> {
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
        #[allow(unreachable_patterns)]
        _ => Err(Error::InvalidValue(
            "unsupported RDF term type in object position".into(),
        )),
    }
}

/// Map an RDF literal to a typed `Value` based on its XSD datatype.
fn literal_to_value(lit: &Literal) -> Result<Value> {
    let dt = lit.datatype().as_str();
    match dt {
        "http://www.w3.org/2001/XMLSchema#integer"
        | "http://www.w3.org/2001/XMLSchema#long"
        | "http://www.w3.org/2001/XMLSchema#int"
        | "http://www.w3.org/2001/XMLSchema#short"
        | "http://www.w3.org/2001/XMLSchema#byte"
        | "http://www.w3.org/2001/XMLSchema#nonNegativeInteger"
        | "http://www.w3.org/2001/XMLSchema#positiveInteger"
        | "http://www.w3.org/2001/XMLSchema#unsignedLong"
        | "http://www.w3.org/2001/XMLSchema#unsignedInt" => {
            let n: i64 = lit
                .value()
                .parse()
                .map_err(|e| Error::InvalidValue(format!("bad integer literal: {e}")))?;
            Ok(Value::Int(n))
        }
        "http://www.w3.org/2001/XMLSchema#double"
        | "http://www.w3.org/2001/XMLSchema#float"
        | "http://www.w3.org/2001/XMLSchema#decimal" => {
            let f: f64 = lit
                .value()
                .parse()
                .map_err(|e| Error::InvalidValue(format!("bad float literal: {e}")))?;
            Ok(Value::Float(f))
        }
        "http://www.w3.org/2001/XMLSchema#boolean" => {
            let b = matches!(lit.value(), "true" | "1");
            Ok(Value::Bool(b))
        }
        _ => {
            if let Some(lang) = lit.language() {
                Ok(Value::Str(format!("{}@{}", lit.value(), lang)))
            } else {
                Ok(Value::Str(lit.value().to_string()))
            }
        }
    }
}

/// Convert a `Value` back to an oxrdf `Term` for serialization.
fn value_to_term(store: &Store, value: &Value) -> Result<OxTerm> {
    match value {
        Value::Ref(id) => {
            let iri = store.resolve(*id)?;
            if let Some(bnode_id) = iri.strip_prefix(BLANK_PREFIX) {
                Ok(OxTerm::BlankNode(
                    BlankNode::new(bnode_id)
                        .map_err(|e| Error::InvalidValue(format!("bad blank node: {e}")))?,
                ))
            } else {
                Ok(OxTerm::NamedNode(
                    NamedNode::new(&iri)
                        .map_err(|e| Error::InvalidValue(format!("bad IRI: {e}")))?,
                ))
            }
        }
        Value::Str(s) => {
            if let Some(at_pos) = s.rfind('@') {
                let (val, lang_with_at) = s.split_at(at_pos);
                let lang = &lang_with_at[1..];
                if !lang.is_empty()
                    && lang.len() <= 10
                    && lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    return Ok(OxTerm::Literal(
                        Literal::new_language_tagged_literal(val, lang).map_err(|e| {
                            Error::InvalidValue(format!("bad language tag: {e}"))
                        })?,
                    ));
                }
            }
            Ok(OxTerm::Literal(Literal::new_simple_literal(s)))
        }
        Value::Int(n) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            n.to_string(),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
        ))),
        Value::Float(f) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            f.to_string(),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#double"),
        ))),
        Value::Bool(b) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            b.to_string(),
            NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#boolean"),
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
        Ok(NamedOrBlankNode::NamedNode(
            NamedNode::new(&iri).map_err(|e| Error::InvalidValue(format!("bad IRI: {e}")))?,
        ))
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
/// Supported formats: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, TriG.
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
    let mut parser = RdfParser::from_format(format);
    if let Some(base) = base_iri {
        parser = parser.with_base_iri(base).map_err(|e| {
            Error::InvalidValue(format!("bad base IRI: {e}"))
        })?;
    }

    let mut datums = Vec::new();
    for result in parser.for_reader(reader) {
        let quad =
            result.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
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

    let count = datums.len();
    if count == 0 {
        let tx_id = store.transact(&[], timestamp, actor, source)?;
        return Ok((tx_id, 0));
    }
    let tx_id = store.transact(&datums, timestamp, actor, source)?;
    Ok((tx_id, count))
}

/// Serialize current facts as RDF in the specified format.
///
/// Supported output formats: Turtle, N-Triples, N-Quads, RDF/XML, TriG.
pub fn export_rdf(store: &Store, format: RdfFormat) -> Result<Vec<u8>> {
    let facts = store.current_facts()?;
    let mut buf = Vec::new();
    let mut serializer = RdfSerializer::from_format(format).for_writer(&mut buf);

    for fact in &facts {
        let subject = id_to_subject(store, fact.entity)?;
        let predicate = id_to_predicate(store, fact.attribute)?;
        let object = value_to_term(store, &fact.value)?;

        let triple = Triple {
            subject,
            predicate,
            object,
        };
        serializer
            .serialize_triple(&triple)
            .map_err(|e| Error::InvalidValue(format!("RDF serialization error: {e}")))?;
    }

    serializer
        .finish()
        .map_err(|e| Error::InvalidValue(format!("RDF serialization finish error: {e}")))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURTLE_DATA: &str = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a ex:Person ;
    ex:name "Alice" ;
    ex:age "30"^^xsd:integer ;
    ex:height "1.65"^^xsd:double ;
    ex:active "true"^^xsd:boolean ;
    ex:knows ex:bob .

ex:bob a ex:Person ;
    ex:name "Bob"@en .
"#;

    #[test]
    fn ingest_turtle_round_trip() {
        let mut store = Store::open_in_memory().unwrap();

        let (tx_id, count) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("test"),
            Some("turtle-test"),
        )
        .unwrap();

        assert!(tx_id > 0);
        assert_eq!(count, 8); // 6 for alice + 2 for bob

        let facts = store.current_facts().unwrap();
        assert_eq!(facts.len(), 8);

        // Verify typed values came through correctly.
        let alice_id = store
            .lookup("http://example.org/alice")
            .unwrap()
            .expect("alice should be interned");
        let age_id = store
            .lookup("http://example.org/age")
            .unwrap()
            .expect("age should be interned");
        let alice_facts = store.entity_facts(alice_id).unwrap();

        let age_fact = alice_facts.iter().find(|f| f.attribute == age_id).unwrap();
        assert_eq!(age_fact.value, Value::Int(30));

        let height_id = store.lookup("http://example.org/height").unwrap().unwrap();
        let height_fact = alice_facts
            .iter()
            .find(|f| f.attribute == height_id)
            .unwrap();
        assert_eq!(height_fact.value, Value::Float(1.65));

        let active_id = store.lookup("http://example.org/active").unwrap().unwrap();
        let active_fact = alice_facts
            .iter()
            .find(|f| f.attribute == active_id)
            .unwrap();
        assert_eq!(active_fact.value, Value::Bool(true));

        // Verify object reference (ex:knows ex:bob).
        let knows_id = store.lookup("http://example.org/knows").unwrap().unwrap();
        let knows_fact = alice_facts
            .iter()
            .find(|f| f.attribute == knows_id)
            .unwrap();
        let bob_id = store.lookup("http://example.org/bob").unwrap().unwrap();
        assert_eq!(knows_fact.value, Value::Ref(bob_id));

        // Verify language-tagged literal.
        let bob_facts = store.entity_facts(bob_id).unwrap();
        let name_id = store.lookup("http://example.org/name").unwrap().unwrap();
        let bob_name = bob_facts.iter().find(|f| f.attribute == name_id).unwrap();
        assert_eq!(bob_name.value, Value::Str("Bob@en".into()));
    }

    #[test]
    fn export_ntriples() {
        let mut store = Store::open_in_memory().unwrap();

        ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        let ntriples = export_rdf(&store, RdfFormat::NTriples).unwrap();
        let output = String::from_utf8(ntriples).unwrap();

        assert!(output.contains("<http://example.org/alice>"));
        assert!(output.contains("<http://example.org/Person>"));
        assert!(output.contains("\"Alice\""));
        assert!(output.contains("\"Bob\"@en"));
        assert!(output.contains("\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"));
    }

    #[test]
    fn round_trip_ntriples() {
        let mut store1 = Store::open_in_memory().unwrap();

        ingest_rdf(
            &mut store1,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        let ntriples = export_rdf(&store1, RdfFormat::NTriples).unwrap();

        let mut store2 = Store::open_in_memory().unwrap();
        let (_, count) = ingest_rdf(
            &mut store2,
            ntriples.as_slice(),
            RdfFormat::NTriples,
            None,
            "2026-04-04T01:00:00Z",
            None,
            None,
        )
        .unwrap();

        assert_eq!(count, 8);
        assert_eq!(store2.current_facts().unwrap().len(), 8);
    }

    #[test]
    fn literal_to_value_types() {
        let xsd = "http://www.w3.org/2001/XMLSchema#";

        let cases: Vec<(Literal, Value)> = vec![
            (
                Literal::new_typed_literal("42", NamedNode::new_unchecked(format!("{xsd}integer"))),
                Value::Int(42),
            ),
            (
                Literal::new_typed_literal(
                    "3.14",
                    NamedNode::new_unchecked(format!("{xsd}double")),
                ),
                Value::Float(3.14),
            ),
            (
                Literal::new_typed_literal(
                    "true",
                    NamedNode::new_unchecked(format!("{xsd}boolean")),
                ),
                Value::Bool(true),
            ),
            (
                Literal::new_typed_literal(
                    "false",
                    NamedNode::new_unchecked(format!("{xsd}boolean")),
                ),
                Value::Bool(false),
            ),
            (
                Literal::new_simple_literal("hello"),
                Value::Str("hello".into()),
            ),
            (
                Literal::new_language_tagged_literal("hola", "es").unwrap(),
                Value::Str("hola@es".into()),
            ),
        ];

        for (lit, expected) in cases {
            let result = super::literal_to_value(&lit).unwrap();
            assert_eq!(result, expected, "failed for literal: {lit}");
        }
    }

    #[test]
    fn blank_node_round_trip() {
        let ntriples = r#"
_:node1 <http://example.org/label> "test" .
<http://example.org/thing> <http://example.org/ref> _:node1 .
"#;
        let mut store = Store::open_in_memory().unwrap();
        let (_, count) = ingest_rdf(
            &mut store,
            ntriples.as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();

        assert_eq!(count, 2);

        // The blank node should be in the term dictionary.
        let bnode_id = store.lookup("_:node1").unwrap().expect("blank node interned");
        assert!(bnode_id > 0);

        // The reference should point to the blank node.
        let thing_id = store
            .lookup("http://example.org/thing")
            .unwrap()
            .unwrap();
        let facts = store.entity_facts(thing_id).unwrap();
        assert_eq!(facts[0].value, Value::Ref(bnode_id));
    }
}

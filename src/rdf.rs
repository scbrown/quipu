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
fn intern_subject(store: &Store, subject: &NamedOrBlankNode) -> Result<i64> {
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
pub(crate) fn value_to_term(store: &Store, value: &Value) -> Result<OxTerm> {
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
        assert_eq!(
            height_fact.value,
            Value::Typed {
                lexical: "1.65E0".into(),
                datatype: namespace::XSD_DOUBLE.into(),
            }
        );

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

        // Verify language-tagged literal. The LEXICAL value is "Bob" and the
        // tag lives beside it. This assertion used to read Str("Bob@en") — it
        // pinned the aegis-fmyi corruption in place as expected behavior.
        let bob_facts = store.entity_facts(bob_id).unwrap();
        let name_id = store.lookup("http://example.org/name").unwrap().unwrap();
        let bob_name = bob_facts.iter().find(|f| f.attribute == name_id).unwrap();
        assert_eq!(
            bob_name.value,
            Value::Lang {
                lexical: "Bob".into(),
                lang: "en".into()
            }
        );
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
                    "3.25",
                    NamedNode::new_unchecked(format!("{xsd}double")),
                ),
                Value::Typed {
                    lexical: "3.25E0".into(),
                    datatype: format!("{xsd}double"),
                },
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
                Value::Lang {
                    lexical: "hola".into(),
                    lang: "es".into(),
                },
            ),
            // The plain string "hola@es" must NOT collapse onto the lang literal
            // above. These two lines are the whole of aegis-fmyi.
            (
                Literal::new_simple_literal("hola@es"),
                Value::Str("hola@es".into()),
            ),
            // Datatypes without a fast-path variant keep their IRI verbatim
            // instead of decaying into an untyped string.
            (
                Literal::new_typed_literal(
                    "2026-07-15",
                    NamedNode::new_unchecked(format!("{xsd}date")),
                ),
                Value::Typed {
                    lexical: "2026-07-15".into(),
                    datatype: format!("{xsd}date"),
                },
            ),
            // xsd:decimal is EXACT and xsd:double is not; collapsing both into
            // f64 was a silent change of numeric semantics.
            (
                Literal::new_typed_literal(
                    "3.25",
                    NamedNode::new_unchecked(format!("{xsd}decimal")),
                ),
                Value::Typed {
                    lexical: "3.25".into(),
                    datatype: format!("{xsd}decimal"),
                },
            ),
            (
                Literal::new_typed_literal("42", NamedNode::new_unchecked(format!("{xsd}long"))),
                Value::Typed {
                    lexical: "42".into(),
                    datatype: format!("{xsd}long"),
                },
            ),
        ];

        for (lit, expected) in cases {
            let result = super::literal_to_value(&lit).unwrap();
            assert_eq!(result, expected, "failed for literal: {lit}");
        }
    }

    #[test]
    fn ingest_same_rdf_twice_no_duplicates() {
        let mut store = Store::open_in_memory().unwrap();

        let (_, count1) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            Some("first"),
            Some("test-1"),
        )
        .unwrap();
        assert_eq!(count1, 8);
        assert_eq!(store.current_facts().unwrap().len(), 8);

        // Second ingestion of the same data — assertions are idempotent,
        // so no new facts should be created.
        let (_, count2) = ingest_rdf(
            &mut store,
            TURTLE_DATA.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T01:00:00Z",
            Some("second"),
            Some("test-2"),
        )
        .unwrap();

        // count2 is the number of triples parsed from the input, not written.
        assert_eq!(count2, 8);

        // But the store should still have only 8 active facts.
        assert_eq!(
            store.current_facts().unwrap().len(),
            8,
            "re-ingesting the same RDF should not create duplicates"
        );
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
        let bnode_id = store
            .lookup("_:node1")
            .unwrap()
            .expect("blank node interned");
        assert!(bnode_id > 0);

        // The reference should point to the blank node.
        let thing_id = store.lookup("http://example.org/thing").unwrap().unwrap();
        let facts = store.entity_facts(thing_id).unwrap();
        assert_eq!(facts[0].value, Value::Ref(bnode_id));
    }
}

/// aegis-fmyi acceptance: RDF term fidelity across a full ingest → store →
/// export round trip.
///
/// These assert the OBSERVED EFFECT of a round trip through the real store, not
/// the behavior of `literal_to_value` in isolation — the defect was that a term
/// survived parsing looking plausible and came back out wrong.
#[cfg(test)]
mod term_fidelity_tests {
    use super::*;
    use crate::store::Store;

    const FIDELITY_TTL: &str = r#"
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:s ex:greeting "hello"@en ;
     ex:lookalike "hello@en" ;
     ex:when "2026-07-15"^^xsd:date ;
     ex:exact "0.1"^^xsd:decimal ;
     ex:approx "0.1"^^xsd:double ;
     ex:custom "abc"^^ex:MyType .
"#;

    fn ingested() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store,
            FIDELITY_TTL.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-07-15T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    fn value_of(store: &Store, pred: &str) -> Value {
        let s = store.lookup("http://example.org/s").unwrap().unwrap();
        let p = store
            .lookup(&format!("http://example.org/{pred}"))
            .unwrap()
            .unwrap();
        store
            .entity_facts(s)
            .unwrap()
            .into_iter()
            .find(|f| f.attribute == p)
            .unwrap_or_else(|| panic!("no fact for ex:{pred}"))
            .value
    }

    /// The headline criterion: the lexical value is "hello", NOT "hello@en".
    #[test]
    fn lang_tag_is_not_mangled_into_the_lexical_value() {
        let store = ingested();
        let v = value_of(&store, "greeting");
        assert_eq!(
            v,
            Value::Lang {
                lexical: "hello".into(),
                lang: "en".into()
            }
        );
        // Stated separately because THIS is the consumer-visible harm: anyone
        // asking for the value got the tag glued onto it.
        assert_eq!(v.as_lexical(), Some("hello"));
        assert_eq!(v.language(), Some("en"));
    }

    /// `"hello"@en` and the plain string `hello@en` must not collapse. This is
    /// the criterion that no serializer change could ever have met: once both
    /// became `Str("hello@en")` the distinction was gone from the database.
    #[test]
    fn lang_literal_and_lookalike_string_stay_distinguishable() {
        let store = ingested();
        let tagged = value_of(&store, "greeting");
        let plain = value_of(&store, "lookalike");

        assert_ne!(tagged, plain);
        assert_eq!(plain, Value::Str("hello@en".into()));
        assert_eq!(plain.language(), None);

        // …and they stay distinct on the way back out to RDF.
        let out = String::from_utf8(export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
        assert!(out.contains(r#""hello"@en"#), "lang literal lost: {out}");
        assert!(out.contains(r#""hello@en""#), "plain string lost: {out}");
    }

    #[test]
    fn xsd_date_round_trips_with_its_datatype_iri() {
        let store = ingested();
        assert_eq!(
            value_of(&store, "when"),
            Value::Typed {
                lexical: "2026-07-15".into(),
                datatype: "http://www.w3.org/2001/XMLSchema#date".into()
            }
        );
        let out = String::from_utf8(export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
        assert!(
            out.contains(r#""2026-07-15"^^<http://www.w3.org/2001/XMLSchema#date>"#),
            "xsd:date datatype lost on export: {out}"
        );
    }

    /// xsd:decimal is exact; xsd:double is not. Collapsing both into f64 was a
    /// silent semantic change, not a formatting choice.
    #[test]
    fn decimal_and_double_stay_distinguishable() {
        let store = ingested();
        let exact = value_of(&store, "exact");
        let approx = value_of(&store, "approx");

        assert_eq!(exact.datatype(), Some(namespace::XSD_DECIMAL));
        assert_eq!(approx.datatype(), Some(namespace::XSD_DOUBLE));
        assert_ne!(exact, approx);
        // Both still read as numbers, so ORDER BY / SUM / FILTER keep working.
        assert_eq!(exact.as_f64(), Some(0.1));
        assert_eq!(approx.as_f64(), Some(0.1));
    }

    #[test]
    fn custom_datatype_survives() {
        let store = ingested();
        assert_eq!(
            value_of(&store, "custom"),
            Value::Typed {
                lexical: "abc".into(),
                datatype: "http://example.org/MyType".into()
            }
        );
    }

    /// A plain string is NEVER sniffed for a trailing "@xx" on the way out.
    /// Recovering a tag that way is the trap the bead calls out: it would
    /// manufacture `"hello"@en` from a string nobody tagged.
    #[test]
    fn plain_string_is_not_sniffed_into_a_lang_literal() {
        let mut store = Store::open_in_memory().unwrap();
        let term = value_to_term(&store, &Value::Str("hello@en".into())).unwrap();
        assert_eq!(
            term,
            OxTerm::Literal(Literal::new_simple_literal("hello@en"))
        );
        // …and not into a datatype either.
        let term = value_to_term(&store, &Value::Str("2026-07-15".into())).unwrap();
        assert_eq!(
            term,
            OxTerm::Literal(Literal::new_simple_literal("2026-07-15"))
        );
        let _ = &mut store;
    }

    /// Storage encoding: the length-delimited framing must survive a lexical
    /// form that itself contains the delimiter-ish characters.
    #[test]
    fn tagged_literals_survive_the_blob_encoding() {
        for v in [
            Value::Lang {
                lexical: "a@b@c".into(),
                lang: "en-GB".into(),
            },
            Value::Typed {
                lexical: String::new(),
                datatype: "http://example.org/T".into(),
            },
            Value::Typed {
                lexical: "^^<not an iri>".into(),
                datatype: "http://example.org/T".into(),
            },
        ] {
            assert_eq!(
                Value::from_bytes(&v.to_bytes()).unwrap(),
                v,
                "round trip: {v:?}"
            );
        }
    }

    // --- Subset export (quipu #36) ------------------------------------------

    fn store_with_named_graph() -> (Store, &'static str) {
        use crate::types::{Op, Value};
        let mut store = Store::open_in_memory().unwrap();
        let ts = "2026-04-04T00:00:00Z";
        ingest_rdf(
            &mut store,
            "@prefix ex: <http://example.org/> .\nex:root ex:p \"ROOTVAL\" .\n".as_bytes(),
            RdfFormat::Turtle,
            None,
            ts,
            None,
            None,
        )
        .unwrap();
        let g_iri = "http://example.org/g/t1";
        let g = store.overlay_create(g_iri, 0).unwrap();
        let a = store.intern("http://example.org/a").unwrap();
        let p = store.intern("http://example.org/p").unwrap();
        let y = store.intern("http://example.org/y").unwrap();
        store
            .overlay_write(g, Op::Assert, a, p, Value::Ref(y), ts)
            .unwrap();
        (store, g_iri)
    }

    #[test]
    fn export_subset_scopes_to_named_graph() {
        let (store, g_iri) = store_with_named_graph();

        // The named-graph slice: only its own triple.
        let (bytes, count) = export_rdf_subset(&store, RdfFormat::NTriples, Some(g_iri)).unwrap();
        let doc = String::from_utf8(bytes).unwrap();
        assert_eq!(count, 1);
        assert!(doc.contains("http://example.org/a"));
        assert!(doc.contains("http://example.org/y"));
        assert!(
            !doc.contains("ROOTVAL"),
            "the ROOT graph must be excluded from a named-graph subset: {doc}"
        );

        // The ROOT default (graph omitted): only ROOT.
        let (rbytes, rcount) = export_rdf_subset(&store, RdfFormat::NTriples, None).unwrap();
        let rdoc = String::from_utf8(rbytes).unwrap();
        assert_eq!(rcount, 1);
        assert!(rdoc.contains("ROOTVAL"));
        assert!(
            !rdoc.contains("http://example.org/a"),
            "the named graph must be excluded from the ROOT export"
        );
    }

    #[test]
    fn export_subset_unknown_graph_errors() {
        let (store, _g) = store_with_named_graph();
        assert!(
            export_rdf_subset(
                &store,
                RdfFormat::NTriples,
                Some("http://example.org/g/nope")
            )
            .is_err(),
            "a targeted export of a non-existent graph is an error, not an empty success"
        );
    }

    #[test]
    fn export_subset_round_trips() {
        let (store, g_iri) = store_with_named_graph();
        // Export the slice, re-ingest it into a fresh store's ROOT, get it back.
        let (bytes, _) = export_rdf_subset(&store, RdfFormat::Turtle, Some(g_iri)).unwrap();
        let mut store2 = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store2,
            bytes.as_slice(),
            RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        let facts = store2.current_facts().unwrap();
        assert_eq!(
            facts.len(),
            1,
            "the exported slice re-ingests to exactly its triples"
        );
    }
}

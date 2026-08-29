//! Scoped and canonical RDF export helpers.

use std::collections::{BTreeSet, HashSet};

use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Triple};
use oxrdfio::{RdfFormat, RdfParser, RdfSerializer};

use crate::error::{Error, Result};
use crate::rdf::{BLANK_PREFIX, serialize_facts, value_to_term};
use crate::store::Store;
use crate::types::Value;

pub(crate) fn serialize_triples_canonical(
    triples: &[Triple],
    format: RdfFormat,
) -> Result<Vec<u8>> {
    let mut lines = BTreeSet::new();
    for triple in triples {
        let mut serializer = RdfSerializer::from_format(RdfFormat::NTriples).for_writer(Vec::new());
        serializer
            .serialize_triple(triple)
            .map_err(|e| Error::InvalidValue(format!("RDF serialization error: {e}")))?;
        lines.insert(
            serializer
                .finish()
                .map_err(|e| Error::InvalidValue(format!("RDF serialization finish error: {e}")))?,
        );
    }
    if format == RdfFormat::NTriples {
        return Ok(lines.into_iter().flatten().collect());
    }
    let mut buf = Vec::new();
    let mut serializer = RdfSerializer::from_format(format).for_writer(&mut buf);
    for line in lines {
        for quad in RdfParser::from_format(RdfFormat::NTriples).for_reader(line.as_slice()) {
            let triple = Triple::from(
                quad.map_err(|e| Error::InvalidValue(format!("canonical RDF parse: {e}")))?,
            );
            serializer
                .serialize_triple(&triple)
                .map_err(|e| Error::InvalidValue(format!("RDF serialization error: {e}")))?;
        }
    }
    serializer
        .finish()
        .map_err(|e| Error::InvalidValue(format!("RDF serialization finish error: {e}")))?;
    Ok(buf)
}

/// Export ROOT facts attributed to an episode group id.
pub fn export_rdf_group(
    store: &Store,
    format: RdfFormat,
    group_id: &str,
) -> Result<(Vec<u8>, usize)> {
    let escaped = group_id
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    let query = format!(
        "SELECT DISTINCT ?s ?ep WHERE {{ ?ep <{}groupId> \"{escaped}\" . ?s <{}wasGeneratedBy> ?ep . }}",
        store.base_ns(),
        crate::namespace::PROV
    );
    let result = crate::sparql::query(store, &query)?;
    let mut subjects = HashSet::new();
    for row in result.rows() {
        for name in ["s", "ep"] {
            if let Some(Value::Ref(id)) = row.get(name) {
                subjects.insert(*id);
            }
        }
    }
    let mut facts = Vec::new();
    for subject in subjects {
        facts.extend(store.entity_facts(subject)?);
    }
    let count = facts.len();
    Ok((serialize_facts(store, &facts, format)?, count))
}

/// Export the graph produced by a SPARQL CONSTRUCT query.
pub fn export_rdf_construct(
    store: &Store,
    format: RdfFormat,
    query: &str,
) -> Result<(Vec<u8>, usize)> {
    match crate::sparql::query(store, query)? {
        crate::sparql::QueryResult::Graph(triples) => {
            let count = triples.len();
            let graph: Result<Vec<Triple>> = triples
                .into_iter()
                .map(|t| {
                    let subject = if let Some(label) = t.subject.strip_prefix(BLANK_PREFIX) {
                        NamedOrBlankNode::BlankNode(BlankNode::new(label).map_err(|e| {
                            Error::InvalidValue(format!("invalid CONSTRUCT blank node: {e}"))
                        })?)
                    } else {
                        NamedOrBlankNode::NamedNode(NamedNode::new(t.subject).map_err(|e| {
                            Error::InvalidValue(format!("invalid CONSTRUCT subject: {e}"))
                        })?)
                    };
                    Ok(Triple {
                        subject,
                        predicate: NamedNode::new(t.predicate).map_err(|e| {
                            Error::InvalidValue(format!("invalid CONSTRUCT predicate: {e}"))
                        })?,
                        object: value_to_term(store, &t.object)?,
                    })
                })
                .collect();
            Ok((serialize_triples_canonical(&graph?, format)?, count))
        }
        _ => Err(Error::InvalidValue(
            "export query must be SPARQL CONSTRUCT or DESCRIBE".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{export_rdf, ingest_rdf};

    #[test]
    fn canonical_export_is_independent_of_interning_order() {
        let a = b"<http://example.org/z> <http://example.org/p> \"last\" .\n<http://example.org/a> <http://example.org/p> \"first\" .\n";
        let b = b"<http://example.org/a> <http://example.org/p> \"first\" .\n<http://example.org/z> <http://example.org/p> \"last\" .\n";
        let mut left = Store::open_in_memory().unwrap();
        let mut right = Store::open_in_memory().unwrap();
        for (store, data) in [(&mut left, &a[..]), (&mut right, &b[..])] {
            ingest_rdf(
                store,
                data,
                RdfFormat::NTriples,
                None,
                "2026-01-01",
                None,
                None,
            )
            .unwrap();
        }
        let l = export_rdf(&left, RdfFormat::NTriples).unwrap();
        let r = export_rdf(&right, RdfFormat::NTriples).unwrap();
        assert_eq!(l, r);
        assert!(
            String::from_utf8(l)
                .unwrap()
                .starts_with("<http://example.org/a>")
        );
    }

    #[test]
    fn group_export_includes_only_attributed_entities_and_episode() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_base_ns("http://example.org/");
        let data = br#"@prefix ex: <http://example.org/> . @prefix prov: <http://www.w3.org/ns/prov#> . ex:ep1 ex:groupId "wanted" . ex:ep2 ex:groupId "other" . ex:alice prov:wasGeneratedBy ex:ep1 ; ex:name "Alice" . ex:bob prov:wasGeneratedBy ex:ep2 ; ex:name "Bob" ."#;
        ingest_rdf(
            &mut store,
            &data[..],
            RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();
        let (bytes, count) = export_rdf_group(&store, RdfFormat::NTriples, "wanted").unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(count >= 3 && text.contains("alice") && text.contains("ep1"));
        assert!(!text.contains("bob") && !text.contains("ep2"));
    }

    #[test]
    fn construct_export_requires_a_graph_result_and_is_sorted() {
        let mut store = Store::open_in_memory().unwrap();
        let data = b"<http://example.org/z> <http://example.org/name> \"Z\" .\n<http://example.org/a> <http://example.org/name> \"A\" .\n";
        ingest_rdf(
            &mut store,
            &data[..],
            RdfFormat::NTriples,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();
        let query = "CONSTRUCT { ?s <http://example.org/label> ?n } WHERE { ?s <http://example.org/name> ?n }";
        let (bytes, count) = export_rdf_construct(&store, RdfFormat::NTriples, query).unwrap();
        assert_eq!(count, 2);
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .starts_with("<http://example.org/a>")
        );
        assert!(
            export_rdf_construct(&store, RdfFormat::NTriples, "SELECT ?s WHERE { ?s ?p ?o }")
                .is_err()
        );
    }
}

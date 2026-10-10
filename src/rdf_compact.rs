//! Lossless, bounded dictionary-Turtle transport for one complete named graph.
//!
//! Each IRI has an empty-local-name prefix; literals and blank nodes retain their
//! ordinary RDF lexical encoding. This reduces repeated IRIs without compression
//! or an expanded-byte budget exception.
use std::collections::{BTreeMap, BTreeSet};

use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest, Sha256};

use crate::{Error, Result, Store, Value, rdf::value_to_term};

pub const COMPACT_RDF_LIMIT: usize = 256 * 1024 * 1024;

pub struct CompactRdf {
    pub bytes: Vec<u8>,
    pub triples: usize,
    pub actions: usize,
    pub sha256: String,
}

fn append(bytes: &mut Vec<u8>, text: &str, limit: usize) -> Result<()> {
    if text.len() > limit.saturating_sub(bytes.len()) {
        return Err(Error::InvalidValue(
            "compact RDF exceeds byte budget".into(),
        ));
    }
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

fn named(node: &NamedNode, dictionary: &BTreeMap<String, usize>) -> String {
    format!("n{}:", dictionary[node.as_str()])
}

fn object(term: &Term, dictionary: &BTreeMap<String, usize>) -> Result<String> {
    match term {
        Term::NamedNode(node) => Ok(named(node, dictionary)),
        Term::BlankNode(node) => Ok(node.to_string()),
        Term::Literal(literal) => {
            let text = literal.to_string();
            let suffix = format!("^^{}", literal.datatype());
            if let Some(value) = text.strip_suffix(&suffix) {
                Ok(format!(
                    "{value}^^n{}:",
                    dictionary[literal.datatype().as_str()]
                ))
            } else {
                Ok(text)
            }
        }
        #[cfg(feature = "shacl")]
        Term::Triple(_) => Err(Error::InvalidValue("unsupported compact RDF term".into())),
    }
}

fn serialize(triples: &[Triple], limit: usize) -> Result<CompactRdf> {
    let mut iris = BTreeSet::new();
    for triple in triples {
        if let NamedOrBlankNode::NamedNode(node) = &triple.subject {
            iris.insert(node.as_str().to_owned());
        }
        iris.insert(triple.predicate.as_str().to_owned());
        match &triple.object {
            Term::NamedNode(node) => {
                iris.insert(node.as_str().to_owned());
            }
            Term::Literal(literal) => {
                iris.insert(literal.datatype().as_str().to_owned());
            }
            Term::BlankNode(_) => {}
            #[cfg(feature = "shacl")]
            Term::Triple(_) => {
                return Err(Error::InvalidValue("unsupported compact RDF term".into()));
            }
        }
    }
    let dictionary: BTreeMap<_, _> = iris
        .into_iter()
        .enumerate()
        .map(|(n, iri)| (iri, n))
        .collect();
    let mut bytes = Vec::new();
    for (iri, index) in &dictionary {
        let node = NamedNode::new(iri.clone()).map_err(|e| Error::InvalidValue(e.to_string()))?;
        append(&mut bytes, &format!("@prefix n{index}: {node} .\n"), limit)?;
    }
    let mut rows = BTreeSet::new();
    let mut row_bytes = 0usize;
    let mut action_rows = BTreeSet::new();
    for triple in triples {
        let subject = match &triple.subject {
            NamedOrBlankNode::NamedNode(node) => named(node, &dictionary),
            NamedOrBlankNode::BlankNode(node) => node.to_string(),
        };
        let row = format!(
            "{subject} {} {} .\n",
            named(&triple.predicate, &dictionary),
            object(&triple.object, &dictionary)?
        );
        if triple.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
            && matches!(&triple.object, Term::NamedNode(node) if node.as_str() == "https://schema.org/Action")
        {
            action_rows.insert(row.clone());
        }
        if !rows.contains(&row) {
            if row.len() > limit.saturating_sub(bytes.len()).saturating_sub(row_bytes) {
                return Err(Error::InvalidValue(
                    "compact RDF exceeds byte budget".into(),
                ));
            }
            row_bytes += row.len();
            rows.insert(row);
        }
    }
    for row in &rows {
        append(&mut bytes, row, limit)?;
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(CompactRdf {
        bytes,
        triples: rows.len(),
        actions: action_rows.len(),
        sha256,
    })
}

/// Serialize all asserted facts of exactly one named graph; unknown scope fails.
pub fn export_compact_graph(store: &Store, graph: &str) -> Result<CompactRdf> {
    let id = store
        .lookup(graph)?
        .ok_or_else(|| Error::InvalidValue("unknown compact RDF graph".into()))?;
    if store.graph_class(id)?.is_none() {
        return Err(Error::InvalidValue(
            "compact RDF scope is not a named graph".into(),
        ));
    }
    let facts = store.current_facts_in_graph(id)?;
    let mut triples = Vec::with_capacity(facts.len());
    for fact in facts {
        let subject = match value_to_term(store, &Value::Ref(fact.entity))? {
            Term::NamedNode(node) => NamedOrBlankNode::NamedNode(node),
            Term::BlankNode(node) => NamedOrBlankNode::BlankNode(node),
            Term::Literal(_) => return Err(Error::InvalidValue("literal RDF subject".into())),
            #[cfg(feature = "shacl")]
            Term::Triple(_) => {
                return Err(Error::InvalidValue(
                    "unsupported compact RDF subject".into(),
                ));
            }
        };
        triples.push(Triple {
            subject,
            predicate: NamedNode::new(store.resolve(fact.attribute)?)
                .map_err(|e| Error::InvalidValue(e.to_string()))?,
            object: value_to_term(store, &fact.value)?,
        });
    }
    serialize(&triples, COMPACT_RDF_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdfio::{RdfFormat, RdfParser};
    #[test]
    fn compact_roundtrip_and_budget() {
        let source = concat!(
            "<https://example.org/long/subject> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <https://schema.org/Action> .\n",
            "<https://example.org/long/subject> <https://example.org/long/predicate> \"literal <https://example.org/long/subject> and \\\\ escape\" .\n",
            "_:blank <https://example.org/long/predicate> \"bonjour\"@fr .\n",
            "_:blank <https://example.org/long/predicate> \"001\"^^<https://example.org/datatype> .\n"
        );
        let triples: Vec<_> = RdfParser::from_format(RdfFormat::NTriples)
            .for_reader(source.as_bytes())
            .map(|q| Triple::from(q.unwrap()))
            .collect();
        let compact = serialize(&triples, COMPACT_RDF_LIMIT).unwrap();
        let decoded: BTreeSet<_> = RdfParser::from_format(RdfFormat::Turtle)
            .for_reader(compact.bytes.as_slice())
            .map(|q| Triple::from(q.unwrap()).to_string())
            .collect();
        // Compare named/literal assertions independently of parser blank-node allocation.
        assert_eq!(decoded, triples.iter().map(ToString::to_string).collect());
        assert_eq!(decoded.len(), triples.len());
        assert!(
            decoded
                .iter()
                .any(|q| q.contains("bonjour") && q.contains("@fr"))
        );
        assert!(
            decoded
                .iter()
                .any(|q| q.contains("\"001\"") && q.contains("https://example.org/datatype"))
        );
        assert_eq!(compact.triples, 4);
        assert_eq!(compact.actions, 1);
        assert!(serialize(&triples, compact.bytes.len() - 1).is_err());
        assert_eq!(
            serialize(&triples, compact.bytes.len()).unwrap().bytes,
            compact.bytes
        );
    }
    #[test]
    fn repeated_iris_are_smaller_without_losing_assertions() {
        let triples: Vec<_> = (0..100)
            .flat_map(|subject| {
                (0..20).map(move |predicate| {
                    Triple::new(
                        NamedNode::new(format!(
                            "https://example.org/application/records/{subject}"
                        ))
                        .unwrap(),
                        NamedNode::new(format!(
                            "https://example.org/application/properties/{predicate}"
                        ))
                        .unwrap(),
                        NamedNode::new("https://example.org/application/common-value").unwrap(),
                    )
                })
            })
            .collect();
        let baseline =
            crate::rdf_export::serialize_triples_canonical(&triples, RdfFormat::NTriples).unwrap();
        let compact = serialize(&triples, COMPACT_RDF_LIMIT).unwrap();
        let decoded: BTreeSet<_> = RdfParser::from_format(RdfFormat::Turtle)
            .for_reader(compact.bytes.as_slice())
            .map(|q| Triple::from(q.unwrap()).to_string())
            .collect();
        assert_eq!(decoded, triples.iter().map(ToString::to_string).collect());
        assert!(compact.bytes.len() < baseline.len() / 2);
        println!(
            "exact2000triples NTriples{} -> compact{}bytes",
            baseline.len(),
            compact.bytes.len()
        );
    }
}

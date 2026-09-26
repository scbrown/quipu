//! Blank-node identity for one complete ingestion document.
use crate::{Error, Result, Store};
use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom, Write},
};

#[cfg(not(target_arch = "wasm32"))]
type Spool = std::fs::File;
#[cfg(target_arch = "wasm32")]
type Spool = std::io::Cursor<Vec<u8>>;

pub(crate) struct Scope {
    prefix: String,
    nodes: HashMap<BlankNode, BlankNode>,
}

/// Spool before parsing so chunk boundaries cannot change document identity.
/// Memory is bounded by the copy buffer and distinct blank-node map, not input size.
pub(crate) fn prepare(
    store: &Store,
    mut reader: impl Read,
    graph: i64,
    explicit: Option<&str>,
) -> Result<(Spool, Scope)> {
    if explicit == Some("") {
        return Err(Error::InvalidValue(
            "blank-node scope must not be empty".into(),
        ));
    }
    let io_error = |e| Error::InvalidValue(format!("RDF document spool: {e}"));
    #[cfg(not(target_arch = "wasm32"))]
    let mut spool = tempfile::tempfile().map_err(io_error)?;
    #[cfg(target_arch = "wasm32")]
    let mut spool = std::io::Cursor::new(Vec::new());
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        spool.write_all(&buffer[..n]).map_err(io_error)?;
    }
    let document = hash.finalize();
    let destination = if graph == 0 {
        "ROOT".into()
    } else {
        store.resolve(graph)?
    };
    let mut hash = Sha256::new();
    hash.update(b"quipu-document-bnodes-v1\0");
    hash.update(document);
    hash.update(if explicit.is_some() {
        b"explicit\0".as_slice()
    } else {
        b"graph\0".as_slice()
    });
    let key = explicit.unwrap_or(&destination);
    hash.update((key.len() as u64).to_be_bytes());
    hash.update(key.as_bytes());
    let prefix = format!("q{:x}", hash.finalize());
    spool.seek(SeekFrom::Start(0)).map_err(io_error)?;
    Ok((
        spool,
        Scope {
            prefix,
            nodes: HashMap::new(),
        },
    ))
}

impl Scope {
    fn node(&mut self, node: BlankNode) -> BlankNode {
        let next = self.nodes.len();
        self.nodes
            .entry(node)
            .or_insert_with(|| BlankNode::new_unchecked(format!("{}_{}", self.prefix, next)))
            .clone()
    }

    pub(crate) fn triple(&mut self, triple: Triple) -> Triple {
        let subject = match triple.subject {
            NamedOrBlankNode::BlankNode(node) => self.node(node).into(),
            other @ NamedOrBlankNode::NamedNode(_) => other,
        };
        let object = match triple.object {
            Term::BlankNode(node) => self.node(node).into(),
            #[cfg(feature = "shacl")]
            Term::Triple(triple) => Term::Triple(Box::new(self.triple(*triple))),
            other => other,
        };
        Triple {
            subject,
            predicate: triple.predicate,
            object,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assigned_transport_is_not_rescoped_by_import_or_delta_parsing() {
        let mut store = Store::open_in_memory().unwrap();
        let text = b"_:assigned <http://e/p> _:other .";
        let graph = store.graph_create("http://e/staging").unwrap();
        crate::rdf::ingest_assigned_rdf(
            &mut store,
            text.as_slice(),
            oxrdfio::RdfFormat::NTriples,
            None,
            "2026-01-01",
            None,
            None,
            graph,
        )
        .unwrap();
        let parsed = crate::rdf::parse_rdf(
            &store,
            text.as_slice(),
            oxrdfio::RdfFormat::NTriples,
            None,
            "2026-01-01",
        )
        .unwrap();
        let fact = &store.current_facts_in_graph(graph).unwrap()[0];
        assert_eq!(fact.entity, parsed[0].entity);
        assert_eq!(fact.value, parsed[0].value);
        assert_eq!(store.resolve(fact.entity).unwrap(), "_:assigned");
    }
}

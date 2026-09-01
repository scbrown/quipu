//! Fact-log operations used by the SPARQL Graph Store HTTP adapter.

use std::io::Read;

use oxrdfio::RdfFormat;

use crate::{Error, Result, Store};

/// Replace one graph's current contents with an RDF payload in one fact-log
/// transaction. `graph=None` targets the default graph; named graphs are
/// registered as committed graphs when first created.
#[allow(clippy::too_many_arguments)]
pub fn replace_rdf_graph(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: Option<&str>,
) -> Result<(i64, usize)> {
    let mut datums = crate::rdf::parse_rdf(store, reader, format, base_iri, timestamp)?;
    let g = match graph {
        None => 0,
        Some(iri) => store.graph_create(iri)?,
    };
    let current = store.current_facts_in_graph(g)?;
    let inserted = datums.len();
    let mut changes = Vec::with_capacity(current.len() + inserted);
    changes.extend(current.into_iter().map(|fact| crate::store::Datum {
        entity: fact.entity,
        attribute: fact.attribute,
        value: fact.value,
        valid_from: timestamp.to_string(),
        valid_to: None,
        op: crate::types::Op::Retract,
    }));
    changes.append(&mut datums);
    let tx_id = store.transact_to_graph(&changes, timestamp, actor, source, g)?;
    Ok((tx_id, inserted))
}

/// Remove every current statement from one graph. Named graph registration is
/// removed too, so a following protocol GET observes 404 rather than an empty
/// graph that still exists.
pub fn delete_rdf_graph(
    store: &mut Store,
    timestamp: &str,
    actor: Option<&str>,
    graph: Option<&str>,
) -> Result<(i64, usize)> {
    let g = match graph {
        None => 0,
        Some(iri) => store
            .lookup(iri)?
            .filter(|id| store.graph_class(*id).ok().flatten().is_some())
            .ok_or_else(|| Error::InvalidValue(format!("unknown graph: {iri}")))?,
    };
    let current = store.current_facts_in_graph(g)?;
    let count = current.len();
    let changes: Vec<_> = current
        .into_iter()
        .map(|fact| crate::store::Datum {
            entity: fact.entity,
            attribute: fact.attribute,
            value: fact.value,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: crate::types::Op::Retract,
        })
        .collect();
    let tx_id =
        store.transact_to_graph(&changes, timestamp, actor, Some("graph-store-delete"), g)?;
    if graph.is_some() {
        store.graph_unregister(g)?;
    }
    Ok((tx_id, count))
}

#[cfg(test)]
mod tests {
    use oxrdfio::RdfFormat;

    use crate::{Store, delete_rdf_graph, export_rdf_subset, replace_rdf_graph};

    #[test]
    fn replace_then_delete_round_trips_named_graph() {
        let mut store = Store::open_in_memory().unwrap();
        let graph = "http://example.org/graphs/interop";
        let first = b"<http://example.org/a> <http://example.org/p> \"one\" .";
        let second = b"<http://example.org/b> <http://example.org/p> \"two\" .";

        replace_rdf_graph(
            &mut store,
            first.as_slice(),
            RdfFormat::NTriples,
            Some(graph),
            "2026-09-01T00:00:00Z",
            Some("test"),
            Some("put"),
            Some(graph),
        )
        .unwrap();
        let (before, count) = export_rdf_subset(&store, RdfFormat::NTriples, Some(graph)).unwrap();
        assert_eq!(count, 1);
        assert!(String::from_utf8(before).unwrap().contains("/a>"));

        replace_rdf_graph(
            &mut store,
            second.as_slice(),
            RdfFormat::NTriples,
            Some(graph),
            "2026-09-01T00:00:01Z",
            Some("test"),
            Some("put"),
            Some(graph),
        )
        .unwrap();
        let (after, count) = export_rdf_subset(&store, RdfFormat::NTriples, Some(graph)).unwrap();
        let after = String::from_utf8(after).unwrap();
        assert_eq!(count, 1);
        assert!(after.contains("/b>"));
        assert!(!after.contains("/a>"));

        delete_rdf_graph(
            &mut store,
            "2026-09-01T00:00:02Z",
            Some("test"),
            Some(graph),
        )
        .unwrap();
        assert!(
            store.lookup(graph).unwrap().is_some(),
            "term history remains interned"
        );
        let g = store.lookup(graph).unwrap().unwrap();
        assert_eq!(
            store.graph_class(g).unwrap(),
            None,
            "registry entry is removed"
        );
    }
}

//! SQLite hybrid candidate scoring; delegated/LanceDB behavior is preserved.
use std::collections::BTreeSet;

use crate::vector::VectorMatch;
use crate::{Error, Result, Store};

pub(super) fn matches(
    store: &Store,
    embedding: &[f32],
    limit: usize,
    pushdown: Option<&str>,
    valid_at: Option<&str>,
    scope: Option<&[String]>,
) -> Result<Vec<VectorMatch>> {
    if let Some(iris) = scope.filter(|_| store.has_sqlite_vector_backend()) {
        if iris.len() > 1000 {
            return Err(Error::InvalidValue(
                "narrow the SPARQL scope: more than 1000 candidate rows".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for iri in iris {
            ids.extend(store.lookup_all(iri)?);
            if ids.len() > 1000 {
                return Err(Error::InvalidValue(
                    "narrow the SPARQL scope: more than 1000 entities".into(),
                ));
            }
        }
        return store.vector_search_scoped_sqlite(
            embedding,
            &ids.into_iter().collect::<Vec<_>>(),
            limit,
            valid_at,
        );
    }
    store
        .vector_store()
        .vector_search_filtered(embedding, limit, pushdown, valid_at)
}

pub(super) fn request_budget(store: &Store) -> crate::time::RequestDeadlineGuard {
    let deadline = crate::time::request_deadline().or_else(|| {
        let ms = store.search_config().query_timeout_ms;
        (ms > 0).then(|| crate::time::Deadline::after_millis(ms))
    });
    crate::time::set_request_deadline(deadline)
}

pub(super) fn candidates(
    store: &Store,
    sparql: &str,
    valid_at: Option<&str>,
) -> Result<Vec<String>> {
    let context = crate::sparql::TemporalContext {
        valid_at: valid_at.map(str::to_owned),
        row_cap: Some(1000),
        result_limit: Some(1001),
        ..Default::default()
    };
    let result = crate::sparql::query_temporal(store, sparql, &context)?;
    if result.rows().len() > 1000 {
        return Err(Error::InvalidValue(
            "narrow the SPARQL scope: candidate result exceeds 1000 rows".into(),
        ));
    }
    let mut iris = Vec::new();
    for row in result.rows() {
        if let Some(first) = result.variables().first() {
            match row.get(first) {
                Some(crate::types::Value::Ref(id)) => iris.push(store.resolve(*id)?),
                Some(crate::types::Value::Str(iri)) => iris.push(iri.clone()),
                _ => {}
            }
        }
    }
    Ok(iris)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_matches_include_composed_iri_alias_ids() {
        use crate::vector::KnowledgeVectorStore;
        let scratch = tempfile::tempdir().unwrap();
        let raw = scratch.path().join("raw.db");
        let layer_path = scratch.path().join("layer.db");
        let main_path = scratch.path().join("main.db");
        let iri = "http://example.org/shared";
        {
            let layer = Store::open(raw.to_str().unwrap()).unwrap();
            layer.intern(iri).unwrap();
        }
        crate::store::respace::respace_file(&raw, &layer_path, 3).unwrap();
        {
            let main = Store::open(main_path.to_str().unwrap()).unwrap();
            main.intern(iri).unwrap();
        }
        let store = Store::open_with_attachments(
            main_path.to_str().unwrap(),
            &[crate::store::attach::Attachment::read_only(
                "layer",
                layer_path.to_str().unwrap(),
            )],
        )
        .unwrap();
        let ids = store.lookup_all(iri).unwrap();
        assert_eq!(ids.len(), 2);
        let alias = *ids
            .iter()
            .find(|id| **id != store.lookup(iri).unwrap().unwrap())
            .unwrap();
        store
            .embed_entity(alias, "aliased vector", &[1.0], "2026-01-01")
            .unwrap();
        let result = matches(
            &store,
            &[1.0],
            1,
            None,
            None,
            Some(&[iri.into(), iri.into()]),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].entity_id, alias);
        assert_eq!(result[0].text, "aliased vector");
    }

    #[test]
    fn candidate_scope_uses_historical_valid_time() {
        let mut store = Store::open_in_memory().unwrap();
        crate::rdf::ingest_rdf(
            &mut store,
            &b"<http://example.org/old> <http://example.org/kind> \"planned\" ."[..],
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();
        store
            .conn
            .execute("UPDATE facts SET valid_to='2026-02-01'", [])
            .unwrap();
        let query = "SELECT ?s WHERE { ?s <http://example.org/kind> \"planned\" }";
        assert!(candidates(&store, query, None).unwrap().is_empty());
        assert_eq!(
            candidates(&store, query, Some("2026-01-15")).unwrap(),
            vec!["http://example.org/old"]
        );
    }

    #[test]
    fn candidate_scope_refuses_materialization_over_cap() {
        let store = Store::open_in_memory().unwrap();
        let rows = (0..1001)
            .map(|i| format!("<http://example.org/{i}>"))
            .collect::<Vec<_>>()
            .join(" ");
        let query = format!("SELECT ?s WHERE {{ VALUES ?s {{ {rows} }} }}");
        assert!(candidates(&store, &query, None).is_err());
    }
}

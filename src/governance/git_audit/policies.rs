//! Load every path-scoped action policy; merge globs by IRI, never by label.
use super::invalid;
use crate::{
    Store,
    error::Result,
    namespace::DEFAULT_BASE_NS,
    sparql::{self, QueryResult},
    types::Value,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Policy {
    pub iri: String,
    pub id: String,
    pub globs: Vec<glob::Pattern>,
    pub effect: Option<String>,
    pub has_selector: bool,
}

pub(super) fn load(store: &Store, include_structural: bool) -> Result<Vec<Policy>> {
    let query = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
         SELECT ?p ?path ?label ?effect ?selector ?predicate WHERE {{ \
         ?p a a:Policy ; a:boundary \"action\" . OPTIONAL {{ ?p a:appliesTo ?path }} \
         OPTIONAL {{ ?p rdfs:label ?label }} OPTIONAL {{ ?p a:effect ?effect }} \
         OPTIONAL {{ ?p a:selector ?selector }} OPTIONAL {{ ?p a:predicate ?predicate }} }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &query)? else {
        return Err(invalid("policy query returned no SELECT result"));
    };
    let mut policies = BTreeMap::<String, Policy>::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let iri = text(store, row.get("p")).ok_or_else(|| invalid("policy IRI missing"))?;
        let has_selector = row.contains_key("selector") || row.contains_key("predicate");
        if !row.contains_key("path") && !(include_structural && has_selector) {
            continue;
        }
        let path = match row.get("path") {
            Some(Value::Str(s)) => Some(s.clone()),
            None => None,
            _ => {
                return Err(invalid(format!(
                    "{iri}: appliesTo must be a literal path glob"
                )));
            }
        };
        if let Some(path) = &path
            && (path.is_empty()
                || path.starts_with('/')
                || path.split('/').any(|p| p == ".." || p == "."))
        {
            return Err(invalid(format!(
                "{iri}: appliesTo must be repository-relative: {path:?}"
            )));
        }
        let pattern = path
            .as_ref()
            .map(|path| {
                glob::Pattern::new(path)
                    .map_err(|e| invalid(format!("{iri}: invalid glob {path:?}: {e}")))
            })
            .transpose()?;
        let id = text(store, row.get("label")).unwrap_or_else(|| {
            iri.rsplit(['#', '/', ':'])
                .next()
                .unwrap_or(&iri)
                .to_string()
        });
        let effect = text(store, row.get("effect"));
        let p = policies.entry(iri.clone()).or_insert_with(|| Policy {
            iri: iri.clone(),
            id: id.clone(),
            globs: Vec::new(),
            effect: effect.clone(),
            has_selector: false,
        });
        if p.id != id || p.effect != effect {
            return Err(invalid(format!("{iri}: ambiguous policy label/effect")));
        }
        if seen.insert((iri, path))
            && let Some(pattern) = pattern
        {
            p.globs.push(pattern);
        }
        p.has_selector |= row.contains_key("selector") || row.contains_key("predicate");
    }
    let mut labels = BTreeSet::new();
    for p in policies.values() {
        if !labels.insert(&p.id) {
            return Err(invalid(format!("ambiguous policy label {:?}", p.id)));
        }
    }
    Ok(policies.into_values().collect())
}

fn text(store: &Store, value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Ref(id)) => store.resolve(*id).ok(),
        _ => None,
    }
}

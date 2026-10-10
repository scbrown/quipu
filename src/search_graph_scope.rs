//! Explicit graph selection shared by lexical and semantic search.
use crate::{Error, Result, Store};
use serde_json::Value;

pub(crate) struct GraphScope {
    pub ids: Vec<i64>,
    pub explicit: bool,
}

#[cfg(test)]
mod tests;

impl GraphScope {
    pub fn parse(store: &Store, input: &Value) -> Result<Self> {
        let graph = input.get("graph").filter(|v| !v.is_null());
        let graphs = input.get("graphs");
        let all = match input.get("all_graphs") {
            None => false,
            Some(Value::Bool(v)) => *v,
            Some(_) => return Err(Error::InvalidValue("all_graphs must be boolean".into())),
        };
        if usize::from(graph.is_some()) + usize::from(graphs.is_some()) + usize::from(all) > 1 {
            return Err(Error::InvalidValue(
                "choose graph, graphs, or all_graphs, not multiple selectors".into(),
            ));
        }
        let mut ids = vec![0];
        let explicit = graph.is_some() || graphs.is_some() || all;
        if all || graph.and_then(Value::as_str) == Some("all") {
            ids.extend(store.all_named_graph_ids()?);
        } else if explicit {
            let iris = match (graph, graphs) {
                (Some(Value::String(iri)), None) => vec![iri.as_str()],
                (None, Some(Value::Array(iris))) if !iris.is_empty() => iris
                    .iter()
                    .map(|v| {
                        v.as_str().ok_or_else(|| {
                            Error::InvalidValue("graphs entries must be IRI strings".into())
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => {
                    return Err(Error::InvalidValue(
                        "graph must be an IRI string; graphs must be a nonempty IRI array".into(),
                    ));
                }
            };
            ids = iris
                .into_iter()
                .map(|iri| {
                    oxrdf::NamedNode::new(iri)
                        .map_err(|e| Error::InvalidValue(format!("invalid graph IRI: {e}")))?;
                    if iri == crate::schema::ROOT_GRAPH_IRI {
                        return Ok(0);
                    }
                    store.registered_graph_id(iri)?.ok_or_else(|| {
                        Error::InvalidValue(format!(
                            "unknown graph: {iri}; search refuses rather than searching ROOT"
                        ))
                    })
                })
                .collect::<Result<_>>()?;
        }
        ids.sort_unstable();
        ids.dedup();
        if explicit && !store.search_config().named_graphs {
            return Err(Error::InvalidValue("explicit graph search is disabled ([quipu.search] named_graphs = false); prepare bounded backfill before enabling".into()));
        }
        Ok(Self { ids, explicit })
    }

    pub fn matching(&self, store: &Store, entity: i64, at: Option<&str>) -> Result<Vec<i64>> {
        let mut out = Vec::new();
        for id in &self.ids {
            let found: bool = store.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM facts WHERE e=?1 AND g=?2 AND op=1 AND ((?3 IS NULL AND valid_to IS NULL) OR (?3 IS NOT NULL AND valid_from<=?3 AND (valid_to IS NULL OR valid_to>?3))))",
                rusqlite::params![entity,id,at], |r| r.get(0))?;
            if found {
                out.push(*id);
            }
        }
        Ok(out)
    }

    pub fn entities(
        &self,
        store: &Store,
        at: Option<&str>,
    ) -> Result<std::collections::HashSet<i64>> {
        let mut out = std::collections::HashSet::new();
        for id in &self.ids {
            let mut stmt = store.conn.prepare("SELECT DISTINCT e FROM facts WHERE g=?1 AND op=1 AND ((?2 IS NULL AND valid_to IS NULL) OR (?2 IS NOT NULL AND valid_from<=?2 AND (valid_to IS NULL OR valid_to>?2)))")?;
            for entity in stmt.query_map(rusqlite::params![id, at], |r| r.get::<_, i64>(0))? {
                out.insert(entity?);
            }
        }
        Ok(out)
    }

    pub fn names(&self, store: &Store, entity: i64, at: Option<&str>) -> Result<Vec<String>> {
        Ok(self
            .matching(store, entity, at)?
            .into_iter()
            .map(|id| store.graph_display_name(id))
            .collect())
    }

    pub fn patterns(&self, store: &Store, body: &str) -> Result<String> {
        self.ids
            .iter()
            .map(|id| {
                if *id == 0 {
                    Ok(format!("{{ {body} }}"))
                } else {
                    let iri = store.graph_display_name(*id);
                    let node = oxrdf::NamedNode::new(iri)
                        .map_err(|e| Error::InvalidValue(format!("invalid graph IRI: {e}")))?;
                    Ok(format!("{{ GRAPH {node} {{ {body} }} }}"))
                }
            })
            .collect::<Result<Vec<_>>>()
            .map(|parts| parts.join(" UNION "))
    }
}

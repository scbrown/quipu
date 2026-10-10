//! Decode the canonical selector/predicate pair without dropping malformed rows.
use std::collections::{BTreeMap, BTreeSet};

use super::invalid;
use crate::{
    Store,
    error::Result,
    namespace::DEFAULT_BASE_NS,
    sparql::{self, QueryResult},
    types::Value,
};

pub(super) type Catalogue = BTreeMap<String, std::result::Result<serde_json::Value, String>>;

pub(super) fn load(store: &Store) -> Result<Catalogue> {
    let query = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?p ?selector ?predicate ?tier ?language ?query ?pattern ?match_type ?gate WHERE {{ \
        ?p a a:Policy ; a:boundary \"action\" . \
        OPTIONAL {{ ?p a:selector ?selector . OPTIONAL {{ ?selector a:tier ?tier }} \
          OPTIONAL {{ ?selector a:language ?language }} OPTIONAL {{ ?selector a:evidenceSource ?query }} }} \
        OPTIONAL {{ ?p a:predicate ?predicate . OPTIONAL {{ ?predicate a:evidenceSource ?pattern }} \
          OPTIONAL {{ ?predicate a:matchType ?match_type }} OPTIONAL {{ ?predicate a:gate ?gate }} }} }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &query)? else {
        return Err(invalid("selector query returned no SELECT result"));
    };
    let mut grouped: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for row in rows {
        let Some(Value::Ref(id)) = row.get("p") else {
            return Err(invalid("policy IRI missing"));
        };
        let iri = store.resolve(*id)?;
        let fields = grouped.entry(iri).or_default();
        for (key, value) in row {
            if key == "p" {
                continue;
            }
            let text = match value {
                Value::Str(s) => s,
                Value::Ref(id) if key == "selector" || key == "predicate" => store.resolve(id)?,
                _ => "<invalid field type>".into(),
            };
            fields.entry(key).or_default().insert(text);
        }
    }
    Ok(grouped
        .into_iter()
        .map(|(iri, fields)| {
            let decoded = decode(&iri, &fields);
            (iri, decoded)
        })
        .collect())
}

fn decode(
    iri: &str,
    fields: &BTreeMap<String, BTreeSet<String>>,
) -> std::result::Result<serde_json::Value, String> {
    let get = |key: &str| -> std::result::Result<String, String> {
        let values = fields
            .get(key)
            .ok_or_else(|| format!("missing selector/predicate field {key}"))?;
        if values.len() != 1 {
            return Err(format!("ambiguous selector/predicate field {key}"));
        }
        let value = values.first().expect("one field");
        if value == "<invalid field type>" {
            return Err(format!("invalid field type for {key}"));
        }
        Ok(value.clone())
    };
    get("selector")?;
    get("predicate")?;
    if get("tier")? != "tree-sitter" {
        return Err("only tree-sitter selectors can be replayed by Yupana".into());
    }
    let mut rule = serde_json::json!({
        "name": iri, "language": get("language")?, "query": get("query")?,
        "pattern": get("pattern")?, "match_type": get("match_type")?
    });
    if fields.contains_key("gate") {
        rule["gate"] = get("gate")?.into();
    }
    Ok(rule)
}

#[cfg(all(test, feature = "shacl"))]
mod tests {
    use super::*;
    #[test]
    fn catalogue_loads_canonical_rules_and_refuses_ambiguous_fields() {
        let mut store = Store::open_in_memory().unwrap();
        crate::ingest_rdf(
            &mut store,
            &include_bytes!("../../../shapes/policies/treesitter.ttl")[..],
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01T00:00:00Z",
            None,
            Some("selector-test"),
        )
        .unwrap();
        let iri = format!("{DEFAULT_BASE_NS}policy_todo_needs_ticket");
        let catalogue = load(&store).unwrap();
        assert_eq!(
            catalogue[&iri].as_ref().unwrap()["query"],
            "(line_comment) @c"
        );
        let extra = format!(
            "<{DEFAULT_BASE_NS}sel_rust_line_comments> <{DEFAULT_BASE_NS}language> \"python\" ."
        );
        crate::ingest_rdf(
            &mut store,
            extra.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-02T00:00:00Z",
            None,
            Some("conflict-test"),
        )
        .unwrap();
        assert!(
            load(&store).unwrap()[&iri]
                .as_ref()
                .unwrap_err()
                .contains("ambiguous")
        );
    }
}

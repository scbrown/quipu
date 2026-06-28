//! Named-query catalog and the `quipu_ask` MCP tool (hq-h75).
//!
//! Agents previously had to hand-write SPARQL with full IRIs to answer common
//! questions ("what depends on X?", "what references X?"). This module exposes a
//! curated catalog of *parameterized* queries callable by name, so agents can
//! ask high-level questions without composing SPARQL themselves.
//!
//! The catalog is self-describing: calling [`tool_ask`] with no `name` (or
//! `name: "list"`) returns the available queries, their parameters, and types —
//! enough for an agent to discover and invoke any query.
//!
//! ## Safety
//!
//! Parameters are never interpolated raw. Each parameter has a [`ParamKind`]
//! that validates and escapes the value before substitution:
//! - [`ParamKind::Iri`] rejects characters that could break out of an
//!   angle-bracket IRI (`<>"{}|\^`, backtick, whitespace, control chars).
//! - [`ParamKind::Text`] escapes backslashes and single quotes for a
//!   single-quoted SPARQL string literal.
//! - [`ParamKind::Int`] parses an `i64`, rejecting anything non-numeric.

use std::collections::BTreeMap;

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql;
use crate::store::Store;

use super::value_to_json;

/// The kind of a named-query parameter — drives validation and escaping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// An entity IRI, substituted inside `<...>`.
    Iri,
    /// A free-text string, substituted inside `'...'`.
    Text,
    /// A signed integer (e.g. a `LIMIT` or a window size).
    Int,
}

impl ParamKind {
    fn label(self) -> &'static str {
        match self {
            ParamKind::Iri => "iri",
            ParamKind::Text => "text",
            ParamKind::Int => "int",
        }
    }

    /// Validate and escape `raw` for safe substitution into a query template.
    fn render(self, name: &str, raw: &str) -> Result<String> {
        match self {
            ParamKind::Iri => {
                if raw.is_empty() {
                    return Err(Error::InvalidValue(format!("param '{name}': empty IRI")));
                }
                if raw.chars().any(|c| {
                    c.is_whitespace()
                        || c.is_control()
                        || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '\\' | '^' | '`')
                }) {
                    return Err(Error::InvalidValue(format!(
                        "param '{name}': invalid character in IRI '{raw}'"
                    )));
                }
                Ok(raw.to_string())
            }
            ParamKind::Text => Ok(raw.replace('\\', "\\\\").replace('\'', "\\'")),
            ParamKind::Int => raw
                .trim()
                .parse::<i64>()
                .map(|n| n.to_string())
                .map_err(|_| {
                    Error::InvalidValue(format!("param '{name}': '{raw}' is not an integer"))
                }),
        }
    }
}

/// A single parameter of a named query.
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// Placeholder name; appears in the template as `{name}`.
    pub name: &'static str,
    /// Validation/escaping kind.
    pub kind: ParamKind,
    /// Whether the caller must supply this parameter.
    pub required: bool,
    /// Default value used when the caller omits an optional parameter.
    pub default: Option<&'static str>,
    /// Human-readable description for the self-describing catalog.
    pub description: &'static str,
}

/// A parameterized query callable by name.
#[derive(Debug, Clone, Copy)]
pub struct NamedQuery {
    /// Stable identifier the caller passes as `name`.
    pub name: &'static str,
    /// What the query answers.
    pub description: &'static str,
    /// SPARQL template with `{param}` placeholders.
    pub template: &'static str,
    /// Parameter specs, in display order.
    pub params: &'static [ParamSpec],
}

impl NamedQuery {
    /// Build executable SPARQL by validating and substituting `args`.
    fn render(&self, args: &BTreeMap<String, String>) -> Result<String> {
        let mut sparql = self.template.to_string();
        for spec in self.params {
            let raw = match args.get(spec.name) {
                Some(v) => v.clone(),
                None => match spec.default {
                    Some(d) => d.to_string(),
                    None if spec.required => {
                        return Err(Error::InvalidValue(format!(
                            "named query '{}' requires param '{}'",
                            self.name, spec.name
                        )));
                    }
                    None => continue,
                },
            };
            let rendered = spec.kind.render(spec.name, &raw)?;
            sparql = sparql.replace(&format!("{{{}}}", spec.name), &rendered);
        }
        Ok(sparql)
    }

    fn to_catalog_json(self) -> JsonValue {
        let params: Vec<JsonValue> = self
            .params
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "type": p.kind.label(),
                    "required": p.required,
                    "default": p.default,
                    "description": p.description,
                })
            })
            .collect();
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "params": params,
        })
    }
}

/// The named-query catalog. Schema-agnostic queries that work on any Quipu
/// store; extend by adding entries here.
pub const CATALOG: &[NamedQuery] = &[
    NamedQuery {
        name: "entity_facts",
        description: "All facts (predicate + object) asserted about an entity.",
        template: "SELECT ?p ?o WHERE { <{entity}> ?p ?o } LIMIT {limit}",
        params: &[
            ParamSpec {
                name: "entity",
                kind: ParamKind::Iri,
                required: true,
                default: None,
                description: "IRI of the entity to describe.",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int,
                required: false,
                default: Some("100"),
                description: "Maximum facts to return.",
            },
        ],
    },
    NamedQuery {
        name: "service_deps",
        description: "Outgoing entity references (dependencies / links) of an entity, e.g. what a service runs on or connects to.",
        template: "SELECT DISTINCT ?p ?o WHERE { <{entity}> ?p ?o . FILTER(isIRI(?o)) } LIMIT {limit}",
        params: &[
            ParamSpec {
                name: "entity",
                kind: ParamKind::Iri,
                required: true,
                default: None,
                description: "IRI whose dependencies to list.",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int,
                required: false,
                default: Some("50"),
                description: "Maximum dependencies to return.",
            },
        ],
    },
    NamedQuery {
        name: "references_to",
        description: "Entities that reference the given entity (incoming links) — the reverse of service_deps.",
        template: "SELECT DISTINCT ?s ?p WHERE { ?s ?p <{entity}> } LIMIT {limit}",
        params: &[
            ParamSpec {
                name: "entity",
                kind: ParamKind::Iri,
                required: true,
                default: None,
                description: "IRI to find references to.",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int,
                required: false,
                default: Some("50"),
                description: "Maximum referencing entities to return.",
            },
        ],
    },
    NamedQuery {
        name: "entities_of_type",
        description: "All entities declared with the given rdf:type.",
        template: "SELECT DISTINCT ?s WHERE { ?s a <{type}> } LIMIT {limit}",
        params: &[
            ParamSpec {
                name: "type",
                kind: ParamKind::Iri,
                required: true,
                default: None,
                description: "Class IRI to enumerate instances of.",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int,
                required: false,
                default: Some("100"),
                description: "Maximum instances to return.",
            },
        ],
    },
    NamedQuery {
        name: "labeled_like",
        description: "Entities whose rdfs:label contains the given text (case-insensitive).",
        template: "SELECT DISTINCT ?s ?label WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label . FILTER(CONTAINS(LCASE(STR(?label)), LCASE('{text}'))) } LIMIT {limit}",
        params: &[
            ParamSpec {
                name: "text",
                kind: ParamKind::Text,
                required: true,
                default: None,
                description: "Substring to match within entity labels.",
            },
            ParamSpec {
                name: "limit",
                kind: ParamKind::Int,
                required: false,
                default: Some("50"),
                description: "Maximum matches to return.",
            },
        ],
    },
];

/// Render the full catalog as JSON for discovery.
fn catalog_json() -> JsonValue {
    serde_json::json!({
        "queries": CATALOG.iter().map(|q| q.to_catalog_json()).collect::<Vec<_>>(),
        "usage": "Call with {\"name\": \"<query>\", \"params\": {...}}. Call with no name to list.",
    })
}

/// MCP tool: `quipu_ask` — run a named, parameterized query by name.
///
/// Input: `{ "name": "<query>", "params": { ... } }`. With no `name` (or
/// `name: "list"`) returns the self-describing catalog.
///
/// Output: `{ "query", "sparql", "columns", "rows": [...], "count" }`.
pub fn tool_ask(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let name = input.get("name").and_then(|v| v.as_str());

    let Some(name) = name else {
        return Ok(catalog_json());
    };
    if name == "list" {
        return Ok(catalog_json());
    }

    let query = CATALOG.iter().find(|q| q.name == name).ok_or_else(|| {
        Error::InvalidValue(format!(
            "unknown named query '{name}'; call quipu_ask with no name to list available queries"
        ))
    })?;

    // Collect caller params into a string map for rendering.
    let mut args: BTreeMap<String, String> = BTreeMap::new();
    if let Some(obj) = input.get("params").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(s) = json_to_param_str(v) {
                args.insert(k.clone(), s);
            } else {
                return Err(Error::InvalidValue(format!(
                    "param '{k}': unsupported value type"
                )));
            }
        }
    }

    let sparql = query.render(&args)?;
    let result = sparql::query(store, &sparql)?;

    let columns: Vec<String> = result.variables().to_vec();
    let rows: Vec<JsonValue> = result
        .rows()
        .iter()
        .map(|binding| {
            let obj: serde_json::Map<String, JsonValue> = columns
                .iter()
                .filter_map(|var| {
                    binding
                        .get(var)
                        .map(|val| (var.clone(), value_to_json(store, val)))
                })
                .collect();
            JsonValue::Object(obj)
        })
        .collect();

    Ok(serde_json::json!({
        "query": query.name,
        "sparql": sparql,
        "columns": columns,
        "rows": rows,
        "count": rows.len(),
    }))
}

/// Coerce a JSON parameter value to its string form for rendering.
fn json_to_param_str(v: &JsonValue) -> Option<String> {
    match v {
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Number(n) => Some(n.to_string()),
        JsonValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn query(name: &str) -> &'static NamedQuery {
        CATALOG.iter().find(|q| q.name == name).unwrap()
    }

    #[test]
    fn renders_required_and_default_params() {
        let sparql = query("entity_facts")
            .render(&args(&[("entity", "http://example.org/traefik")]))
            .unwrap();
        assert!(sparql.contains("<http://example.org/traefik>"));
        assert!(sparql.contains("LIMIT 100")); // default applied
    }

    #[test]
    fn caller_overrides_default() {
        let sparql = query("service_deps")
            .render(&args(&[("entity", "http://example.org/x"), ("limit", "5")]))
            .unwrap();
        assert!(sparql.contains("LIMIT 5"));
    }

    #[test]
    fn missing_required_param_errors() {
        let err = query("entity_facts").render(&args(&[])).unwrap_err();
        assert!(err.to_string().contains("requires param 'entity'"));
    }

    #[test]
    fn iri_injection_is_rejected() {
        // A closing angle bracket would break out of the <...> IRI.
        let err = query("entity_facts")
            .render(&args(&[("entity", "x> ?p ?o } DROP ALL #")]))
            .unwrap_err();
        assert!(err.to_string().contains("invalid character"));
    }

    #[test]
    fn non_integer_limit_is_rejected() {
        let err = query("entity_facts")
            .render(&args(&[
                ("entity", "http://example.org/x"),
                ("limit", "; DROP"),
            ]))
            .unwrap_err();
        assert!(err.to_string().contains("not an integer"));
    }

    #[test]
    fn text_param_escapes_quotes() {
        let sparql = query("labeled_like")
            .render(&args(&[("text", "o'brien")]))
            .unwrap();
        assert!(sparql.contains("o\\'brien"));
    }

    #[test]
    fn list_mode_returns_catalog() {
        let store = Store::open_in_memory().unwrap();
        let out = tool_ask(&store, &serde_json::json!({})).unwrap();
        let names: Vec<&str> = out["queries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"service_deps"));
        assert_eq!(names.len(), CATALOG.len());
    }

    #[test]
    fn unknown_query_errors() {
        let store = Store::open_in_memory().unwrap();
        let err = tool_ask(&store, &serde_json::json!({ "name": "nope" })).unwrap_err();
        assert!(err.to_string().contains("unknown named query"));
    }
}

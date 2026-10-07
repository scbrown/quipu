//! Stored queries carried by a share as `queries.ttl` (aegis-fxpbys.2).
//!
//! A qpack ships its competency questions beside its data and shapes: every
//! selected stored query (the registry `quipu_ask` reads) is described as RDF in
//! a `queries.ttl` member whose SHA-256 the manifest seals, so the queries are
//! part of the share's identity exactly as its shapes are.
//!
//! ## An imported query is DATA, never code
//!
//! Only SELECT, CONSTRUCT, ASK and DESCRIBE travel. The form is decided by
//! PARSING the template with the crate's SPARQL parser, never by pattern
//! matching on its text, and it is decided twice: when the producer writes the
//! member and again when a receiver reads it. A SPARQL Update is refused at
//! both ends. Nothing is evaluated on import; a query runs only when somebody
//! asks for it, under the receiver's normal read policy and timeouts.
//!
//! ## Which queries a share carries by default
//!
//! A stored query is *registered against* the shared graph when its dataset
//! scope would make `quipu_ask` answer it from that graph:
//!
//! - an unscoped query (`dataset = None`) answers from ROOT, so it belongs to a
//!   ROOT share and to the group and CONSTRUCT slices of ROOT;
//! - a dataset-scoped query belongs to a `--graph <iri>` share when `<iri>` is a
//!   member of its dataset, and to nothing else.
//!
//! `quipu share --queries <name>` replaces that default with an explicit list,
//! and `--no-queries` carries none. A share with no queries has no
//! `queries.ttl` and a manifest byte-identical to one produced before the
//! member existed.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::{NamedOrBlankNode, Term};

use crate::error::{Error, Result};
use crate::share::ShareScope;
use crate::store::Store;
use crate::store::queries::{StoredParam, StoredQuery};

/// The share member name.
pub const QUERIES_FILE: &str = "queries.ttl";

/// The SHACL contract for `queries.ttl`, compiled in so the importer applies
/// the shapes that ship with this code rather than any the share brings.
pub const QUERIES_SHAPES: &str = include_str!("../shapes/stored-queries.ttl");

const Q: &str = "https://quipu.dev/ontology/";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const DCT_DESCRIPTION: &str = "http://purl.org/dc/terms/description";
const SUBJECT_PREFIX: &str = "urn:quipu:query:";

/// One query as a share carries it: the definition plus what parsing derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedQuery {
    /// The definition, exactly as the producer's registry held it (minus the
    /// producer-local dataset scope — see [`select`]).
    pub query: StoredQuery,
    /// `SELECT` | `CONSTRUCT` | `ASK` | `DESCRIBE`, from the parsed template.
    pub form: &'static str,
    /// Constant `rdf:type` objects in the template's patterns, sorted.
    pub targets: Vec<String>,
}

/// Decide a template's read-only form by PARSING it, or refuse it.
///
/// # Errors
/// The template is a SPARQL Update (named as such, because that is the refusal
/// a reader needs to see), or does not parse as SPARQL at all.
pub fn read_only_form(query: &StoredQuery) -> Result<(&'static str, spargebra::Query)> {
    let probe = query.probe_sparql();
    match crate::sparql_structure::parse_query(crate::sparql::sparql_parser(), &probe)? {
        Ok(parsed) => {
            let form = match &parsed {
                spargebra::Query::Select { .. } => "SELECT",
                spargebra::Query::Construct { .. } => "CONSTRUCT",
                spargebra::Query::Ask { .. } => "ASK",
                spargebra::Query::Describe { .. } => "DESCRIBE",
            };
            Ok((form, parsed))
        }
        Err(query_error) => {
            if spargebra::SparqlParser::new()
                .with_base_iri("http://example.org/")
                .is_ok_and(|p| {
                    matches!(crate::sparql_structure::parse_update(p, &probe), Ok(Ok(_)))
                })
            {
                Err(Error::InvalidValue(format!(
                    "stored query '{}' is a SPARQL Update; a share carries read-only \
                     queries only (SELECT, CONSTRUCT, ASK, DESCRIBE)",
                    query.name
                )))
            } else {
                Err(Error::InvalidValue(format!(
                    "stored query '{}' does not parse as SPARQL: {query_error}",
                    query.name
                )))
            }
        }
    }
}

/// Parse, classify and inspect one definition for carriage.
///
/// # Errors
/// See [`read_only_form`] and [`StoredQuery::validate`].
pub fn inspect(query: StoredQuery) -> Result<SharedQuery> {
    let (form, parsed) = read_only_form(&query)?;
    query.validate()?;
    let targets: BTreeSet<String> = crate::sparql::rdfs::type_constants(&parsed)
        .into_iter()
        .collect();
    Ok(SharedQuery {
        query,
        form,
        targets: targets.into_iter().collect(),
    })
}

fn registered_against(store: &Store, query: &StoredQuery, scope: &ShareScope) -> Result<bool> {
    Ok(match (&query.dataset, scope) {
        (None, _) => true,
        (Some(dataset), ShareScope::Graph(iri)) => store
            .dataset_members(dataset)?
            .iter()
            .any(|member| &member.graph_iri == iri),
        (Some(_), _) => false,
    })
}

/// The queries a share carries: the named ones, or those registered against
/// its scope (module docs).
///
/// The dataset scope is dropped from each carried definition. A dataset IRI is
/// producer-local layout, and the receiver re-homes the data (staging, then
/// ROOT on promotion), so a carried scope would point at nothing there.
///
/// # Errors
/// A named query does not exist, or any selected query is not read-only.
pub fn select(
    store: &Store,
    scope: &ShareScope,
    names: Option<&[String]>,
) -> Result<Vec<SharedQuery>> {
    let chosen = match names {
        Some(names) => names
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|name| {
                store.query_get(name)?.ok_or_else(|| {
                    Error::InvalidValue(format!("share: no such stored query: {name}"))
                })
            })
            .collect::<Result<Vec<_>>>()?,
        None => {
            let mut out = Vec::new();
            for query in store.query_list()? {
                if registered_against(store, &query, scope)? {
                    out.push(query);
                }
            }
            out
        }
    };
    chosen
        .into_iter()
        .map(|mut query| {
            query.dataset = None;
            inspect(query)
        })
        .collect()
}

fn subject(name: &str) -> String {
    let mut out = String::from(SUBJECT_PREFIX);
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn short(value: &str) -> String {
    oxrdf::Literal::new_simple_literal(value).to_string()
}

fn long(value: &str) -> String {
    // Long-string form keeps a multi-line template readable in review. Only
    // `\` and `"` need escaping inside it; both are valid Turtle ECHARs.
    format!(
        "\"\"\"{}\"\"\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Serialize the carried queries deterministically (sorted by name).
#[must_use]
pub fn to_turtle(queries: &[SharedQuery]) -> String {
    let mut sorted: Vec<&SharedQuery> = queries.iter().collect();
    sorted.sort_by(|a, b| a.query.name.cmp(&b.query.name));
    let mut out = String::from(
        "@prefix dct: <http://purl.org/dc/terms/> .\n\
         @prefix quipu: <https://quipu.dev/ontology/> .\n",
    );
    for shared in sorted {
        let q = &shared.query;
        out.push_str(&format!(
            "\n<{}> a quipu:StoredQuery ;\n  quipu:queryName {} ;\n  quipu:queryForm {} ;\n",
            subject(&q.name),
            short(&q.name),
            short(shared.form)
        ));
        if !q.description.is_empty() {
            out.push_str(&format!("  dct:description {} ;\n", short(&q.description)));
        }
        for target in &shared.targets {
            out.push_str(&format!("  quipu:targetsClass <{target}> ;\n"));
        }
        for (index, p) in q.params.iter().enumerate() {
            out.push_str(&format!(
                "  quipu:parameter [ a quipu:QueryParameter ; quipu:parameterName {} ; \
                 quipu:parameterIndex {index} ; quipu:parameterKind {} ; quipu:required {}",
                short(&p.name),
                short(&p.kind),
                p.required
            ));
            if let Some(default) = &p.default {
                out.push_str(&format!(" ; quipu:defaultValue {}", short(default)));
            }
            if !p.description.is_empty() {
                out.push_str(&format!(" ; dct:description {}", short(&p.description)));
            }
            out.push_str(" ] ;\n");
        }
        out.push_str(&format!("  quipu:sparqlTemplate {} .\n", long(&q.template)));
    }
    out
}

#[derive(Default)]
struct Node {
    props: BTreeMap<String, Vec<Term>>,
}

fn one<'a>(node: &'a Node, predicate: &str, what: &str) -> Result<Option<&'a Term>> {
    match node
        .props
        .get(&format!("{Q}{predicate}"))
        .map(Vec::as_slice)
    {
        None | Some([]) => Ok(None),
        Some([value]) => Ok(Some(value)),
        Some(_) => Err(Error::InvalidValue(format!(
            "queries.ttl: {what} has more than one {predicate}"
        ))),
    }
}

fn string(term: Option<&Term>, what: &str) -> Result<Option<String>> {
    match term {
        None => Ok(None),
        Some(Term::Literal(l)) => Ok(Some(l.value().to_string())),
        Some(other) => Err(Error::InvalidValue(format!(
            "queries.ttl: {what} is not a literal: {other}"
        ))),
    }
}

fn required(node: &Node, predicate: &str, what: &str) -> Result<String> {
    string(one(node, predicate, what)?, what)?
        .ok_or_else(|| Error::InvalidValue(format!("queries.ttl: {what} has no {predicate}")))
}

fn description(node: &Node, what: &str) -> Result<String> {
    match node.props.get(DCT_DESCRIPTION).map(Vec::as_slice) {
        None | Some([]) => Ok(String::new()),
        Some([Term::Literal(l)]) => Ok(l.value().to_string()),
        Some(_) => Err(Error::InvalidValue(format!(
            "queries.ttl: {what} has an invalid dct:description"
        ))),
    }
}

/// The closed predicate set, enforced in every build (the SHACL mirror below
/// runs only where the `shacl` feature is compiled).
fn closed(key: &str, node: &Node) -> Result<()> {
    const ALLOWED: [&str; 10] = [
        "queryName",
        "queryForm",
        "sparqlTemplate",
        "targetsClass",
        "parameter",
        "parameterName",
        "parameterIndex",
        "parameterKind",
        "required",
        "defaultValue",
    ];
    for predicate in node.props.keys() {
        let known = predicate == RDF_TYPE
            || predicate == DCT_DESCRIPTION
            || predicate
                .strip_prefix(Q)
                .is_some_and(|local| ALLOWED.contains(&local));
        if !known {
            return Err(Error::InvalidValue(format!(
                "queries.ttl: {key} uses {predicate}, which a stored-query description \
                 does not have"
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "shacl")]
fn conform(turtle: &str) -> Result<()> {
    let feedback = crate::shacl::validate_shapes(QUERIES_SHAPES, turtle)?;
    if feedback.conforms {
        return Ok(());
    }
    let first = feedback
        .results
        .first()
        .map(|r| {
            format!(
                "{} {} {}",
                r.focus_node,
                r.path.as_deref().unwrap_or(""),
                r.message.as_deref().unwrap_or(&r.component)
            )
        })
        .unwrap_or_default();
    Err(Error::InvalidValue(format!(
        "queries.ttl does not conform to the stored-query shapes ({} violation(s)): {first}",
        feedback.violations
    )))
}

#[cfg(not(feature = "shacl"))]
#[allow(clippy::unnecessary_wraps)]
fn conform(_turtle: &str) -> Result<()> {
    Ok(())
}

/// Read a `queries.ttl` member back into definitions, re-deriving everything a
/// producer could have misstated.
///
/// The member must describe queries and NOTHING ELSE: a triple whose subject is
/// neither a query nor one of its parameters is refused, so the member cannot
/// smuggle graph data around the import quarantine. The declared form and
/// targets must equal what parsing the template derives.
///
/// # Errors
/// Parse errors, shape violations, a SPARQL Update, duplicate names, stray
/// triples, or a declared form/target set the template does not support.
pub fn from_turtle(turtle: &str) -> Result<Vec<SharedQuery>> {
    conform(turtle)?;
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for triple in
        oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle).for_reader(turtle.as_bytes())
    {
        let triple = triple.map_err(|e| Error::InvalidValue(format!("queries.ttl parse: {e}")))?;
        let key = match &triple.subject {
            NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        nodes
            .entry(key)
            .or_default()
            .props
            .entry(triple.predicate.as_str().to_string())
            .or_default()
            .push(triple.object);
    }
    for (key, node) in &nodes {
        closed(key, node)?;
    }
    let query_class = Term::NamedNode(oxrdf::NamedNode::new_unchecked(format!("{Q}StoredQuery")));
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    let mut names = BTreeSet::new();
    for (key, node) in &nodes {
        if !node
            .props
            .get(RDF_TYPE)
            .is_some_and(|types| types.contains(&query_class))
        {
            continue;
        }
        used.insert(key.clone());
        let name = required(node, "queryName", key)?;
        if key != &subject(&name) {
            return Err(Error::InvalidValue(format!(
                "queries.ttl: {key} does not identify query '{name}'"
            )));
        }
        if !names.insert(name.clone()) {
            return Err(Error::InvalidValue(format!(
                "queries.ttl: query '{name}' is described twice"
            )));
        }
        let mut params = Vec::new();
        for param in node
            .props
            .get(&format!("{Q}parameter"))
            .into_iter()
            .flatten()
        {
            let Term::BlankNode(b) = param else {
                return Err(Error::InvalidValue(format!(
                    "queries.ttl: query '{name}' parameter must be a blank node"
                )));
            };
            let pkey = format!("_:{}", b.as_str());
            let pnode = nodes.get(&pkey).ok_or_else(|| {
                Error::InvalidValue(format!(
                    "queries.ttl: query '{name}' has an empty parameter"
                ))
            })?;
            used.insert(pkey);
            let what = format!("query '{name}' parameter");
            let index = required(pnode, "parameterIndex", &what)?
                .parse::<usize>()
                .map_err(|e| Error::InvalidValue(format!("queries.ttl: {what} index: {e}")))?;
            params.push((
                index,
                StoredParam {
                    name: required(pnode, "parameterName", &what)?,
                    kind: required(pnode, "parameterKind", &what)?,
                    required: required(pnode, "required", &what)? == "true",
                    default: string(one(pnode, "defaultValue", &what)?, &what)?,
                    description: description(pnode, &what)?,
                },
            ));
        }
        params.sort_by_key(|(index, _)| *index);
        if params.iter().enumerate().any(|(i, (index, _))| i != *index) {
            return Err(Error::InvalidValue(format!(
                "queries.ttl: query '{name}' parameter indexes are not 0..n"
            )));
        }
        let declared_form = required(node, "queryForm", &name)?;
        let mut declared_targets: Vec<String> = node
            .props
            .get(&format!("{Q}targetsClass"))
            .into_iter()
            .flatten()
            .map(|t| match t {
                Term::NamedNode(n) => n.as_str().to_string(),
                other => other.to_string(),
            })
            .collect();
        declared_targets.sort();
        let shared = inspect(StoredQuery {
            template: required(node, "sparqlTemplate", &name)?,
            description: description(node, &name)?,
            dataset: None,
            params: params.into_iter().map(|(_, p)| p).collect(),
            name: name.clone(),
        })?;
        if shared.form != declared_form || shared.targets != declared_targets {
            return Err(Error::InvalidValue(format!(
                "queries.ttl: query '{name}' declares form {declared_form} targeting \
                 {declared_targets:?}, but its template is {} targeting {:?}",
                shared.form, shared.targets
            )));
        }
        out.push(shared);
    }
    if let Some(stray) = nodes.keys().find(|key| !used.contains(*key)) {
        return Err(Error::InvalidValue(format!(
            "queries.ttl carries a node that is not a stored query or one of its \
             parameters: {stray}"
        )));
    }
    out.sort_by(|a, b| a.query.name.cmp(&b.query.name));
    Ok(out)
}

mod import;
mod pending;
pub use import::{
    Pending, QueryCollision, QueryImport, QueryQuarantine, check_namespace, default_namespace,
    install, off_vocabulary, prepare, settle, verify_member,
};
pub use pending::release;

#[cfg(test)]
#[path = "share_queries_tests.rs"]
mod tests;

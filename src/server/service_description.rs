//! `VoID` and SPARQL 1.1 Service Description interoperability projection.

use axum::{
    extract::State,
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value as JsonValue, json};

use super::{SharedStore, base::AppError, base::blocking};

const SD: &str = "http://www.w3.org/ns/sparql-service-description#";
const VOID: &str = "http://rdfs.org/ns/void#";

pub(crate) async fn service_description(
    State(store): State<SharedStore>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let base = request_base(&headers);
    let description = format!("{base}/.well-known/void");
    let endpoint = format!("{base}/query");
    let dataset = format!("{description}#dataset");
    let service = format!("{description}#service");
    let (graphs, vocabularies, counts) = blocking(move || {
        let store = store.read();
        Ok((
            store.graph_fact_counts()?,
            store.predicate_vocabularies()?,
            store.graph_counts()?,
        ))
    })
    .await?;

    let wants_json_ld = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("application/ld+json"));
    if wants_json_ld {
        let body = json_ld(
            &dataset,
            &service,
            &endpoint,
            &graphs,
            &vocabularies,
            counts,
        );
        Ok((
            [
                (header::CONTENT_TYPE, "application/ld+json"),
                (header::VARY, "Accept"),
            ],
            axum::Json(body),
        )
            .into_response())
    } else {
        let body = turtle(
            &dataset,
            &service,
            &endpoint,
            &graphs,
            &vocabularies,
            counts,
        );
        Ok((
            [
                (header::CONTENT_TYPE, "text/turtle; charset=utf-8"),
                (header::VARY, "Accept"),
            ],
            body,
        )
            .into_response())
    }
}

fn request_base(headers: &HeaderMap) -> String {
    let candidate_scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .unwrap_or("http")
        .trim();
    let scheme = match candidate_scheme {
        "http" | "https" => candidate_scheme,
        _ => "http",
    };
    let candidate_host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .unwrap_or("localhost")
        .trim();
    let host = if !candidate_host.is_empty()
        && candidate_host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
    {
        candidate_host
    } else {
        "localhost"
    };
    format!("{scheme}://{host}")
}

fn result_formats() -> impl Iterator<Item = &'static str> {
    quipu::w3c::SUPPORTED_RESULT_FORMATS
        .iter()
        .map(|(_, iri)| *iri)
}

fn service_features() -> Vec<&'static str> {
    vec!["http://www.w3.org/ns/sparql-service-description#EmptyGraphs"]
}

fn entailment_regimes() -> Vec<&'static str> {
    let mut out = Vec::new();
    if cfg!(feature = "owl") {
        out.push("http://www.w3.org/ns/entailment/OWL-Direct");
    }
    if cfg!(feature = "reactive-reasoner") {
        out.push("https://quipu.dev/features/reactive-datalog");
    }
    out
}

fn json_ld(
    dataset: &str,
    service: &str,
    endpoint: &str,
    graphs: &[(String, u64)],
    vocabularies: &[String],
    counts: (u64, u64, u64),
) -> JsonValue {
    let named: Vec<JsonValue> = graphs
        .iter()
        .filter(|(iri, _)| iri != quipu::schema::ROOT_GRAPH_IRI)
        .map(|(iri, triples)| {
            json!({
                "@type": "sd:NamedGraph", "sd:name": {"@id": iri},
                "sd:graph": {"@type": "sd:Graph", "void:triples": triples}
            })
        })
        .collect();
    json!({
        "@context": {
            "sd": SD, "void": VOID, "dcterms": "http://purl.org/dc/terms/",
            "quipu": "https://quipu.dev/ontology/",
            "xsd": "http://www.w3.org/2001/XMLSchema#"
        },
        "@graph": [
            {"@id": dataset, "@type": ["void:Dataset", "sd:Dataset"],
             "dcterms:title": "Quipu knowledge graph",
             "void:sparqlEndpoint": {"@id": endpoint},
             "void:triples": counts.1, "void:entities": counts.0,
             "void:properties": counts.2,
             "void:vocabulary": vocabularies.iter().map(|v| json!({"@id": v})).collect::<Vec<_>>(),
             "quipu:exportEndpoint": {"@id": endpoint.replace("/query", "/export")},
             "quipu:shareEndpoint": {"@id": endpoint.replace("/query", "/share")},
             "sd:namedGraph": named},
            {"@id": service, "@type": "sd:Service", "sd:endpoint": {"@id": endpoint},
             "sd:supportedLanguage": {"@id": format!("{SD}SPARQL11Query")},
             "sd:resultFormat": result_formats().map(|v| json!({"@id": v})).collect::<Vec<_>>(),
             "sd:feature": service_features().iter().map(|v| json!({"@id": v})).collect::<Vec<_>>(),
             "sd:defaultEntailmentRegime": entailment_regimes().iter().map(|v| json!({"@id": v})).collect::<Vec<_>>(),
             "sd:defaultDataset": {"@id": dataset}}
        ]
    })
}

fn turtle(
    dataset: &str,
    service: &str,
    endpoint: &str,
    graphs: &[(String, u64)],
    vocabularies: &[String],
    counts: (u64, u64, u64),
) -> String {
    let mut out = format!(
        "@prefix sd: <{SD}> .\n@prefix void: <{VOID}> .\n@prefix dcterms: <http://purl.org/dc/terms/> .\n@prefix quipu: <https://quipu.dev/ontology/> .\n\n\
         <{dataset}> a void:Dataset, sd:Dataset ;\n  dcterms:title \"Quipu knowledge graph\" ;\n  void:sparqlEndpoint <{endpoint}> ;\n  void:triples {} ;\n  void:entities {} ;\n  void:properties {}",
        counts.1, counts.0, counts.2
    );
    for vocabulary in vocabularies {
        out.push_str(&format!(" ;\n  void:vocabulary <{vocabulary}>"));
    }
    out.push_str(&format!(
        " ;\n  quipu:exportEndpoint <{}> ;\n  quipu:shareEndpoint <{}>",
        endpoint.replace("/query", "/export"),
        endpoint.replace("/query", "/share")
    ));
    for (iri, triples) in graphs
        .iter()
        .filter(|(iri, _)| iri != quipu::schema::ROOT_GRAPH_IRI)
    {
        out.push_str(&format!(" ;\n  sd:namedGraph [ a sd:NamedGraph ; sd:name <{iri}> ; sd:graph [ a sd:Graph ; void:triples {triples} ] ]"));
    }
    out.push_str(" .\n\n");
    out.push_str(&format!("<{service}> a sd:Service ;\n  sd:endpoint <{endpoint}> ;\n  sd:supportedLanguage sd:SPARQL11Query"));
    for format_iri in result_formats() {
        out.push_str(&format!(" ;\n  sd:resultFormat <{format_iri}>"));
    }
    for feature in service_features() {
        out.push_str(&format!(" ;\n  sd:feature <{feature}>"));
    }
    for regime in entailment_regimes() {
        out.push_str(&format!(" ;\n  sd:defaultEntailmentRegime <{regime}>"));
    }
    out.push_str(&format!(" ;\n  sd:defaultDataset <{dataset}> .\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_result_formats_are_the_executable_registry() {
        for (media_type, _) in quipu::w3c::SUPPORTED_RESULT_FORMATS {
            assert!(
                quipu::w3c::negotiate(media_type).is_some(),
                "advertised format {media_type} is not executable"
            );
        }
    }

    #[test]
    fn forwarded_origin_builds_dereferenceable_identifiers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        headers.insert("x-forwarded-host", "example.test".parse().unwrap());
        assert_eq!(request_base(&headers), "https://example.test");
    }

    #[test]
    fn renderings_carry_required_void_and_service_description_terms() {
        let graphs = vec![
            (quipu::schema::ROOT_GRAPH_IRI.into(), 7),
            ("urn:g:one".into(), 3),
        ];
        let vocabularies = vec!["http://example.org/vocab/".into()];
        let ttl = turtle(
            "https://example.test/.well-known/void#dataset",
            "https://example.test/.well-known/void#service",
            "https://example.test/query",
            &graphs,
            &vocabularies,
            (4, 7, 2),
        );
        for required in [
            "void:Dataset",
            "void:sparqlEndpoint",
            "void:vocabulary",
            "void:triples",
            "sd:Service",
            "sd:supportedLanguage",
            "sd:resultFormat",
            "sd:namedGraph",
        ] {
            assert!(ttl.contains(required), "Turtle omitted {required}");
        }
        let parsed = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle)
            .for_reader(ttl.as_bytes())
            .collect::<Result<Vec<_>, _>>();
        assert!(parsed.is_ok(), "generated Turtle must parse: {parsed:?}");
        let json = json_ld(
            "https://example.test/.well-known/void#dataset",
            "https://example.test/.well-known/void#service",
            "https://example.test/query",
            &graphs,
            &vocabularies,
            (4, 7, 2),
        );
        assert_eq!(json["@graph"][0]["@type"][0], "void:Dataset");
        assert_eq!(json["@graph"][1]["@type"], "sd:Service");
    }

    #[test]
    fn route_registry_auth_registry_and_book_move_together() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let server = std::fs::read_to_string(root.join("src/server.rs")).unwrap();
        let auth = std::fs::read_to_string(root.join("src/http_auth.rs")).unwrap();
        let book =
            std::fs::read_to_string(root.join("docs/book/src/reference/rest-api.md")).unwrap();
        for source in [&server, &auth, &book] {
            assert!(source.contains("/.well-known/void"));
        }
    }
}

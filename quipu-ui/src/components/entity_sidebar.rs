//! Entity page component — renders at /entity/{iri}.
//!
//! Shows full entity details with statement groups and emits a JSON-LD
//! script block for semantic web crawlers.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::components::graph_explorer::{Fact, FactGroup};

/// Shorten IRI for display.
fn short_name(iri: &str) -> String {
    let iri = iri.trim_start_matches('<').trim_end_matches('>');
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

/// Group facts by predicate.
fn group_facts(facts: &[Fact]) -> Vec<FactGroup> {
    let mut groups: Vec<FactGroup> = Vec::new();
    for fact in facts {
        if let Some(group) = groups.iter_mut().find(|g| g.predicate == fact.predicate) {
            group.values.push(fact.clone());
        } else {
            groups.push(FactGroup {
                predicate: fact.predicate.clone(),
                values: vec![fact.clone()],
            });
        }
    }
    groups
}

/// Build JSON-LD object from entity facts.
fn build_jsonld(iri: &str, entity_type: &str, facts: &[Fact]) -> String {
    let mut props = serde_json::Map::new();
    props.insert(
        "@context".to_string(),
        serde_json::Value::String("https://schema.org".to_string()),
    );
    props.insert(
        "@id".to_string(),
        serde_json::Value::String(iri.to_string()),
    );
    props.insert(
        "@type".to_string(),
        serde_json::Value::String(entity_type.to_string()),
    );

    for fact in facts {
        let key = short_name(&fact.predicate);
        let val = if fact.is_iri {
            serde_json::json!({"@id": fact.value})
        } else {
            serde_json::Value::String(fact.value.clone())
        };

        // If key already exists, convert to array
        if let Some(existing) = props.get_mut(&key) {
            if let serde_json::Value::Array(arr) = existing {
                arr.push(val);
            } else {
                let prev = existing.clone();
                *existing = serde_json::Value::Array(vec![prev, val]);
            }
        } else {
            props.insert(key, val);
        }
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(props)).unwrap_or_default()
}

/// Inject a JSON-LD script block into the document head.
fn inject_jsonld(jsonld: &str) {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let head = document.head().unwrap();

    // Remove any existing quipu JSON-LD block
    let existing = document.query_selector_all("script[data-quipu-jsonld]");
    if let Ok(list) = existing {
        for i in 0..list.length() {
            if let Some(el) = list.item(i) {
                let _ = el.parent_node().map(|p| p.remove_child(&el));
            }
        }
    }

    let script = document.create_element("script").unwrap();
    script.set_attribute("type", "application/ld+json").unwrap();
    script.set_attribute("data-quipu-jsonld", "").unwrap();
    script.set_text_content(Some(jsonld));
    let _ = head.append_child(&script);
}

/// Entity page view — /entity/{iri}.
#[component]
pub fn EntityPage() -> impl IntoView {
    let params = use_params_map();
    let (facts, set_facts) = signal(Vec::<Fact>::new());
    let (entity_type, set_entity_type) = signal(String::new());
    let (loading, set_loading) = signal(true);

    let iri = Memo::new(move |_| {
        params.get().get("iri").unwrap_or_default()
    });

    // Load entity data when IRI changes
    Effect::new(move || {
        let iri_val = iri.get();
        if iri_val.is_empty() {
            return;
        }
        set_loading.set(true);
        let decoded = urldecode(&iri_val);
        spawn_local(async move {
            match api::fetch_entity_facts(&decoded).await {
                Ok(f) => {
                    // Extract type from rdf:type facts
                    let etype = f
                        .iter()
                        .find(|fact| {
                            fact.predicate.contains("type")
                                || fact.predicate.contains("rdf:type")
                                || fact.predicate.ends_with("#type")
                        })
                        .map(|fact| short_name(&fact.value))
                        .unwrap_or_else(|| "Thing".to_string());

                    let jsonld = build_jsonld(&decoded, &etype, &f);
                    inject_jsonld(&jsonld);

                    set_entity_type.set(etype);
                    set_facts.set(f);
                    set_loading.set(false);
                }
                Err(e) => {
                    log::error!("Failed to load entity: {e}");
                    set_loading.set(false);
                }
            }
        });
    });

    view! {
        <div class="entity-page">
            {move || {
                if loading.get() {
                    return view! { <div class="loading">"Loading entity..."</div> }.into_any();
                }

                let iri_val = iri.get();
                let decoded = urldecode(&iri_val);
                let label = short_name(&decoded);
                let etype = entity_type.get();
                let groups = group_facts(&facts.get());

                view! {
                    <div>
                        <h2>{label}</h2>
                        <div class="entity-type">{etype}</div>
                        <div class="detail-iri">{decoded}</div>

                        {groups
                            .iter()
                            .map(|g| {
                                let pred_name = short_name(&g.predicate);
                                let values = g.values.clone();
                                view! {
                                    <div class="fact-group">
                                        <div class="fact-group-header">{pred_name}</div>
                                        {values
                                            .iter()
                                            .map(|v| {
                                                if v.is_iri {
                                                    let href = format!(
                                                        "/entity/{}",
                                                        urlencoding(&v.value),
                                                    );
                                                    let display = short_name(&v.value);
                                                    view! {
                                                        <div class="fact-value">
                                                            <a href=href>{display}</a>
                                                        </div>
                                                    }
                                                    .into_any()
                                                } else {
                                                    view! {
                                                        <div class="fact-value">{v.value.clone()}</div>
                                                    }
                                                    .into_any()
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>
                }
                .into_any()
            }}
        </div>
    }
}

fn urlencoding(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F")
        .replace('&', "%26")
        .replace('=', "%3D")
}

fn urldecode(s: &str) -> String {
    s.replace("%25", "%")
        .replace("%20", " ")
        .replace("%23", "#")
        .replace("%3F", "?")
        .replace("%26", "&")
        .replace("%3D", "=")
}

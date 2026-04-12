//! Schema Browser — tree view of entity types, SHACL shape cards,
//! visual schema graph (WebVOWL-inspired), and validation report.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::interop;

#[derive(Clone, Copy, PartialEq)]
enum SchemaTab { Tree, Visual, Validation }

#[derive(Clone, Debug, Default)]
struct ShapeInfo { name: String, loaded_at: String }

#[derive(Clone, Debug)]
struct ValidationIssue {
    severity: String, focus_node: String, message: String, source_shape: String,
}

fn parse_issue(v: &serde_json::Value) -> ValidationIssue {
    let s = |k| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    ValidationIssue { severity: s("severity"), focus_node: s("focus_node"), message: s("message"), source_shape: s("source_shape") }
}

#[component]
pub fn SchemaBrowser() -> impl IntoView {
    let (active_tab, set_active_tab) = signal(SchemaTab::Tree);
    let (type_counts, set_type_counts) = signal(Vec::<api::TypeCount>::new());
    let (shapes, set_shapes) = signal(Vec::<ShapeInfo>::new());
    let (selected_type, set_selected_type) = signal(Option::<String>::None);
    let (type_facts, set_type_facts) = signal(Vec::<(String, String)>::new());
    let (val_results, set_val_results) = signal(serde_json::Value::Null);
    let (val_running, set_val_running) = signal(false);
    let (loading, set_loading) = signal(true);
    let (schema_graph_ready, set_schema_graph_ready) = signal(false);

    // Fetch type counts and shapes on mount
    Effect::new(move || {
        spawn_local(async move {
            set_type_counts.set(api::fetch_type_counts().await.unwrap_or_default());
            let data = api::fetch_shapes().await.unwrap_or(serde_json::Value::Null);
            let list: Vec<ShapeInfo> = data.get("shapes").and_then(|s| s.as_array())
                .map(|arr| arr.iter().map(|s| ShapeInfo {
                    name: s.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").into(),
                    loaded_at: s.get("loaded_at").and_then(|n| n.as_str()).unwrap_or("").into(),
                }).collect())
                .unwrap_or_default();
            set_shapes.set(list);
            set_loading.set(false);
        });
    });

    // Build visual schema graph when tab switches to Visual
    Effect::new(move || {
        if active_tab.get() == SchemaTab::Visual && !schema_graph_ready.get() {
            let counts = type_counts.get();
            spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(100).await;
                if interop::js_schema_init("schema-graph-container") {
                    for tc in &counts {
                        interop::js_schema_add_class(&tc.iri, &tc.label, tc.count as u32);
                    }
                    let q = "SELECT ?child ?parent WHERE { ?child <http://www.w3.org/2000/01/rdf-schema#subClassOf> ?parent }";
                    if let Ok(result) = api::sparql_query(q).await {
                        if let Some(rows) = result.get("rows").and_then(|r| r.as_array()) {
                            for row in rows {
                                let c = row.get("child").and_then(|v| v.as_str()).unwrap_or("");
                                let p = row.get("parent").and_then(|v| v.as_str()).unwrap_or("");
                                if !c.is_empty() && !p.is_empty() { interop::js_schema_add_subclass(c, p); }
                            }
                        }
                    }
                    interop::js_schema_layout();
                    set_schema_graph_ready.set(true);
                    let on_click = Closure::new(move |id: String| {
                        set_selected_type.set(Some(id));
                        set_active_tab.set(SchemaTab::Tree);
                    });
                    interop::js_schema_register_click(&on_click);
                    on_click.forget();
                }
            });
        }
    });

    // Load facts about selected type
    Effect::new(move || {
        if let Some(type_iri) = selected_type.get() {
            spawn_local(async move {
                let q = format!("SELECT DISTINCT ?p ?datatype WHERE {{\n  ?instance a <{type_iri}> .\n  ?instance ?p ?val .\n  BIND(DATATYPE(?val) AS ?datatype)\n}} ORDER BY ?p");
                match api::sparql_query(&q).await {
                    Ok(r) => {
                        let rows = r.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default();
                        let facts: Vec<(String, String)> = rows.iter().filter_map(|row| {
                            let p = row.get("p").and_then(|v| v.as_str())?.to_string();
                            let dt = row.get("datatype").and_then(|v| v.as_str()).unwrap_or("IRI ref").to_string();
                            Some((p, dt))
                        }).collect();
                        set_type_facts.set(facts);
                    }
                    Err(_) => set_type_facts.set(vec![]),
                }
            });
        }
    });

    let run_validation = move |_| {
        set_val_running.set(true);
        spawn_local(async move {
            match api::fetch_shapes().await {
                Ok(data) => {
                    let names: Vec<String> = data.get("shapes").and_then(|s| s.as_array())
                        .map(|a| a.iter().filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
                        .unwrap_or_default();
                    if names.is_empty() {
                        set_val_results.set(serde_json::json!({"message": "No SHACL shapes loaded. Load shapes via /shapes to enable validation."}));
                    } else {
                        match api::run_validation("").await {
                            Ok(r) => set_val_results.set(r),
                            Err(e) => set_val_results.set(serde_json::json!({"error": e})),
                        }
                    }
                }
                Err(e) => set_val_results.set(serde_json::json!({"error": e})),
            }
            set_val_running.set(false);
        });
    };

    view! {
        <div class="schema-view">
            <div class="schema-tabs">
                <button class="schema-tab" class:active=move || active_tab.get() == SchemaTab::Tree
                    on:click=move |_| set_active_tab.set(SchemaTab::Tree)>"Type Tree"</button>
                <button class="schema-tab" class:active=move || active_tab.get() == SchemaTab::Visual
                    on:click=move |_| set_active_tab.set(SchemaTab::Visual)>"Visual Schema"</button>
                <button class="schema-tab" class:active=move || active_tab.get() == SchemaTab::Validation
                    on:click=move |_| set_active_tab.set(SchemaTab::Validation)>"Validation"</button>
            </div>
            <div class="schema-content">
                {move || {
                    if loading.get() {
                        return view! { <div class="loading">"Loading schema..."</div> }.into_any();
                    }
                    match active_tab.get() {
                        SchemaTab::Tree => render_tree_tab(type_counts, shapes, selected_type, set_selected_type, type_facts).into_any(),
                        SchemaTab::Visual => view! { <div id="schema-graph-container" class="schema-graph-container"></div> }.into_any(),
                        SchemaTab::Validation => render_validation_tab(val_results, val_running, run_validation).into_any(),
                    }
                }}
            </div>
        </div>
    }
}

fn render_tree_tab(
    type_counts: ReadSignal<Vec<api::TypeCount>>,
    shapes: ReadSignal<Vec<ShapeInfo>>,
    selected_type: ReadSignal<Option<String>>,
    set_selected_type: WriteSignal<Option<String>>,
    type_facts: ReadSignal<Vec<(String, String)>>,
) -> impl IntoView {
    view! {
        <div class="schema-tree-layout">
            <div class="type-tree">
                <h3 class="tree-header">"Entity Types"</h3>
                {move || {
                    let counts = type_counts.get();
                    if counts.is_empty() {
                        view! { <div class="tree-empty">"No entity types found."</div> }.into_any()
                    } else {
                        view! { <div class="tree-list">
                            {counts.iter().map(|tc| {
                                let iri = tc.iri.clone(); let iri2 = tc.iri.clone();
                                let is_sel = move || selected_type.get().as_deref() == Some(&iri2);
                                view! { <div class="tree-item" class:selected=is_sel
                                    on:click=move |_| set_selected_type.set(Some(iri.clone()))>
                                    <span class="tree-icon">"+"</span>
                                    <span class="tree-label">{tc.label.clone()}</span>
                                    <span class="tree-count">{format!("({})", tc.count)}</span>
                                </div> }
                            }).collect::<Vec<_>>()}
                        </div> }.into_any()
                    }
                }}
                <h3 class="tree-header" style="margin-top: 1rem;">"SHACL Shapes"</h3>
                {move || {
                    let list = shapes.get();
                    if list.is_empty() {
                        view! { <div class="tree-empty">"No shapes loaded"</div> }.into_any()
                    } else {
                        view! { <div class="tree-list">
                            {list.iter().map(|s| view! {
                                <div class="tree-item shape-item">
                                    <span class="shape-icon">"S"</span>
                                    <span class="tree-label">{s.name.clone()}</span>
                                    <span class="tree-count">{s.loaded_at.clone()}</span>
                                </div>
                            }).collect::<Vec<_>>()}
                        </div> }.into_any()
                    }
                }}
            </div>
            <div class="shape-detail">
                {move || match selected_type.get() {
                    None => view! { <div class="shape-placeholder">"Select a type to view its properties"</div> }.into_any(),
                    Some(iri) => {
                        let label = api::short_name(&iri);
                        let facts = type_facts.get();
                        let count = type_counts.get().iter().find(|tc| tc.iri == iri).map(|tc| tc.count).unwrap_or(0);
                        let iri_display = iri.clone();
                        let query_url = format!("/sparql#q={}", js_sys::encode_uri_component(
                            &format!("SELECT ?inst ?p ?o WHERE {{\n  ?inst a <{iri}> .\n  ?inst ?p ?o\n}} LIMIT 50")
                        ).as_string().unwrap_or_default());
                        view! {
                            <div class="shape-card">
                                <div class="shape-card-header">
                                    <h3>{label}</h3>
                                    <span class="shape-card-count">{format!("{count} instances")}</span>
                                </div>
                                <div class="shape-card-iri">{iri_display}</div>
                                <div class="shape-card-section">
                                    <h4>"Properties"</h4>
                                    {if facts.is_empty() {
                                        view! { <div class="tree-empty">"Loading..."</div> }.into_any()
                                    } else {
                                        view! { <table class="property-table">
                                            <thead><tr><th>"Property"</th><th>"Type"</th></tr></thead>
                                            <tbody>
                                                {facts.iter().map(|(p, dt)| view! {
                                                    <tr><td class="prop-name">{api::short_name(p)}</td>
                                                        <td class="prop-type">{api::short_name(dt)}</td></tr>
                                                }).collect::<Vec<_>>()}
                                            </tbody>
                                        </table> }.into_any()
                                    }}
                                </div>
                                <div class="shape-card-actions">
                                    <a class="btn" href=query_url>"Query instances"</a>
                                </div>
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}

fn render_validation_tab(
    val_results: ReadSignal<serde_json::Value>,
    val_running: ReadSignal<bool>,
    run_validation: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <div class="validation-view">
            <div class="validation-toolbar">
                <button class="btn btn-primary" on:click=run_validation disabled=move || val_running.get()>
                    {move || if val_running.get() { "Validating..." } else { "Run Validation" }}
                </button>
            </div>
            <div class="validation-results">
                {move || {
                    let res = val_results.get();
                    if res.is_null() {
                        return view! { <div class="loading">"Click 'Run Validation' to check entities against SHACL shapes"</div> }.into_any();
                    }
                    if let Some(msg) = res.get("message").and_then(|m| m.as_str()) {
                        return view! { <div class="validation-info">{msg.to_string()}</div> }.into_any();
                    }
                    if let Some(err) = res.get("error").and_then(|e| e.as_str()) {
                        return view! { <div class="result-error">{format!("Error: {err}")}</div> }.into_any();
                    }
                    let conforms = res.get("conforms").and_then(|c| c.as_bool()).unwrap_or(false);
                    let violations = res.get("violations").and_then(|v| v.as_u64()).unwrap_or(0);
                    let warnings = res.get("warnings").and_then(|w| w.as_u64()).unwrap_or(0);
                    let issues: Vec<ValidationIssue> = res.get("issues").and_then(|i| i.as_array())
                        .map(|arr| arr.iter().map(parse_issue).collect()).unwrap_or_default();
                    view! {
                        <div class="validation-summary">
                            <span class=if conforms { "validation-badge ok" } else { "validation-badge fail" }>
                                {if conforms { "CONFORMS" } else { "ISSUES FOUND" }}
                            </span>
                            <span class="validation-stat">{format!("{violations} violations, {warnings} warnings")}</span>
                        </div>
                        <div class="validation-issues">
                            {issues.iter().map(|issue| {
                                let focus = issue.focus_node.clone();
                                let icon = if issue.severity == "violation" { "!!" } else { "!" };
                                let cls = format!("validation-issue issue-{}", issue.severity);
                                view! { <div class=cls>
                                    <span class="issue-icon">{icon}</span>
                                    <div class="issue-body">
                                        <div class="issue-focus">
                                            <a href=format!("/entity/{}", js_sys::encode_uri_component(&focus).as_string().unwrap_or_default())>
                                                {api::short_name(&issue.focus_node)}
                                            </a>
                                            <span class="issue-shape">{api::short_name(&issue.source_shape)}</span>
                                        </div>
                                        <div class="issue-message">{issue.message.clone()}</div>
                                    </div>
                                </div> }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

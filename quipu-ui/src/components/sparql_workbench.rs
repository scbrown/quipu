//! SPARQL Workbench — CodeMirror 6 editor with schema-aware autocomplete,
//! query templates, multiple result views, and query history.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::interop;

struct QueryTemplate { name: &'static str, persona: &'static str, sparql: &'static str }

const TEMPLATES: &[QueryTemplate] = &[
    QueryTemplate { name: "All entities", persona: "general",
        sparql: "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 100" },
    QueryTemplate { name: "Entity types", persona: "general",
        sparql: "SELECT ?type (COUNT(?s) AS ?count)\nWHERE { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type }\nGROUP BY ?type\nORDER BY DESC(?count)" },
    QueryTemplate { name: "Infrastructure hosts", persona: "operator",
        sparql: "SELECT ?host ?status ?ip\nWHERE {\n  ?host a <http://schema.org/Server> .\n  OPTIONAL { ?host <http://schema.org/status> ?status }\n  OPTIONAL { ?host <http://schema.org/ipAddress> ?ip }\n}" },
    QueryTemplate { name: "Service dependencies", persona: "operator",
        sparql: "SELECT ?service ?dep\nWHERE {\n  ?service <http://schema.org/dependsOn> ?dep\n}\nORDER BY ?service" },
    QueryTemplate { name: "Agent crew members", persona: "agent builder",
        sparql: "SELECT ?agent ?role ?rig\nWHERE {\n  ?agent a <http://schema.org/SoftwareAgent> .\n  OPTIONAL { ?agent <http://schema.org/roleName> ?role }\n  OPTIONAL { ?agent <http://schema.org/memberOf> ?rig }\n}" },
    QueryTemplate { name: "Recent knowledge", persona: "archaeologist",
        sparql: "SELECT ?s ?p ?o\nWHERE { ?s ?p ?o }\nORDER BY DESC(?s)\nLIMIT 50" },
    QueryTemplate { name: "Predicate frequency", persona: "gardener",
        sparql: "SELECT ?p (COUNT(*) AS ?usage)\nWHERE { ?s ?p ?o }\nGROUP BY ?p\nORDER BY DESC(?usage)" },
    QueryTemplate { name: "Orphan entities", persona: "gardener",
        sparql: "SELECT ?entity\nWHERE {\n  ?entity <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type .\n  FILTER NOT EXISTS { ?other ?p ?entity }\n}\nLIMIT 50" },
];

#[derive(Clone, Copy, PartialEq)]
enum ResultView { Table, Graph, Json }

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct HistoryEntry { query: String, timestamp: String, row_count: usize }

fn get_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn load_history() -> Option<Vec<HistoryEntry>> {
    let s = get_storage()?;
    let json = s.get_item("quipu_sparql_history").ok()??;
    serde_json::from_str::<Vec<HistoryEntry>>(&json).ok()
}

fn save_history(history: &[HistoryEntry]) {
    if let Some(s) = get_storage() {
        if let Ok(json) = serde_json::to_string(history) {
            let _ = s.set_item("quipu_sparql_history", &json);
        }
    }
}

#[component]
pub fn SparqlWorkbench() -> impl IntoView {
    let (results, set_results) = signal(serde_json::Value::Null);
    let (running, set_running) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (result_view, set_result_view) = signal(ResultView::Table);
    let (history, set_history) = signal(load_history().unwrap_or_default());
    let (show_history, set_show_history) = signal(false);
    let (show_templates, set_show_templates) = signal(false);
    let (row_count, set_row_count) = signal(0usize);
    let (editor_ready, set_editor_ready) = signal(false);

    // Initialize CodeMirror editor
    Effect::new(move || {
        let on_run = Closure::new(move || {
            run_query(set_results, set_running, set_error, set_row_count, set_history);
        });
        interop::js_editor_init(
            "sparql-editor-container",
            "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 25",
            &on_run,
        );
        on_run.forget();
        set_editor_ready.set(true);
    });

    // Fetch schema data for autocomplete
    Effect::new(move || {
        spawn_local(async move {
            let types = api::fetch_entity_types().await.unwrap_or_default();
            let preds = api::fetch_predicates().await.unwrap_or_default();
            let t = serde_json::to_string(&types).unwrap_or_else(|_| "[]".into());
            let p = serde_json::to_string(&preds).unwrap_or_else(|_| "[]".into());
            interop::js_editor_set_completion_data(&t, &p);
        });
    });

    // Encode query in URL hash for shareability
    Effect::new(move || {
        if !results.get().is_null() {
            let query = interop::js_editor_get_value();
            if let Some(w) = web_sys::window() {
                let enc = js_sys::encode_uri_component(&query);
                let _ = w.location().set_hash(&format!("q={}", enc.as_string().unwrap_or_default()));
            }
        }
    });

    // Restore query from URL hash on mount
    Effect::new(move || {
        let _ = editor_ready.get();
        if let Some(w) = web_sys::window() {
            if let Ok(hash) = w.location().hash() {
                if let Some(enc) = hash.strip_prefix("#q=") {
                    if let Some(q) = js_sys::decode_uri_component(enc).ok().and_then(|v| v.as_string()) {
                        interop::js_editor_set_value(&q);
                    }
                }
            }
        }
    });

    let on_run_click = move |_| {
        run_query(set_results, set_running, set_error, set_row_count, set_history);
    };

    view! {
        <div class="sparql-view">
            <div class="sparql-editor">
                <div class="sparql-toolbar">
                    <button class="btn btn-primary" on:click=on_run_click disabled=move || running.get()>
                        {move || if running.get() { "Running..." } else { "Run (Ctrl+Enter)" }}
                    </button>
                    <div class="toolbar-group">
                        <button class="btn" class:active=move || show_templates.get()
                            on:click=move |_| { set_show_templates.update(|v| *v = !*v); set_show_history.set(false); }>
                            "Templates"
                        </button>
                        <button class="btn" class:active=move || show_history.get()
                            on:click=move |_| { set_show_history.update(|v| *v = !*v); set_show_templates.set(false); }>
                            "History"
                        </button>
                    </div>
                    <div class="toolbar-spacer"></div>
                    {move || {
                        let c = row_count.get();
                        if c > 0 { view! { <span class="result-count">{format!("{c} rows")}</span> }.into_any() }
                        else { view! { <span></span> }.into_any() }
                    }}
                </div>
                {move || if show_templates.get() {
                    view! { <div class="templates-panel">
                        {TEMPLATES.iter().map(|t| { let sparql = t.sparql; view! {
                            <div class="template-item" on:click=move |_| { interop::js_editor_set_value(sparql); set_show_templates.set(false); }>
                                <span class="template-name">{t.name}</span>
                                <span class="template-persona">{t.persona}</span>
                            </div>
                        }}).collect::<Vec<_>>()}
                    </div> }.into_any()
                } else { view! { <div></div> }.into_any() }}
                {move || if show_history.get() {
                    let entries = history.get();
                    if entries.is_empty() {
                        view! { <div class="history-panel"><div class="history-empty">"No query history yet"</div></div> }.into_any()
                    } else {
                        view! { <div class="history-panel">
                            {entries.iter().rev().take(20).map(|e| {
                                let query = e.query.clone();
                                let display = if query.len() > 80 { format!("{}...", &query[..80]) } else { query.clone() };
                                view! { <div class="history-item" on:click=move |_| { interop::js_editor_set_value(&query); set_show_history.set(false); }>
                                    <span class="history-query">{display}</span>
                                    <span class="history-meta">{format!("{} rows", e.row_count)}</span>
                                </div> }
                            }).collect::<Vec<_>>()}
                        </div> }.into_any()
                    }
                } else { view! { <div></div> }.into_any() }}
                <div id="sparql-editor-container" class="cm-editor-container"></div>
            </div>
            <div class="sparql-results">
                <div class="result-tabs">
                    <button class="result-tab" class:active=move || result_view.get() == ResultView::Table
                        on:click=move |_| set_result_view.set(ResultView::Table)>"Table"</button>
                    <button class="result-tab" class:active=move || result_view.get() == ResultView::Graph
                        on:click=move |_| set_result_view.set(ResultView::Graph)>"Graph"</button>
                    <button class="result-tab" class:active=move || result_view.get() == ResultView::Json
                        on:click=move |_| set_result_view.set(ResultView::Json)>"JSON"</button>
                </div>
                <div class="result-content">
                    {move || {
                        if let Some(err) = error.get() {
                            return view! { <div class="result-error">{format!("Error: {err}")}</div> }.into_any();
                        }
                        let res = results.get();
                        if res.is_null() {
                            return view! { <div class="loading">"Run a query to see results (Ctrl+Enter)"</div> }.into_any();
                        }
                        match result_view.get() {
                            ResultView::Table => render_table(&res).into_any(),
                            ResultView::Graph => render_graph(&res).into_any(),
                            ResultView::Json => render_json(&res).into_any(),
                        }
                    }}
                </div>
            </div>
        </div>
    }
}

fn run_query(
    set_results: WriteSignal<serde_json::Value>, set_running: WriteSignal<bool>,
    set_error: WriteSignal<Option<String>>, set_row_count: WriteSignal<usize>,
    set_history: WriteSignal<Vec<HistoryEntry>>,
) {
    let sparql = interop::js_editor_get_value();
    if sparql.trim().is_empty() { return; }
    set_running.set(true);
    set_error.set(None);
    spawn_local(async move {
        match api::sparql_query(&sparql).await {
            Ok(r) => {
                let count = r.get("rows").and_then(|r| r.as_array()).map(|a| a.len()).unwrap_or(0);
                let entry = HistoryEntry {
                    query: sparql,
                    timestamp: js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default(),
                    row_count: count,
                };
                set_history.update(|h| { h.push(entry); if h.len() > 50 { h.remove(0); } save_history(h); });
                set_row_count.set(count);
                set_results.set(r);
                set_running.set(false);
            }
            Err(e) => { set_error.set(Some(e)); set_running.set(false); }
        }
    });
}

fn cell(row: &serde_json::Value, key: &str) -> String {
    row.get(key)
        .and_then(|v| v.as_str().map(String::from).or_else(|| v.get("value")?.as_str().map(String::from)))
        .unwrap_or_else(|| "\u{2014}".into())
}

fn parse_columns(res: &serde_json::Value) -> Vec<String> {
    res.get("columns").and_then(|c| c.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn parse_rows(res: &serde_json::Value) -> Vec<serde_json::Value> {
    res.get("rows").and_then(|r| r.as_array()).cloned().unwrap_or_default()
}

fn render_table(res: &serde_json::Value) -> impl IntoView {
    let columns = parse_columns(res);
    let rows = parse_rows(res);
    view! {
        <table class="results-table">
            <thead><tr>{columns.iter().map(|c| view! { <th>{c.clone()}</th> }).collect::<Vec<_>>()}</tr></thead>
            <tbody>
                {rows.iter().map(|row| { let cols = columns.clone(); view! { <tr>
                    {cols.iter().map(|col| {
                        let val = cell(row, col);
                        if api::is_iri(&val) {
                            let href = format!("/entity/{}", js_sys::encode_uri_component(&val).as_string().unwrap_or_default());
                            let display = api::short_name(&val);
                            view! { <td><a href=href title=val.clone()>{display}</a></td> }.into_any()
                        } else { view! { <td>{val}</td> }.into_any() }
                    }).collect::<Vec<_>>()}
                </tr> }}).collect::<Vec<_>>()}
            </tbody>
        </table>
    }
}

fn render_graph(res: &serde_json::Value) -> impl IntoView {
    let cols = parse_columns(res);
    let rows = parse_rows(res);
    let has_spo = cols.contains(&"s".into()) && cols.contains(&"p".into()) && cols.contains(&"o".into());
    if !has_spo || rows.is_empty() {
        return view! { <div class="graph-hint">"Graph view requires ?s ?p ?o columns."</div> }.into_any();
    }
    let rows_clone = rows.clone();
    Effect::new(move || {
        if interop::js_schema_init("sparql-result-graph") {
            for row in &rows_clone {
                let (s, p, o) = (cell(row, "s"), cell(row, "p"), cell(row, "o"));
                if api::is_iri(&s) { interop::js_schema_add_class(&s, &api::short_name(&s), 0); }
                if api::is_iri(&o) {
                    interop::js_schema_add_class(&o, &api::short_name(&o), 0);
                    interop::js_schema_add_property(&s, &o, &api::short_name(&p));
                }
            }
            interop::js_schema_layout();
        }
    });
    view! { <div id="sparql-result-graph" class="result-graph-container"></div> }.into_any()
}

fn render_json(res: &serde_json::Value) -> impl IntoView {
    let text = serde_json::to_string_pretty(res).unwrap_or_else(|_| "{}".into());
    view! { <pre class="json-view">{text}</pre> }
}

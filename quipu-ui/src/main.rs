//! Quipu UI — Leptos WASM frontend for the knowledge graph explorer.
//!
//! Routes:
//!   /              → Graph explorer (Sigma.js + Graphology)
//!   /entity/{iri}  → Entity detail page with JSON-LD
//!   /sparql        → SPARQL workbench
//!   /schema        → Schema browser (placeholder)
//!   /timeline      → Temporal navigator (placeholder)

mod api;
mod components;
mod interop;

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use wasm_bindgen_futures::spawn_local;

use components::entity_sidebar::EntityPage;
use components::graph_explorer::GraphExplorer;

fn main() {
    console_error_panic_hook::set_once();
    _ = console_log::init_with_level(log::Level::Debug);

    mount_to_body(App);
}

/// Root application component.
#[component]
fn App() -> impl IntoView {
    let (stats, set_stats) = signal(serde_json::Value::Null);

    // Fetch server stats on mount
    Effect::new(move || {
        spawn_local(async move {
            match api::fetch_stats().await {
                Ok(s) => set_stats.set(s),
                Err(e) => log::warn!("Stats unavailable: {e}"),
            }
        });
    });

    view! {
        <Router>
            <div id="app">
                <header class="app-header">
                    <h1><span class="accent">"Q"</span>"uipu"</h1>
                    <div class="header-stats">
                        {move || {
                            let s = stats.get();
                            let facts = s
                                .get("facts")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let entities = s
                                .get("entities")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let connected = facts > 0;
                            view! {
                                <span class=if connected {
                                    "status-dot ok"
                                } else {
                                    "status-dot"
                                }></span>
                                {format!("{entities} entities / {facts} facts")}
                            }
                        }}
                    </div>
                </header>

                <nav class="app-nav">
                    <a href="/" class="active">"Explorer"</a>
                    <a href="/sparql">"SPARQL"</a>
                    <a href="/schema">"Schema"</a>
                    <a href="/timeline">"Timeline"</a>
                </nav>

                <main class="app-main">
                    <Routes fallback=|| "Page not found">
                        <Route path=path!("/") view=GraphExplorer />
                        <Route path=path!("/entity/:iri") view=EntityPage />
                        <Route path=path!("/sparql") view=SparqlWorkbench />
                        <Route path=path!("/schema") view=SchemaView />
                        <Route path=path!("/timeline") view=TimelineView />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}

/// SPARQL workbench — textarea + run button + results table.
#[component]
fn SparqlWorkbench() -> impl IntoView {
    let (query_text, set_query_text) = signal(
        "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 25".to_string(),
    );
    let (results, set_results) = signal(serde_json::Value::Null);
    let (running, set_running) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);

    let run_query = move |_| {
        let sparql = query_text.get();
        set_running.set(true);
        set_error.set(None);
        spawn_local(async move {
            match api::sparql_query(&sparql).await {
                Ok(r) => {
                    set_results.set(r);
                    set_running.set(false);
                }
                Err(e) => {
                    set_error.set(Some(e));
                    set_running.set(false);
                }
            }
        });
    };

    view! {
        <div class="sparql-view">
            <div class="sparql-editor">
                <div class="sparql-toolbar">
                    <button class="btn btn-primary" on:click=run_query disabled=move || running.get()>
                        {move || if running.get() { "Running..." } else { "Run Query" }}
                    </button>
                </div>
                <textarea
                    class="sparql-textarea"
                    prop:value=move || query_text.get()
                    on:input=move |ev| {
                        use wasm_bindgen::JsCast;
                        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                        set_query_text.set(target.value());
                    }
                    spellcheck="false"
                ></textarea>
            </div>

            <div class="sparql-results">
                {move || {
                    if let Some(err) = error.get() {
                        return view! {
                            <div style="color: var(--error); padding: 0.5rem;">
                                {format!("Error: {err}")}
                            </div>
                        }
                        .into_any();
                    }

                    let res = results.get();
                    if res.is_null() {
                        return view! {
                            <div class="loading">"Run a query to see results"</div>
                        }
                        .into_any();
                    }

                    // Parse columns and rows
                    let columns: Vec<String> = res
                        .get("columns")
                        .and_then(|c| c.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let rows: Vec<serde_json::Value> = res
                        .get("rows")
                        .and_then(|r| r.as_array())
                        .cloned()
                        .unwrap_or_default();

                    view! {
                        <table class="results-table">
                            <thead>
                                <tr>
                                    {columns
                                        .iter()
                                        .map(|c| {
                                            view! { <th>{c.clone()}</th> }
                                        })
                                        .collect::<Vec<_>>()}
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .iter()
                                    .map(|row| {
                                        let cols = columns.clone();
                                        view! {
                                            <tr>
                                                {cols
                                                    .iter()
                                                    .map(|col| {
                                                        let val = row
                                                            .get(col)
                                                            .and_then(|v| {
                                                                v.as_str().map(String::from).or_else(|| {
                                                                    v
                                                                        .get("value")
                                                                        .and_then(|x| x.as_str())
                                                                        .map(String::from)
                                                                })
                                                            })
                                                            .unwrap_or_else(|| "—".to_string());
                                                        view! { <td>{val}</td> }
                                                    })
                                                    .collect::<Vec<_>>()}
                                            </tr>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </tbody>
                        </table>
                    }
                    .into_any()
                }}
            </div>
        </div>
    }
}

/// Schema browser — placeholder for Phase 2.
#[component]
fn SchemaView() -> impl IntoView {
    view! {
        <div class="placeholder-view">
            "Schema browser — coming in Phase 2"
        </div>
    }
}

/// Timeline view — placeholder for Phase 3.
#[component]
fn TimelineView() -> impl IntoView {
    view! {
        <div class="placeholder-view">
            "Temporal navigator — coming in Phase 3"
        </div>
    }
}

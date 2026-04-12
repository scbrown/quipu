//! Quipu UI — Leptos WASM frontend for the knowledge graph explorer.
//!
//! Routes:
//!   /              → Graph explorer (Sigma.js + Graphology)
//!   /entity/{iri}  → Entity detail page with JSON-LD
//!   /sparql        → SPARQL workbench
//!   /schema        → Schema browser
//!   /timeline      → Temporal navigator (placeholder)

mod api;
mod components;
mod interop;

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use wasm_bindgen_futures::spawn_local;

use components::entity_sidebar::EntityPage;
use components::episode_timeline::EpisodeTimeline;
use components::graph_explorer::GraphExplorer;
use components::schema_browser::SchemaBrowser;
use components::sparql_workbench::SparqlWorkbench;

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
                        <Route path=path!("/schema") view=SchemaBrowser />
                        <Route path=path!("/timeline") view=EpisodeTimeline />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}

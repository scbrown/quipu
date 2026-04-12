//! Graph explorer: Sigma.js force-directed graph + entity sidebar + type filters.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::temporal_navigator::{EntityHistory, GraphDiffControls, TemporalControls};
use crate::api;
use crate::interop;

/// Entity node data from the API.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityNode {
    pub iri: String,
    pub label: String,
    pub entity_type: String,
}

/// A single fact (predicate + value).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fact {
    pub predicate: String,
    pub value: String,
    pub is_iri: bool,
}

/// A group of facts sharing the same predicate.
#[derive(Clone, Debug)]
pub struct FactGroup {
    pub predicate: String,
    pub values: Vec<Fact>,
}

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

fn type_color(entity_type: &str) -> &'static str {
    match entity_type {
        "Host" => "#4cc9f0",
        "Service" => "#4ecca3",
        "Person" => "#f7a072",
        "Organization" => "#e94560",
        "Event" => "#fca311",
        "Place" => "#533483",
        _ => "#8892a4",
    }
}

/// Push nodes and edges to the JS graph, then run layout.
fn populate_graph(nodes: &[EntityNode], edges: &[(String, String, String)]) {
    for node in nodes {
        interop::js_add_node(&node.iri, &node.label, &node.entity_type, 0.0, 0.0);
    }
    for (src, tgt, label) in edges {
        interop::js_add_edge(src, tgt, label);
    }
    interop::js_start_layout();
}

/// The main graph explorer component.
#[component]
pub fn GraphExplorer() -> impl IntoView {
    let (entities, set_entities) = signal(Vec::<EntityNode>::new());
    let (selected_node, set_selected_node) = signal(Option::<String>::None);
    let (node_facts, set_node_facts) = signal(Vec::<Fact>::new());
    let (search_term, set_search_term) = signal(String::new());
    let (active_types, set_active_types) = signal(Vec::<String>::new());
    let (graph_initialized, set_graph_initialized) = signal(false);
    let (loading, set_loading) = signal(true);
    let (max_tx, set_max_tx) = signal(1i64);

    // Fetch max tx on mount for diff controls
    Effect::new(move || {
        spawn_local(async move {
            if let Ok(result) = api::fetch_transactions().await {
                if let Some(id) = result
                    .get("transactions")
                    .and_then(|t| t.as_array())
                    .and_then(|a| a.last())
                    .and_then(|v| v.get("id"))
                    .and_then(|v| v.as_i64())
                {
                    set_max_tx.set(id);
                }
            }
        });
    });

    let entity_types = Memo::new(move |_| {
        let mut types: Vec<String> = entities
            .get()
            .iter()
            .map(|e| e.entity_type.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        types.sort();
        types
    });

    let filtered_entities = Memo::new(move |_| {
        let term = search_term.get().to_lowercase();
        let types = active_types.get();
        entities
            .get()
            .iter()
            .filter(|e| {
                (term.is_empty() || e.label.to_lowercase().contains(&term))
                    && (types.is_empty() || types.contains(&e.entity_type))
            })
            .cloned()
            .collect::<Vec<_>>()
    });

    // Initialize graph and load data on mount
    Effect::new(move || {
        if graph_initialized.get() {
            return;
        }
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(100).await;
            if !interop::js_init_graph("sigma-container") {
                log::error!("Failed to initialize Sigma.js graph");
                return;
            }
            let click_cb = Closure::new(move |node_id: String| {
                set_selected_node.set(Some(node_id.clone()));
                interop::js_highlight_node(&node_id);
                let nid = node_id.clone();
                spawn_local(async move {
                    match api::fetch_entity_facts(&nid).await {
                        Ok(facts) => set_node_facts.set(facts),
                        Err(e) => log::error!("Failed to load facts: {e}"),
                    }
                });
            });
            let dblclick_cb = Closure::new(move |node_id: String| {
                spawn_local(async move {
                    match api::expand_neighborhood(&node_id).await {
                        Ok((nodes, edges)) => {
                            populate_graph(&nodes, &edges);
                            set_entities.update(|cur| {
                                for n in nodes {
                                    if !cur.iter().any(|e| e.iri == n.iri) {
                                        cur.push(n);
                                    }
                                }
                            });
                        }
                        Err(e) => log::error!("Failed to expand: {e}"),
                    }
                });
            });
            let stage_cb = Closure::new(move || {
                set_selected_node.set(None);
                set_node_facts.set(Vec::new());
            });
            interop::js_register_callbacks(&click_cb, &dblclick_cb, &stage_cb);
            click_cb.forget();
            dblclick_cb.forget();
            stage_cb.forget();
            set_graph_initialized.set(true);
            match api::fetch_initial_graph().await {
                Ok((nodes, edges)) => {
                    populate_graph(&nodes, &edges);
                    set_entities.set(nodes);
                    set_loading.set(false);
                }
                Err(e) => {
                    log::error!("Failed to load initial graph: {e}");
                    set_loading.set(false);
                }
            }
        });
    });

    Effect::new(move || {
        let types = active_types.get();
        if !graph_initialized.get() {
            return;
        }
        for entity in entities.get().iter() {
            let visible = types.is_empty() || types.contains(&entity.entity_type);
            interop::js_set_node_visibility(&entity.iri, visible);
        }
    });

    // Temporal change: re-query graph with time context
    let on_temporal_change =
        Callback::new(move |(valid_at, as_of_tx): (Option<String>, Option<i64>)| {
            if !graph_initialized.get() {
                return;
            }
            set_loading.set(true);
            spawn_local(async move {
                match api::fetch_temporal_graph(valid_at.as_deref(), as_of_tx).await {
                    Ok((nodes, edges)) => {
                        interop::js_clear_graph();
                        populate_graph(&nodes, &edges);
                        set_entities.set(nodes);
                        set_loading.set(false);
                    }
                    Err(e) => {
                        log::error!("Temporal query failed: {e}");
                        set_loading.set(false);
                    }
                }
            });
        });

    // Graph diff: compare two transaction points
    let on_diff = Callback::new(move |(t1, t2): (i64, i64)| {
        if !graph_initialized.get() {
            return;
        }
        set_loading.set(true);
        spawn_local(async move {
            let (g1, g2) = (
                api::fetch_temporal_graph(None, Some(t1)).await,
                api::fetch_temporal_graph(None, Some(t2)).await,
            );
            if let (Ok((n1, _)), Ok((n2, e2))) = (g1, g2) {
                let iris1: std::collections::HashSet<_> = n1.iter().map(|n| &n.iri).collect();
                let iris2: std::collections::HashSet<_> = n2.iter().map(|n| &n.iri).collect();
                interop::js_clear_graph();
                for node in &n2 {
                    let dt = if iris1.contains(&node.iri) { &node.entity_type } else { "diff-added" };
                    interop::js_add_node(&node.iri, &node.label, dt, 0.0, 0.0);
                }
                for node in &n1 {
                    if !iris2.contains(&node.iri) {
                        interop::js_add_node(&node.iri, &node.label, "diff-removed", 0.0, 0.0);
                    }
                }
                for (s, t, l) in &e2 { interop::js_add_edge(s, t, l); }
                interop::js_start_layout();
                let mut all = n2;
                for n in n1 { if !all.iter().any(|e| e.iri == n.iri) { all.push(n); } }
                set_entities.set(all);
            } else {
                log::error!("Failed to compute graph diff");
            }
            set_loading.set(false);
        });
    });

    let on_clear_diff = Callback::new(move |()| {
        set_loading.set(true);
        spawn_local(async move {
            match api::fetch_initial_graph().await {
                Ok((nodes, edges)) => {
                    interop::js_clear_graph();
                    populate_graph(&nodes, &edges);
                    set_entities.set(nodes);
                }
                Err(e) => log::error!("Failed to reload graph: {e}"),
            }
            set_loading.set(false);
        });
    });

    let entity_iri_signal = Signal::derive(move || selected_node.get());

    view! {
        <div class="explorer">
            <div class="explorer-sidebar">
                <div class="sidebar-search">
                    <input type="text" placeholder="Search entities..."
                        prop:value=move || search_term.get()
                        on:input=move |ev| {
                            use wasm_bindgen::JsCast;
                            let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                            set_search_term.set(target.value());
                        }
                    />
                </div>
                <div class="type-filters">
                    {move || entity_types.get().iter().map(|t| {
                        let t = t.clone(); let t2 = t.clone();
                        let cls = { let t = t.clone(); move || if active_types.get().contains(&t) { "type-badge active" } else { "type-badge" } };
                        let sty = { let t = t2.clone(); move || if active_types.get().contains(&t) { format!("border-color: {}", type_color(&t)) } else { String::new() } };
                        view! { <span class=cls style=sty on:click={ let t = t.clone(); move |_| { set_active_types.update(|v| { if let Some(p) = v.iter().position(|x| x == &t) { v.remove(p); } else { v.push(t.clone()); } }); } }>{t.clone()}</span> }
                    }).collect::<Vec<_>>()}
                </div>
                <div class="entity-list">
                    {move || {
                        if loading.get() { return vec![view! { <div class="loading">"Loading..."</div> }.into_any()]; }
                        filtered_entities.get().iter().map(|e| {
                            let iri = e.iri.clone(); let iri2 = e.iri.clone();
                            let label = e.label.clone(); let color = type_color(&e.entity_type).to_string();
                            view! {
                                <div class=move || if selected_node.get().as_deref() == Some(&iri) { "entity-item selected" } else { "entity-item" }
                                    on:click={ let iri = iri2.clone(); move |_| {
                                        set_selected_node.set(Some(iri.clone()));
                                        interop::js_highlight_node(&iri);
                                        interop::js_focus_node(&iri);
                                        let i = iri.clone();
                                        spawn_local(async move { match api::fetch_entity_facts(&i).await { Ok(f) => set_node_facts.set(f), Err(e) => log::error!("Facts: {e}") } });
                                    } }>
                                    <span class="type-dot" style=format!("background: {color}")></span>
                                    <span class="entity-name">{label}</span>
                                </div>
                            }.into_any()
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
            <div class="graph-area">
                <div id="sigma-container" class="graph-container"></div>
                <div class="graph-controls">
                    <button class="btn" on:click=move |_| { interop::js_start_layout(); }>"Layout"</button>
                </div>
                <TemporalControls on_temporal_change=on_temporal_change />
                <GraphDiffControls on_diff=on_diff on_clear_diff=on_clear_diff max_tx=Signal::derive(move || max_tx.get()) />
            </div>
            {move || {
                let sel = selected_node.get();
                if sel.is_none() { return view! { <div class="detail-panel hidden"></div> }.into_any(); }
                let node_id = sel.unwrap();
                let label = entities.get().iter().find(|e| e.iri == node_id).map(|e| e.label.clone()).unwrap_or_else(|| short_name(&node_id));
                let groups = group_facts(&node_facts.get());
                view! {
                    <div class="detail-panel">
                        <h3>{label}</h3>
                        <div class="detail-iri">{node_id.clone()}</div>
                        {groups.iter().map(|g| {
                            let pred_name = short_name(&g.predicate);
                            let values = g.values.clone();
                            view! {
                                <div class="fact-group">
                                    <div class="fact-group-header">{pred_name}</div>
                                    {values.iter().map(|v| {
                                        if v.is_iri {
                                            let href = format!("/entity/{}", urlencoding(&v.value));
                                            view! { <div class="fact-value"><a href=href>{short_name(&v.value)}</a></div> }.into_any()
                                        } else {
                                            view! { <div class="fact-value">{v.value.clone()}</div> }.into_any()
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                        <EntityHistory entity_iri=entity_iri_signal />
                    </div>
                }.into_any()
            }}
        </div>
    }
}

fn urlencoding(s: &str) -> String {
    s.replace('%', "%25").replace(' ', "%20").replace('#', "%23")
        .replace('?', "%3F").replace('&', "%26").replace('=', "%3D")
}

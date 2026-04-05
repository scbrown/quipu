//! Graph explorer: Sigma.js force-directed graph + entity sidebar + type filters.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

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

/// Group facts by predicate for the detail sidebar.
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

/// Shorten an IRI to its local name for display.
fn short_name(iri: &str) -> String {
    // Strip < > wrapper if present
    let iri = iri.trim_start_matches('<').trim_end_matches('>');
    // Take fragment or last path segment
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

/// Color for a given entity type.
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

    // All discovered entity types
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

    // Filtered entities for the sidebar list
    let filtered_entities = Memo::new(move |_| {
        let term = search_term.get().to_lowercase();
        let types = active_types.get();
        entities
            .get()
            .iter()
            .filter(|e| {
                let name_match = term.is_empty() || e.label.to_lowercase().contains(&term);
                let type_match = types.is_empty() || types.contains(&e.entity_type);
                name_match && type_match
            })
            .cloned()
            .collect::<Vec<_>>()
    });

    // Initialize graph and load data on mount
    Effect::new(move || {
        if graph_initialized.get() {
            return;
        }

        // Use a small delay to ensure the DOM container exists
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(100).await;

            let ok = interop::js_init_graph("sigma-container");
            if !ok {
                log::error!("Failed to initialize Sigma.js graph");
                return;
            }

            // Register JS -> Rust callbacks
            let click_cb = Closure::new(move |node_id: String| {
                set_selected_node.set(Some(node_id.clone()));
                interop::js_highlight_node(&node_id);
                // Load facts for this node
                let node_id_inner = node_id.clone();
                spawn_local(async move {
                    match api::fetch_entity_facts(&node_id_inner).await {
                        Ok(facts) => set_node_facts.set(facts),
                        Err(e) => log::error!("Failed to load facts: {e}"),
                    }
                });
            });

            let dblclick_cb = Closure::new(move |node_id: String| {
                // Expand 1-hop neighborhood
                let node_id = node_id.clone();
                spawn_local(async move {
                    match api::expand_neighborhood(&node_id).await {
                        Ok((nodes, edges)) => {
                            for node in &nodes {
                                interop::js_add_node(
                                    &node.iri,
                                    &node.label,
                                    &node.entity_type,
                                    0.0,
                                    0.0,
                                );
                            }
                            for (src, tgt, label) in &edges {
                                interop::js_add_edge(src, tgt, label);
                            }
                            interop::js_start_layout();
                            // Update entity list
                            set_entities.update(|current| {
                                for node in nodes {
                                    if !current.iter().any(|e| e.iri == node.iri) {
                                        current.push(node);
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

            // Leak closures so they persist (they live for the app lifetime)
            click_cb.forget();
            dblclick_cb.forget();
            stage_cb.forget();

            set_graph_initialized.set(true);

            // Load initial subgraph
            match api::fetch_initial_graph().await {
                Ok((nodes, edges)) => {
                    for node in &nodes {
                        interop::js_add_node(
                            &node.iri,
                            &node.label,
                            &node.entity_type,
                            0.0,
                            0.0,
                        );
                    }
                    for (src, tgt, label) in &edges {
                        interop::js_add_edge(src, tgt, label);
                    }
                    interop::js_start_layout();
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

    // Apply type filtering to the graph when active_types changes
    Effect::new(move || {
        let types = active_types.get();
        if !graph_initialized.get() {
            return;
        }
        let all_entities = entities.get();
        for entity in all_entities.iter() {
            let visible = types.is_empty() || types.contains(&entity.entity_type);
            interop::js_set_node_visibility(&entity.iri, visible);
        }
    });

    view! {
        <div class="explorer">
            // Left sidebar: search + type filters + entity list
            <div class="explorer-sidebar">
                <div class="sidebar-search">
                    <input
                        type="text"
                        placeholder="Search entities..."
                        prop:value=move || search_term.get()
                        on:input=move |ev| {
                            use wasm_bindgen::JsCast;
                            let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                            set_search_term.set(target.value());
                        }
                    />
                </div>

                <div class="type-filters">
                    {move || {
                        entity_types
                            .get()
                            .iter()
                            .map(|t| {
                                let t = t.clone();
                                let t2 = t.clone();
                                let is_active_class = {
                                    let t = t.clone();
                                    move || {
                                        if active_types.get().contains(&t) {
                                            "type-badge active"
                                        } else {
                                            "type-badge"
                                        }
                                    }
                                };
                                let is_active_style = {
                                    let t = t2.clone();
                                    move || {
                                        if active_types.get().contains(&t) {
                                            format!("border-color: {}", type_color(&t))
                                        } else {
                                            String::new()
                                        }
                                    }
                                };
                                view! {
                                    <span
                                        class=is_active_class
                                        style=is_active_style
                                        on:click={
                                            let t = t.clone();
                                            move |_| {
                                                set_active_types.update(|types| {
                                                    if let Some(pos) = types.iter().position(|x| x == &t) {
                                                        types.remove(pos);
                                                    } else {
                                                        types.push(t.clone());
                                                    }
                                                });
                                            }
                                        }
                                    >
                                        {t.clone()}
                                    </span>
                                }
                            })
                            .collect::<Vec<_>>()
                    }}
                </div>

                <div class="entity-list">
                    {move || {
                        if loading.get() {
                            return vec![view! { <div class="loading">"Loading..."</div> }.into_any()];
                        }
                        filtered_entities
                            .get()
                            .iter()
                            .map(|e| {
                                let iri = e.iri.clone();
                                let iri2 = e.iri.clone();
                                let label = e.label.clone();
                                let color = type_color(&e.entity_type).to_string();
                                view! {
                                    <div
                                        class=move || {
                                            let sel = selected_node.get();
                                            if sel.as_deref() == Some(&iri) {
                                                "entity-item selected"
                                            } else {
                                                "entity-item"
                                            }
                                        }
                                        on:click={
                                            let iri = iri2.clone();
                                            move |_| {
                                                set_selected_node.set(Some(iri.clone()));
                                                interop::js_highlight_node(&iri);
                                                interop::js_focus_node(&iri);
                                                let iri_inner = iri.clone();
                                                spawn_local(async move {
                                                    match api::fetch_entity_facts(&iri_inner).await {
                                                        Ok(facts) => set_node_facts.set(facts),
                                                        Err(e) => log::error!("Facts: {e}"),
                                                    }
                                                });
                                            }
                                        }
                                    >
                                        <span
                                            class="type-dot"
                                            style=format!("background: {color}")
                                        ></span>
                                        <span class="entity-name">{label}</span>
                                    </div>
                                }
                                .into_any()
                            })
                            .collect::<Vec<_>>()
                    }}
                </div>
            </div>

            // Center: Sigma.js graph
            <div class="graph-area">
                <div id="sigma-container" class="graph-container"></div>
                <div class="graph-controls">
                    <button
                        class="btn"
                        on:click=move |_| { interop::js_start_layout(); }
                    >
                        "Layout"
                    </button>
                </div>
            </div>

            // Right: detail panel (shown when a node is selected)
            {move || {
                let sel = selected_node.get();
                if sel.is_none() {
                    return view! { <div class="detail-panel hidden"></div> }.into_any();
                }
                let node_id = sel.unwrap();
                let label = entities
                    .get()
                    .iter()
                    .find(|e| e.iri == node_id)
                    .map(|e| e.label.clone())
                    .unwrap_or_else(|| short_name(&node_id));
                let groups = group_facts(&node_facts.get());

                view! {
                    <div class="detail-panel">
                        <h3>{label}</h3>
                        <div class="detail-iri">{node_id.clone()}</div>
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
                                                    let href = format!("/entity/{}", urlencoding(&v.value));
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

/// Simple URL encoding for IRIs.
fn urlencoding(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F")
        .replace('&', "%26")
        .replace('=', "%3D")
}

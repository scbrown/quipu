//! Episode timeline — chronological view of ingested episodes.
//!
//! Shows all prov:Activity entities with source type badges, entity counts,
//! expandable mini-graph, and filters by source type and date range.

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;

/// A parsed episode entry.
#[derive(Clone, Debug, PartialEq)]
struct EpisodeEntry {
    iri: String,
    label: String,
    source: Option<String>,
    comment: Option<String>,
}

/// Source type filter badge.
fn source_badge(source: &Option<String>) -> (&'static str, &'static str) {
    match source.as_deref() {
        Some(s) if s.contains("crew") || s.contains("agent") => ("agent", "#f7a072"),
        Some(s) if s.contains("ci") || s.contains("pipeline") => ("ci", "#4cc9f0"),
        Some(s) if s.contains("incident") || s.contains("alert") => ("incident", "#e94560"),
        Some(s) if s.contains("observation") => ("obs", "#fca311"),
        Some(_) => ("manual", "#4ecca3"),
        None => ("unknown", "#8892a4"),
    }
}

/// The timeline view component — replaces the placeholder.
#[component]
pub fn EpisodeTimeline() -> impl IntoView {
    let (episodes, set_episodes) = signal(Vec::<EpisodeEntry>::new());
    let (loading, set_loading) = signal(true);
    let (source_filter, set_source_filter) = signal(Option::<String>::None);
    let (expanded_ep, set_expanded_ep) = signal(Option::<String>::None);
    let (expanded_data, set_expanded_data) = signal(serde_json::Value::Null);
    let (search_term, set_search_term) = signal(String::new());

    // Fetch episodes on mount
    Effect::new(move || {
        spawn_local(async move {
            match api::fetch_episodes().await {
                Ok(result) => {
                    let rows = result
                        .get("rows")
                        .and_then(|r| r.as_array())
                        .cloned()
                        .unwrap_or_default();

                    let mut eps = Vec::new();
                    for row in &rows {
                        let iri = extract_val(row, "ep");
                        let label = extract_val(row, "label");
                        if let (Some(iri), Some(label)) = (iri, label) {
                            eps.push(EpisodeEntry {
                                iri,
                                label,
                                source: extract_val(row, "source"),
                                comment: extract_val(row, "comment"),
                            });
                        }
                    }
                    set_episodes.set(eps);
                    set_loading.set(false);
                }
                Err(e) => {
                    log::error!("Failed to load episodes: {e}");
                    set_loading.set(false);
                }
            }
        });
    });

    // Filter episodes
    let filtered = Memo::new(move |_| {
        let term = search_term.get().to_lowercase();
        let sf = source_filter.get();
        episodes
            .get()
            .iter()
            .filter(|ep| {
                let name_match = term.is_empty() || ep.label.to_lowercase().contains(&term);
                let source_match = match &sf {
                    Some(filter) => {
                        let (badge, _) = source_badge(&ep.source);
                        badge == filter.as_str()
                    }
                    None => true,
                };
                name_match && source_match
            })
            .cloned()
            .collect::<Vec<_>>()
    });

    // Available source types for filter badges
    let source_types = Memo::new(move |_| {
        let mut types = std::collections::BTreeSet::new();
        for ep in episodes.get().iter() {
            let (badge, _) = source_badge(&ep.source);
            types.insert(badge.to_string());
        }
        types.into_iter().collect::<Vec<_>>()
    });

    view! {
        <div class="timeline-view">
            <div class="timeline-toolbar">
                <div class="timeline-search">
                    <input
                        type="text"
                        placeholder="Search episodes..."
                        prop:value=move || search_term.get()
                        on:input=move |ev| {
                            use wasm_bindgen::JsCast;
                            let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                            set_search_term.set(target.value());
                        }
                    />
                </div>
                <div class="timeline-filters">
                    <button
                        class=move || if source_filter.get().is_none() { "type-badge active" } else { "type-badge" }
                        on:click=move |_| set_source_filter.set(None)
                    >"All"</button>
                    {move || {
                        source_types.get().iter().map(|st| {
                            let st_clone = st.clone();
                            let st2 = st.clone();
                            let st3 = st.clone();
                            view! {
                                <button
                                    class=move || {
                                        if source_filter.get().as_deref() == Some(&st_clone) {
                                            "type-badge active"
                                        } else {
                                            "type-badge"
                                        }
                                    }
                                    on:click=move |_| set_source_filter.set(Some(st2.clone()))
                                >{st3.clone()}</button>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>

            <div class="timeline-content">
                {move || {
                    if loading.get() {
                        return view! { <div class="loading">"Loading episodes..."</div> }.into_any();
                    }
                    let eps = filtered.get();
                    if eps.is_empty() {
                        return view! {
                            <div class="timeline-empty">
                                "No episodes found. Episodes are created when knowledge is ingested via the /episode endpoint."
                            </div>
                        }.into_any();
                    }
                    view! {
                        <div class="timeline-list">
                            {eps.iter().map(|ep| {
                                let iri = ep.iri.clone();
                                let iri2 = ep.iri.clone();
                                let iri3 = ep.iri.clone();
                                let label = ep.label.clone();
                                let (badge_label, badge_color) = source_badge(&ep.source);
                                let source_text = ep.source.clone().unwrap_or_else(|| "unknown".into());
                                let comment = ep.comment.clone().unwrap_or_default();
                                let badge_style = format!("border-color: {badge_color}; color: {badge_color}");

                                view! {
                                    <div class="timeline-entry">
                                        <div
                                            class="timeline-entry-header"
                                            on:click=move |_| {
                                                let current = expanded_ep.get();
                                                if current.as_deref() == Some(&iri) {
                                                    set_expanded_ep.set(None);
                                                    set_expanded_data.set(serde_json::Value::Null);
                                                } else {
                                                    set_expanded_ep.set(Some(iri.clone()));
                                                    let iri_inner = iri.clone();
                                                    spawn_local(async move {
                                                        match api::fetch_episode_entities(&iri_inner).await {
                                                            Ok(data) => set_expanded_data.set(data),
                                                            Err(e) => log::error!("Failed to load episode entities: {e}"),
                                                        }
                                                    });
                                                }
                                            }
                                        >
                                            <span class="timeline-expand-icon">
                                                {move || if expanded_ep.get().as_deref() == Some(&iri2) { "v" } else { ">" }}
                                            </span>
                                            <span class="timeline-label">{label.clone()}</span>
                                            <span class="timeline-badge" style=badge_style.clone()>
                                                {badge_label}
                                            </span>
                                            <span class="timeline-source">{source_text.clone()}</span>
                                        </div>
                                        {if !comment.is_empty() {
                                            view! { <div class="timeline-comment">{comment.clone()}</div> }.into_any()
                                        } else {
                                            view! { <div></div> }.into_any()
                                        }}
                                        {move || {
                                            if expanded_ep.get().as_deref() != Some(&iri3) {
                                                return view! { <div></div> }.into_any();
                                            }
                                            let data = expanded_data.get();
                                            let rows = data.get("rows")
                                                .and_then(|r| r.as_array())
                                                .cloned()
                                                .unwrap_or_default();

                                            if rows.is_empty() {
                                                return view! {
                                                    <div class="timeline-entities">
                                                        <div class="timeline-empty">"Loading entities..."</div>
                                                    </div>
                                                }.into_any();
                                            }

                                            // Group by subject
                                            let mut subjects = std::collections::BTreeMap::<String, Vec<(String, String)>>::new();
                                            for row in &rows {
                                                let s = extract_val(row, "s").unwrap_or_default();
                                                let p = extract_val(row, "p").unwrap_or_default();
                                                let o = extract_val(row, "o").unwrap_or_default();
                                                subjects.entry(api::short_name(&s))
                                                    .or_default()
                                                    .push((api::short_name(&p), display_value(&o)));
                                            }

                                            view! {
                                                <div class="timeline-entities">
                                                    {subjects.into_iter().map(|(subj, facts)| {
                                                        view! {
                                                            <div class="timeline-entity">
                                                                <div class="timeline-entity-name">{subj}</div>
                                                                {facts.iter().map(|(p, v)| {
                                                                    view! {
                                                                        <div class="timeline-entity-fact">
                                                                            <span class="te-pred">{p.clone()}</span>
                                                                            <span class="te-val">{v.clone()}</span>
                                                                        </div>
                                                                    }
                                                                }).collect::<Vec<_>>()}
                                                            </div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }}
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

fn extract_val(row: &serde_json::Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else if let Some(val) = v.get("value").and_then(|v| v.as_str()) {
                Some(val.to_string())
            } else {
                None
            }
        })
}

fn display_value(val: &str) -> String {
    if api::is_iri(val) {
        api::short_name(val)
    } else if val.len() > 50 {
        format!("{}...", &val[..47])
    } else {
        val.to_string()
    }
}

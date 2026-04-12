//! Temporal navigator: dual-axis time controls, entity history, and graph diff.
//!
//! Integrates into the graph explorer as an overlay panel at the bottom of the
//! graph area. Controls valid-time and transaction-time sliders that re-query
//! the graph in real-time.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api;
use crate::interop;

/// History entry from the API.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct HistoryEntry {
    pub op: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub tx: i64,
}

/// Transaction metadata.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TxInfo {
    pub id: i64,
    pub timestamp: String,
    #[allow(dead_code)]
    pub actor: Option<String>,
    #[allow(dead_code)]
    pub source: Option<String>,
}

/// Temporal controls panel — renders at the bottom of the graph area.
#[component]
pub fn TemporalControls(
    #[prop(into)] on_temporal_change: Callback<(Option<String>, Option<i64>)>,
) -> impl IntoView {
    let (valid_time, set_valid_time) = signal(String::new());
    let (tx_value, set_tx_value) = signal(0i64);
    let (max_tx, set_max_tx) = signal(1i64);
    let (transactions, set_transactions) = signal(Vec::<TxInfo>::new());
    let (playing, set_playing) = signal(false);
    let (temporal_enabled, set_temporal_enabled) = signal(false);

    // Fetch transaction range on mount
    Effect::new(move || {
        spawn_local(async move {
            if let Ok(result) = api::fetch_transactions().await {
                if let Some(txns) = result.get("transactions").and_then(|t| t.as_array()) {
                    let parsed: Vec<TxInfo> = txns
                        .iter()
                        .filter_map(|t| serde_json::from_value(t.clone()).ok())
                        .collect();
                    if let Some(last) = parsed.last() {
                        set_max_tx.set(last.id);
                        set_tx_value.set(last.id);
                    }
                    set_transactions.set(parsed);
                }
            }
        });
    });

    // Apply temporal context when sliders change
    let apply_temporal = move || {
        if !temporal_enabled.get() {
            on_temporal_change.run((None, None));
            return;
        }
        let vt = {
            let v = valid_time.get();
            if v.is_empty() { None } else { Some(v) }
        };
        let tx = {
            let t = tx_value.get();
            let m = max_tx.get();
            if t >= m { None } else { Some(t) }
        };
        on_temporal_change.run((vt, tx));
    };

    // Playback timer
    let playback_handle: StoredValue<Option<i32>> = StoredValue::new(None);

    let toggle_playback = move |_| {
        if playing.get() {
            // Stop
            set_playing.set(false);
            if let Some(handle) = playback_handle.get_value() {
                let window = web_sys::window().unwrap();
                window.clear_interval_with_handle(handle);
            }
            playback_handle.set_value(None);
        } else {
            // Start
            set_playing.set(true);
            set_temporal_enabled.set(true);
            let cb = Closure::<dyn Fn()>::new(move || {
                let current = tx_value.get();
                let m = max_tx.get();
                if current >= m {
                    set_tx_value.set(1);
                } else {
                    set_tx_value.set(current + 1);
                }
                apply_temporal();
            });
            let window = web_sys::window().unwrap();
            let handle = window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    1000,
                )
                .unwrap_or(0);
            playback_handle.set_value(Some(handle));
            cb.forget();
        }
    };

    view! {
        <div class="temporal-controls">
            <div class="temporal-header">
                <label class="temporal-toggle">
                    <input
                        type="checkbox"
                        prop:checked=move || temporal_enabled.get()
                        on:change=move |ev| {
                            use wasm_bindgen::JsCast;
                            let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                            set_temporal_enabled.set(target.checked());
                            apply_temporal();
                        }
                    />
                    "Time Travel"
                </label>
                <button
                    class=move || if playing.get() { "btn btn-play active" } else { "btn btn-play" }
                    on:click=toggle_playback
                    disabled=move || !temporal_enabled.get()
                >
                    {move || if playing.get() { "Stop" } else { "Play" }}
                </button>
                {move || {
                    if temporal_enabled.get() {
                        let txs = transactions.get();
                        let current_tx = tx_value.get();
                        let tx_info = txs.iter().find(|t| t.id == current_tx);
                        let ts = tx_info.map(|t| t.timestamp.clone()).unwrap_or_default();
                        view! { <span class="temporal-status">{format!("tx:{current_tx} {ts}")}</span> }.into_any()
                    } else {
                        view! { <span class="temporal-status">"current"</span> }.into_any()
                    }
                }}
            </div>

            {move || {
                if !temporal_enabled.get() {
                    return view! { <div class="temporal-sliders hidden"></div> }.into_any();
                }
                let m = max_tx.get();
                view! {
                    <div class="temporal-sliders">
                        <div class="slider-row">
                            <label class="slider-label">"TX time"</label>
                            <input
                                type="range"
                                min="1"
                                max=move || max_tx.get().to_string()
                                prop:value=move || tx_value.get().to_string()
                                class="time-slider"
                                on:input=move |ev| {
                                    use wasm_bindgen::JsCast;
                                    let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                                    if let Ok(v) = target.value().parse::<i64>() {
                                        set_tx_value.set(v);
                                    }
                                }
                                on:change=move |_| { apply_temporal(); }
                            />
                            <span class="slider-value">{move || format!("{}/{m}", tx_value.get())}</span>
                        </div>
                        <div class="slider-row">
                            <label class="slider-label">"Valid at"</label>
                            <input
                                type="date"
                                prop:value=move || valid_time.get()
                                class="time-date"
                                on:change=move |ev| {
                                    use wasm_bindgen::JsCast;
                                    let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                                    set_valid_time.set(target.value());
                                    apply_temporal();
                                }
                            />
                            <button class="btn btn-sm" on:click=move |_| {
                                set_valid_time.set(String::new());
                                apply_temporal();
                            }>"Clear"</button>
                        </div>
                    </div>
                }.into_any()
            }}
        </div>
    }
}

/// Entity history panel — shows assertion/retraction timeline for a selected entity.
#[component]
pub fn EntityHistory(
    #[prop(into)] entity_iri: Signal<Option<String>>,
) -> impl IntoView {
    let (history, set_history) = signal(Vec::<HistoryEntry>::new());
    let (loading, set_loading) = signal(false);
    let (expanded, set_expanded) = signal(false);

    // Fetch history when entity changes
    Effect::new(move || {
        let iri = entity_iri.get();
        if let Some(iri) = iri {
            if !expanded.get() {
                return;
            }
            set_loading.set(true);
            spawn_local(async move {
                match api::fetch_entity_history(&iri).await {
                    Ok(result) => {
                        if let Some(entries) = result.get("history").and_then(|h| h.as_array()) {
                            let parsed: Vec<HistoryEntry> = entries
                                .iter()
                                .filter_map(|e| serde_json::from_value(e.clone()).ok())
                                .collect();
                            set_history.set(parsed);
                        }
                        set_loading.set(false);
                    }
                    Err(e) => {
                        log::error!("Failed to load history: {e}");
                        set_loading.set(false);
                    }
                }
            });
        } else {
            set_history.set(Vec::new());
        }
    });

    // Also refetch when expanded toggles on
    Effect::new(move || {
        if expanded.get() {
            if let Some(iri) = entity_iri.get() {
                set_loading.set(true);
                spawn_local(async move {
                    match api::fetch_entity_history(&iri).await {
                        Ok(result) => {
                            if let Some(entries) =
                                result.get("history").and_then(|h| h.as_array())
                            {
                                let parsed: Vec<HistoryEntry> = entries
                                    .iter()
                                    .filter_map(|e| serde_json::from_value(e.clone()).ok())
                                    .collect();
                                set_history.set(parsed);
                            }
                            set_loading.set(false);
                        }
                        Err(_) => set_loading.set(false),
                    }
                });
            }
        }
    });

    view! {
        <div class="entity-history">
            <div
                class="history-toggle"
                on:click=move |_| set_expanded.update(|v| *v = !*v)
            >
                <span class="toggle-icon">{move || if expanded.get() { "v" } else { ">" }}</span>
                " History"
                <span class="history-count">{move || {
                    let h = history.get();
                    if h.is_empty() { String::new() } else { format!("({})", h.len()) }
                }}</span>
            </div>
            {move || {
                if !expanded.get() {
                    return view! { <div class="history-entries hidden"></div> }.into_any();
                }
                if loading.get() {
                    return view! { <div class="history-entries"><div class="loading">"Loading..."</div></div> }.into_any();
                }
                let entries = history.get();
                if entries.is_empty() {
                    return view! { <div class="history-entries"><div class="history-empty">"No history"</div></div> }.into_any();
                }
                view! {
                    <div class="history-entries">
                        {entries.iter().map(|e| {
                            let op_class = if e.op == "assert" { "op-assert" } else { "op-retract" };
                            let op_symbol = if e.op == "assert" { "+" } else { "-" };
                            let pred = api::short_name(&e.predicate);
                            let val = format_history_value(&e.value);
                            let valid = format_valid_range(&e.valid_from, &e.valid_to);
                            view! {
                                <div class=format!("history-entry {op_class}")>
                                    <span class="history-op">{op_symbol}</span>
                                    <span class="history-pred">{pred}</span>
                                    <span class="history-val">{val}</span>
                                    <span class="history-time">{format!("tx:{} {}", e.tx, valid)}</span>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

/// Graph diff controls — compare graph at two time points.
#[component]
pub fn GraphDiffControls(
    #[prop(into)] on_diff: Callback<(i64, i64)>,
    #[prop(into)] on_clear_diff: Callback<()>,
    #[prop(into)] max_tx: Signal<i64>,
) -> impl IntoView {
    let (t1, set_t1) = signal(1i64);
    let (t2, set_t2) = signal(1i64);
    let (diff_active, set_diff_active) = signal(false);

    Effect::new(move || {
        let m = max_tx.get();
        set_t2.set(m);
        if m > 1 {
            set_t1.set(m - 1);
        }
    });

    view! {
        <div class="diff-controls">
            <div class="diff-header">
                <span class="diff-title">"Graph Diff"</span>
                {move || {
                    if diff_active.get() {
                        view! {
                            <button class="btn btn-sm" on:click=move |_| {
                                set_diff_active.set(false);
                                on_clear_diff.run(());
                            }>"Clear"</button>
                        }.into_any()
                    } else {
                        view! { <span></span> }.into_any()
                    }
                }}
            </div>
            <div class="diff-inputs">
                <label>"t1:"</label>
                <input
                    type="number"
                    min="1"
                    max=move || max_tx.get().to_string()
                    prop:value=move || t1.get().to_string()
                    class="diff-input"
                    on:input=move |ev| {
                        use wasm_bindgen::JsCast;
                        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                        if let Ok(v) = target.value().parse::<i64>() {
                            set_t1.set(v);
                        }
                    }
                />
                <label>"t2:"</label>
                <input
                    type="number"
                    min="1"
                    max=move || max_tx.get().to_string()
                    prop:value=move || t2.get().to_string()
                    class="diff-input"
                    on:input=move |ev| {
                        use wasm_bindgen::JsCast;
                        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                        if let Ok(v) = target.value().parse::<i64>() {
                            set_t2.set(v);
                        }
                    }
                />
                <button class="btn btn-primary btn-sm" on:click=move |_| {
                    set_diff_active.set(true);
                    on_diff.run((t1.get(), t2.get()));
                }>"Diff"</button>
            </div>
            {move || {
                if diff_active.get() {
                    view! {
                        <div class="diff-legend">
                            <span class="legend-item added">"+ added"</span>
                            <span class="legend-item removed">"- removed"</span>
                            <span class="legend-item changed">"~ changed"</span>
                        </div>
                    }.into_any()
                } else {
                    view! { <div></div> }.into_any()
                }
            }}
        </div>
    }
}

fn format_history_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => {
            if api::is_iri(s) {
                api::short_name(s)
            } else if s.len() > 40 {
                format!("{}...", &s[..37])
            } else {
                s.clone()
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(v) = obj.get("value").and_then(|v| v.as_str()) {
                if v.len() > 40 {
                    format!("{}...", &v[..37])
                } else {
                    v.to_string()
                }
            } else {
                format!("{val}")
            }
        }
        other => format!("{other}"),
    }
}

fn format_valid_range(from: &str, to: &Option<String>) -> String {
    let short_from = if from.len() > 10 { &from[..10] } else { from };
    match to {
        Some(t) => {
            let short_to = if t.len() > 10 { &t[..10] } else { t };
            format!("{short_from}..{short_to}")
        }
        None => format!("{short_from}..now"),
    }
}

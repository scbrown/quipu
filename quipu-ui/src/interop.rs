//! wasm-bindgen bridge to Sigma.js + Graphology.
//!
//! Rust owns the graph state as Leptos signals and pushes changes to the
//! JS side via these FFI calls. JS events (click, hover) call back into
//! Rust via exported functions registered at init time.

use wasm_bindgen::prelude::*;

// ── JS function bindings (call INTO JavaScript) ──────────────────────

#[wasm_bindgen(inline_js = r#"
export function js_init_graph(containerId) {
    return window.__quipu.initGraph(containerId);
}

export function js_add_node(id, label, nodeType, x, y) {
    window.__quipu.addNode(id, label, nodeType, x, y);
}

export function js_add_edge(source, target, label) {
    window.__quipu.addEdge(source, target, label);
}

export function js_clear_graph() {
    window.__quipu.clear();
}

export function js_start_layout() {
    window.__quipu.startLayout();
}

export function js_highlight_node(nodeId) {
    window.__quipu.highlightNode(nodeId);
}

export function js_focus_node(nodeId) {
    window.__quipu.focusNode(nodeId);
}

export function js_set_node_visibility(nodeId, visible) {
    window.__quipu.setNodeVisibility(nodeId, visible);
}

export function js_get_node_ids() {
    return window.__quipu.getNodeIds();
}

export function js_get_node_attrs(nodeId) {
    return window.__quipu.getNodeAttrs(nodeId);
}

export function js_get_neighbors(nodeId) {
    return window.__quipu.getNeighbors(nodeId);
}

export function js_get_graph_stats() {
    return window.__quipu.stats();
}

export function js_register_callbacks(onNodeClick, onNodeDblClick, onStageClick) {
    window.__quipu_on_node_click = onNodeClick;
    window.__quipu_on_node_dblclick = onNodeDblClick;
    window.__quipu_on_stage_click = onStageClick;
}
"#)]
extern "C" {
    pub fn js_init_graph(container_id: &str) -> bool;
    pub fn js_add_node(id: &str, label: &str, node_type: &str, x: f64, y: f64);
    pub fn js_add_edge(source: &str, target: &str, label: &str);
    pub fn js_clear_graph();
    pub fn js_start_layout();
    pub fn js_highlight_node(node_id: &str);
    pub fn js_focus_node(node_id: &str);
    pub fn js_set_node_visibility(node_id: &str, visible: bool);
    pub fn js_get_node_ids() -> String;
    pub fn js_get_node_attrs(node_id: &str) -> String;
    pub fn js_get_neighbors(node_id: &str) -> String;
    pub fn js_get_graph_stats() -> String;
    pub fn js_register_callbacks(
        on_node_click: &Closure<dyn Fn(String)>,
        on_node_dblclick: &Closure<dyn Fn(String)>,
        on_stage_click: &Closure<dyn Fn()>,
    );
}

/// Graph statistics from the JS side.
#[derive(serde::Deserialize, Default, Clone, Debug)]
pub struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
}

/// Fetch graph stats from the JS Graphology instance.
pub fn get_graph_stats() -> GraphStats {
    let json = js_get_graph_stats();
    serde_json::from_str(&json).unwrap_or_default()
}

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

// ── CodeMirror editor bridge ─────────────────────────────────────

#[wasm_bindgen(inline_js = r#"
export function js_editor_init(containerId, initialValue, onRunCb) {
    window.__quipu_editor.init(containerId, initialValue, onRunCb);
}

export function js_editor_get_value() {
    return window.__quipu_editor.getValue();
}

export function js_editor_set_value(text) {
    window.__quipu_editor.setValue(text);
}

export function js_editor_set_completion_data(typesJson, predicatesJson) {
    window.__quipu_editor.setCompletionData(typesJson, predicatesJson);
}

export function js_editor_focus() {
    window.__quipu_editor.focus();
}
"#)]
extern "C" {
    pub fn js_editor_init(container_id: &str, initial_value: &str, on_run: &Closure<dyn Fn()>);
    pub fn js_editor_get_value() -> String;
    pub fn js_editor_set_value(text: &str);
    pub fn js_editor_set_completion_data(types_json: &str, predicates_json: &str);
    pub fn js_editor_focus();
}

// ── Schema graph bridge (separate Sigma.js for ontology view) ────

#[wasm_bindgen(inline_js = r#"
export function js_schema_init(containerId) {
    return window.__quipu_schema.initGraph(containerId);
}

export function js_schema_add_class(id, label, count) {
    window.__quipu_schema.addClassNode(id, label, count);
}

export function js_schema_add_datatype(id, label) {
    window.__quipu_schema.addDatatypeNode(id, label);
}

export function js_schema_add_property(source, target, label) {
    window.__quipu_schema.addPropertyEdge(source, target, label);
}

export function js_schema_add_subclass(child, parent) {
    window.__quipu_schema.addSubclassEdge(child, parent);
}

export function js_schema_layout() {
    window.__quipu_schema.runLayout();
}

export function js_schema_clear() {
    window.__quipu_schema.clear();
}

export function js_schema_register_click(onNodeClick) {
    window.__quipu_schema_on_node_click = onNodeClick;
}
"#)]
extern "C" {
    pub fn js_schema_init(container_id: &str) -> bool;
    pub fn js_schema_add_class(id: &str, label: &str, count: u32);
    pub fn js_schema_add_datatype(id: &str, label: &str);
    pub fn js_schema_add_property(source: &str, target: &str, label: &str);
    pub fn js_schema_add_subclass(child: &str, parent: &str);
    pub fn js_schema_layout();
    pub fn js_schema_clear();
    pub fn js_schema_register_click(on_node_click: &Closure<dyn Fn(String)>);
}


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

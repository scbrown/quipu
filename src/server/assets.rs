//! UI assets compiled into the server binary.

pub(crate) const UI_HTML: &str = include_str!("../../ui/index.html");
pub(crate) const COMPONENTS_JS: &str = include_str!("../../ui/quipu-components.js");
pub(crate) const GRAPH_CANVAS_JS: &str = include_str!("../../ui/graph-canvas.js");
pub(crate) const DATALINKS_JS: &str = include_str!("../../ui/datalinks.js");
// Vendored, not fetched: the UI must render on an air-gapped deploy.
pub(crate) const THREE_JS: &str = include_str!("../../ui/vendor/three.module.min.js");

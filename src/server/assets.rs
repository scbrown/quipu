//! Static UI assets embedded in the server binary.

use axum::{
    Router,
    response::{Html, IntoResponse},
    routing::get,
};

use super::SharedStore;

pub(crate) const UI_HTML: &str = include_str!("../../ui/index.html");
pub(crate) const COMPONENTS_JS: &str = include_str!("../../ui/quipu-components.js");
pub(crate) const GRAPH_CANVAS_JS: &str = include_str!("../../ui/graph-canvas.js");
pub(crate) const DATALINKS_JS: &str = include_str!("../../ui/datalinks.js");
// Vendored, not fetched: the UI must render on an air-gapped deploy.
pub(crate) const THREE_JS: &str = include_str!("../../ui/vendor/three.module.min.js");

/// The UI and its vendored assets. These handlers already lived here; the
/// routes did not, which made this the one server submodule whose caller had
/// to know its URL space.
pub(crate) fn routes() -> Router<SharedStore> {
    Router::new()
        .route("/", get(ui))
        .route("/ui", get(ui))
        .route("/quipu-components.js", get(components_js))
        .route("/graph-canvas.js", get(graph_canvas_js))
        .route("/datalinks.js", get(datalinks_js))
        .route("/vendor/three.module.min.js", get(three_js))
}

pub(crate) async fn ui() -> Html<&'static str> {
    Html(UI_HTML)
}

macro_rules! javascript_asset {
    ($name:ident, $body:ident) => {
        pub(crate) async fn $name() -> impl IntoResponse {
            (
                [(axum::http::header::CONTENT_TYPE, "application/javascript")],
                $body,
            )
        }
    };
}

javascript_asset!(components_js, COMPONENTS_JS);
javascript_asset!(graph_canvas_js, GRAPH_CANVAS_JS);
javascript_asset!(datalinks_js, DATALINKS_JS);
javascript_asset!(three_js, THREE_JS);

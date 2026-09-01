//! Static UI assets embedded in the server binary.

use axum::response::{Html, IntoResponse};

const UI_HTML: &str = include_str!("../../ui/index.html");
const COMPONENTS_JS: &str = include_str!("../../ui/quipu-components.js");
const GRAPH_CANVAS_JS: &str = include_str!("../../ui/graph-canvas.js");
const DATALINKS_JS: &str = include_str!("../../ui/datalinks.js");
// Vendored, not fetched: the UI must render on an air-gapped deploy.
const THREE_JS: &str = include_str!("../../ui/vendor/three.module.min.js");

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

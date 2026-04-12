//! Web component registration — loads quipu-components.js and registers
//! custom elements when the WASM module initializes.
//!
//! The actual custom element implementations live in `ui/quipu-components.js`
//! (pure JS for minimal payload). This module injects the script tag and
//! provides the postMessage bridge for Rust↔host communication.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function register_quipu_components() {
    // If components are already registered, skip.
    if (customElements.get("quipu-graph")) return true;

    // Inject the components script from the Quipu server.
    const script = document.createElement("script");
    const origin = window.location.origin;
    script.src = origin + "/quipu-components.js";
    script.async = true;
    document.head.appendChild(script);
    return true;
}

export function post_message_to_component(tagName, data) {
    window.postMessage({ target: tagName, ...JSON.parse(data) }, "*");
}
"#)]
extern "C" {
    /// Inject the quipu-components.js script to register all custom elements.
    pub fn register_quipu_components() -> bool;

    /// Send a postMessage to a specific web component by tag name.
    pub fn post_message_to_component(tag_name: &str, data: &str);
}

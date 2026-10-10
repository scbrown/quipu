//! Browser feature wiring beyond the query-only explorer.

use wasm_bindgen::prelude::*;

use super::{Explorer, err_js};

pub(super) fn engine_error(error: quipu::Error) -> JsValue {
    let message = error.to_string();
    let js_error = js_sys::Error::new(&message);
    if let quipu::Error::ValidationFailed {
        violations,
        messages,
    } = error
    {
        let report = serde_json::json!({ "violations": violations, "messages": messages });
        if let Ok(value) = js_sys::JSON::parse(&report.to_string()) {
            // Error.message preserves existing rejection text; the additive
            // validation property carries the native write gate's feedback.
            let _ = js_sys::Reflect::set(&js_error, &JsValue::from_str("validation"), &value);
        }
    }
    js_error.into()
}

pub(super) fn register_reasoners(store: &mut quipu::Store) -> Result<(), JsValue> {
    #[cfg(feature = "reactive-reasoner")]
    {
        let turtle = store
            .get_combined_shapes()
            .map_err(err_js)?
            .unwrap_or_default();
        let rules = quipu::reasoner::parse_rules(&turtle, None).map_err(err_js)?;
        store.add_observer(std::sync::Arc::new(quipu::ReactiveReasoner::new(rules)));
    }
    #[cfg(all(feature = "owl", feature = "reactive-reasoner"))]
    store.add_observer(std::sync::Arc::new(quipu::ReactiveOwl));
    Ok(())
}

/// Validate a locally declared remote label through the native label parser.
#[wasm_bindgen(js_name = remoteLabel)]
pub fn remote_label(endpoint: &str) -> Result<String, JsValue> {
    let endpoint: quipu::config::RemoteEndpoint = serde_json::from_str(endpoint).map_err(err_js)?;
    let label = endpoint.declared_label().map_err(engine_error)?;
    serde_json::to_string(&label.to_json()).map_err(err_js)
}

#[wasm_bindgen]
impl Explorer {
    /// Check a federation's local and remote labels before any network request.
    ///
    /// The supplied floor applies to this check only; existing query settings
    /// are restored on both success and refusal.
    #[wasm_bindgen(js_name = checkFederation)]
    pub fn check_federation(
        &mut self,
        sparql: &str,
        remotes: &str,
        floor: &str,
    ) -> Result<(), JsValue> {
        let remotes = serde_json::from_str(remotes).map_err(err_js)?;
        let federation = quipu::config::FederationConfig { remotes };
        let floor = serde_json::from_str(floor).map_err(err_js)?;
        let previous = std::mem::replace(self.store.labels_config_mut(), floor);
        let result = quipu::provider::check_federated_floor(&self.store, sparql, &federation);
        *self.store.labels_config_mut() = previous;
        result.map_err(engine_error)
    }

    /// Manage explicitly adopted OWL ontologies through the native tool API.
    ///
    /// Accepts the same load/list/remove/materialize JSON as `/ontology`.
    /// Pack loading does not silently adopt an additional OWL authority.
    #[cfg(feature = "owl")]
    pub fn ontology(&mut self, input: &str) -> Result<String, JsValue> {
        let input: serde_json::Value = serde_json::from_str(input).map_err(err_js)?;
        let result = quipu::tool_load_ontology(&mut self.store, &input).map_err(err_js)?;
        self.record("ontology", &result);
        serde_json::to_string(&result).map_err(err_js)
    }
}

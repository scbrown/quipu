use serde_json::{Value, json};

pub(super) fn defs() -> Vec<Value> {
    vec![json!({
        "name": "quipu_rule_query",
        "description": "Read explicit positive/negative support as supported/refuted/both/neither, plus a separate admitted judgment disposition. Current ROOT only; failed/incomplete reads are UNKNOWN, never absence. No action authority or implicit boolean conversion.",
        "inputSchema": {
            "type": "object", "additionalProperties": false,
            "properties": {
                "subject": {"type": "string"},
                "positive_predicate": {"type": "string"},
                "negative_predicate": {"type": "string"},
                "object": {"oneOf": [
                    {"type": "object", "properties": {"iri": {"type": "string"}}, "required": ["iri"], "additionalProperties": false},
                    {"type": "object", "properties": {"str": {"type": "string"}}, "required": ["str"], "additionalProperties": false}
                ]},
                "judgment_policy": {"type": "string"},
                "witness_limit": {"type": "integer", "minimum": 1, "maximum": 256, "default": 64}
            },
            "required": ["subject", "positive_predicate", "negative_predicate", "object", "judgment_policy"]
        }
    })]
}

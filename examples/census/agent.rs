//! The agent arm (extension, bead quipu-yr5): phase-2 recording driven by
//! an EXTERNAL writer instead of the scripted drivers.
//!
//! The recording is a JSONL file of write actions. The harness replays
//! them through the same gated store and reports each action's outcome
//! with the gate's structured feedback — the loop an agent lives in:
//! write, read the refusal, revise, resubmit. Scoring against camayoc's
//! competency suites is future work (the suites have no runner yet);
//! what this arm measures today is acceptance/refusal per attempt and
//! defects present in the final graph.
//!
//! Action schema (one JSON object per line):
//! `{"writer": "amaru", "graph": "urn:census:graph:district-north",
//!   "subject": "urn:census:...", "type": "urn:census:Tally",
//!   "props": {"http://...label": "x", "urn:census:vocab:...": "y"}}`
//! `type` is optional; `props` values are strings.

use quipu::{Datum, Op, Value};
use serde::Deserialize;

use crate::phases::Ctx;

#[derive(Deserialize)]
struct Action {
    writer: String,
    graph: String,
    subject: String,
    #[serde(rename = "type")]
    type_iri: Option<String>,
    #[serde(default)]
    props: std::collections::BTreeMap<String, String>,
}

/// Replay a recording through the gate. Returns (accepted, refused).
pub fn replay_recording(ctx: &mut Ctx, path: &str) -> (usize, usize) {
    let text = std::fs::read_to_string(path).expect("recording readable");
    let (mut accepted, mut refused) = (0usize, 0usize);
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let action: Action = serde_json::from_str(line).expect("recording line parses");
        let ts = ctx.tick();
        let g = ctx
            .store
            .lookup(&action.graph)
            .ok()
            .flatten()
            .expect("recording targets a founded graph");
        let mut datums = Vec::new();
        if let Some(t) = &action.type_iri {
            let type_id = ctx.store.intern(t).expect("intern type");
            datums.push(Datum {
                entity: ctx.store.intern(&action.subject).expect("intern subject"),
                attribute: ctx
                    .store
                    .intern(quipu::namespace::RDF_TYPE)
                    .expect("intern rdf:type"),
                value: Value::Ref(type_id),
                valid_from: ts.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        for (p, v) in &action.props {
            datums.push(Datum {
                entity: ctx.store.intern(&action.subject).expect("intern subject"),
                attribute: ctx.store.intern(p).expect("intern predicate"),
                value: Value::Str(v.clone()),
                valid_from: ts.clone(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        ctx.store.set_principal_chain(vec![action.writer.clone()]);
        let r = ctx.store.transact_to_graph(
            &datums,
            &ts,
            Some(&action.writer),
            Some("census:agent"),
            g,
        );
        let (observed, ok) = match &r {
            Ok(tx) => (format!("accepted: tx {tx}"), true),
            Err(e) => (format!("refused: {e}"), false),
        };
        if ok {
            accepted += 1;
        } else {
            refused += 1;
        }
        ctx.probe(
            &format!("CEN-AG.{}", i + 1),
            2,
            &format!(
                "agent action {} by {}: {}",
                i + 1,
                action.writer,
                action.subject
            ),
            &observed,
            "RQ2-agent",
        );
    }
    (accepted, refused)
}

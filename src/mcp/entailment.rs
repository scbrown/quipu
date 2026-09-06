//! Closure freshness for the entailment marker (aegis-ab4m51).
//!
//! Split out of `mod.rs` rather than parked in the size-ratchet baseline: the
//! baseline grandfathers files that are already too big and exists to shrink,
//! so adding a new entry to it to accommodate new code inverts its purpose.
//! These three items are one unit — a per-graph lag, how it is derived, and how
//! it is rendered — so the split follows a seam that was already there.

use serde_json::Value as JsonValue;

use crate::store::Store;

/// One composed graph's closure lag: how far its companion trails its premises.
pub(super) struct GraphLag {
    pub(super) graph: String,
    pub(super) base_newest: Option<String>,
    pub(super) companion_newest: Option<String>,
    /// Seconds the closure trails the base. `None` is UNKNOWN, never zero.
    pub(super) lag_seconds: Option<i64>,
}

/// Per-graph closure freshness for the entailment marker (aegis-ab4m51).
///
/// Per-graph rather than two store-wide maxima, and that is not a refinement —
/// the scalar version is WRONG. Difference-of-maxima is not maximum-of-
/// differences: with graph A at base 10:00 / companion 10:00 and graph B at
/// base 09:00 / companion 08:00, both maxima are 10:00, so the marker reports
/// no lag while B is an hour stale. A fresh companion masks a stale one,
/// because a maximum hides its minimum — and the marker would then say
/// "nothing to worry about" on exactly the composition it exists to warn
/// about. Separating FRESH from STALE wrongly is worse than the prose note it
/// replaces (malcolm, ruling on aegis-ab4m51).
///
/// `composed` is the flat post-composition scope, so the base/companion
/// pairing has to be recovered. A graph is a BASE graph iff its companion IRI
/// resolves AND that companion is itself in scope; a companion's own companion
/// does not exist, so companions drop out without needing to be recognised.
pub(super) fn entailment_freshness(store: &Store, composed: &[i64]) -> Vec<GraphLag> {
    let mut lags = Vec::new();
    for &g in composed {
        let Ok(iri) = store.companion_inferred_iri(g) else {
            continue;
        };
        let Ok(Some(companion)) = store.lookup(&iri) else {
            continue;
        };
        if !composed.contains(&companion) {
            continue;
        }
        let base = store.graph_newest_valid_from(g).ok().flatten();
        let comp = store.graph_newest_valid_from(companion).ok().flatten();
        // A base graph holding no facts cannot be stale; anything else whose
        // seconds we cannot read is UNKNOWN, which must not render as fresh.
        let lag_seconds = match (&base, &comp) {
            (None, _) => Some(0),
            (Some((_, Some(b))), Some((_, Some(c)))) => Some((b - c).max(0)),
            _ => None,
        };
        lags.push(GraphLag {
            graph: store.resolve(g).unwrap_or_else(|_| format!("graph:{g}")),
            base_newest: base.map(|(t, _)| t),
            companion_newest: comp.map(|(t, _)| t),
            lag_seconds,
        });
    }
    lags
}

/// Announce the entailment regime the answer was computed under.
///
/// A SEPARATE key from `inference`, not an extra field on it: `inference`
/// reports that a constant `rdf:type` pattern was expanded over subclasses,
/// which is a different mechanism with a different remedy. Folding both into
/// one `applied: true` would leave a reader unable to tell which one widened
/// their answer.
///
/// Present only when a regime was requested, so — as with `inference` — the
/// field's PRESENCE is the signal.
///
/// `worstLagSeconds` (aegis-ab4m51) is the marker's verdict: the largest lag
/// over the composed graphs, so it cannot read fresh while any graph is stale.
/// `null` means UNKNOWN — at least one graph's lag could not be computed — and
/// is deliberately distinct from `0`. This is NOT a refusal, per wu: refusing a
/// stale closure would fail `--entailment` on any graph written since
/// materialisation, which is most of them, and the response would be to stop
/// using the flag. A guard that gets disabled protects nothing. Report the
/// fact; let the caller judge.
///
/// The per-graph `graphs` list is emitted only when there is something to
/// explain — a caller who sees a lag needs the culprit, and `composedGraphs` is
/// an integer that cannot name it. In the healthy case the list is silence.
pub(super) fn add_entailment(
    out: &mut JsonValue,
    regime: Option<&str>,
    graphs: usize,
    lags: &[GraphLag],
) {
    let Some(regime) = regime else {
        return;
    };
    let worst = if lags.iter().any(|l| l.lag_seconds.is_none()) {
        None
    } else {
        lags.iter().filter_map(|l| l.lag_seconds).max().or(Some(0))
    };
    if let Some(obj) = out.as_object_mut() {
        let mut marker = serde_json::json!({
            "regime": regime,
            "composedGraphs": graphs,
            "worstLagSeconds": worst,
            "note": "answered over the RDF merge of the requested graph(s) and their companion inferred graph(s); the closure is materialised out of band. worstLagSeconds is how far the stalest companion trails its premises: 0 is up to date, null means at least one graph could not be measured and must not be read as fresh",
        });
        if worst != Some(0)
            && let Some(m) = marker.as_object_mut()
        {
            m.insert(
                "graphs".to_string(),
                lags.iter()
                    .map(|l| {
                        serde_json::json!({
                            "graph": l.graph,
                            "baseNewest": l.base_newest,
                            "companionNewest": l.companion_newest,
                            "lagSeconds": l.lag_seconds,
                        })
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        obj.insert("entailment".to_string(), marker);
    }
}

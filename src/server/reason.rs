//! `POST /reason` — on-demand Datalog evaluation over REST — and the
//! `POST /shapes` handler that keeps the reactive ruleset live.
//!
//! Both close gap G6 of `docs/design/semantic-reasoning-gaps.md` (quipu-923):
//! reasoning was CLI + MCP + library only, and the reactive reasoner's ruleset
//! was a snapshot taken at server startup, so rules loaded through `/shapes`
//! needed a restart to take effect — a wart recorded in
//! `shapes/aegis-class-subsumption.rules.ttl` as having bitten that
//! workstream five times.

use axum::extract::State;
use serde_json::{Value as JsonValue, json};

use super::SharedStore;
use super::base::{AppError, blocking};

/// `POST /shapes` — load/remove shapes, then refresh the reactive ruleset.
///
/// The body/behaviour of the shapes tool is unchanged (`quipu::tool_shapes`,
/// formerly registered through `rw_handler!`); this hand-written version adds
/// one step: after a successful write it re-reads the combined shapes and
/// swaps them into the registered [`quipu::ReactiveReasoner`], so a rule
/// loaded here derives on the very next write, no restart.
pub(crate) async fn shapes(
    State(s): State<SharedStore>,
    axum::Json(i): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let (out, work) = {
            let mut st = s.lock();
            // `&st`, not `&mut st`: `tool_shapes` writes through `&self`
            // (interior mutability over the SQLite connection) — see the
            // ro_handler! warning in tools.rs about what `&Store` does NOT
            // prove. The write-endpoint classification lives in http_auth.
            let out = quipu::tool_shapes(&st, &i)?;
            (out, st.take_deferred_embed())
        };
        if let Some(work) = work {
            super::tools::finish_deferred_embed(&s, &work)?;
        }

        // Refresh the live ruleset. A parse failure must not fail the request
        // — the shapes themselves committed — but it must not pass unremarked
        // either: a reasoner silently running yesterday's rules is the exact
        // inertness this route exists to end.
        #[cfg(feature = "reactive-reasoner")]
        if let Some(reasoner) = &s.reasoner {
            let combined = { s.lock().get_combined_shapes() };
            match combined {
                Ok(Some(ttl)) => match quipu::reasoner::parse_rules(&ttl, None) {
                    Ok(ruleset) => {
                        let n = ruleset.len();
                        reasoner.reload(ruleset);
                        eprintln!("reactive reasoner reloaded — {n} Datalog rule(s) live");
                    }
                    Err(e) => {
                        eprintln!("reactive reasoner NOT reloaded — rules failed to parse: {e}");
                    }
                },
                Ok(None) => {
                    reasoner.reload(quipu::reasoner::RuleSet::empty(
                        quipu::namespace::DEFAULT_BASE_NS,
                    ));
                    eprintln!("reactive reasoner reloaded — no shapes stored, ruleset now empty");
                }
                Err(e) => {
                    eprintln!("reactive reasoner NOT reloaded — could not read shapes: {e}");
                }
            }
        }

        Ok(axum::Json(out))
    })
    .await
}

/// `POST /reason` — run a Datalog ruleset to fixpoint and persist derivations.
///
/// Body:
/// - `rules` (optional): inline rule Turtle. Absent, the stored combined
///   shapes are used — the same source the reactive reasoner reads.
/// - `prefix` (optional): default IRI prefix for unqualified predicate names.
/// - `graph` (optional): a named graph IRI; premises and derivations are both
///   scoped to it (`evaluate_in_graph`). Absent means ROOT.
/// - `timestamp` (optional): valid-from for the derived facts; defaults to now.
///
/// Response reports rules run, strata, asserted/retracted counts, and the
/// per-rule delta — the same numbers `quipu reason` prints.
pub(crate) async fn reason(
    State(s): State<SharedStore>,
    axum::Json(i): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let mut st = s.lock();
        let ttl = match i.get("rules").and_then(JsonValue::as_str) {
            Some(r) => r.to_string(),
            None => st.get_combined_shapes()?.ok_or_else(|| {
                quipu::Error::InvalidValue(
                    "no rules: pass `rules` (Turtle) or load rules via POST /shapes first".into(),
                )
            })?,
        };
        let prefix = i.get("prefix").and_then(JsonValue::as_str);
        let ruleset = quipu::reasoner::parse_rules(&ttl, prefix)
            .map_err(|e| quipu::Error::InvalidValue(format!("rules failed to parse: {e}")))?;
        if ruleset.is_empty() {
            return Err(quipu::Error::InvalidValue(
                "no `a rule:Rule` subjects found in the rules".into(),
            )
            .into());
        }

        let timestamp = i
            .get("timestamp")
            .and_then(JsonValue::as_str)
            .map_or_else(quipu::time::now_iso, str::to_string);

        let report = match i.get("graph").and_then(JsonValue::as_str) {
            Some(graph_iri) => {
                let g = st.lookup(graph_iri)?.ok_or_else(|| {
                    quipu::Error::InvalidValue(format!("unknown graph <{graph_iri}>"))
                })?;
                quipu::reasoner::evaluate_in_graph(&mut st, &ruleset, &timestamp, g)
            }
            None => quipu::reasoner::evaluate(&mut st, &ruleset, &timestamp),
        }
        .map_err(|e| quipu::Error::Store(format!("reasoner error: {e}")))?;

        Ok(axum::Json(json!({
            "rules": ruleset.len(),
            "strata_run": report.strata_run,
            "asserted": report.asserted,
            "retracted": report.retracted,
            "per_rule": report
                .per_rule
                .iter()
                .map(|(id, n)| json!({"rule": id, "asserted": n}))
                .collect::<Vec<_>>(),
        })))
    })
    .await
}

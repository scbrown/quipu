//! MCP tools: `quipu_align_propose` / `_decide` / `_apply` (aegis-5qmg3r).
//!
//! # Why THREE tools and not one with a `mode`
//!
//! codex judges an MCP tool by its ANNOTATION, not by the arguments of a
//! particular call. A single `quipu_align` taking `mode=propose|decide|apply`
//! gets ONE annotation, and it would have to be `destructive` because the tool
//! CAN write. Every read-only call — including `propose`, which is the entry
//! point to the whole feature — would then hit codex's approval-never refusal
//! (aegis-n549ii, aegis-h3zyq0).
//!
//! We would ship an operator-driven alignment feature that the operator's own
//! agent cannot start. So the split is functional, not stylistic: `propose` and
//! `decide` are reads and annotate as such; `apply` is the only writer and
//! carries the destructive annotation alone.
//!
//! These same functions back the REST routes, so the two surfaces cannot
//! diverge — the defect measured on bobbin's `archive_recent`, where an MCP
//! tool was dark for every input while its HTTP twin answered, was a *constant
//! argument* into a shared function rather than a second implementation
//! (aegis-7xfyjr). Sharing is necessary and not sufficient; the tests below
//! assert the OUTPUT, not the shape.

use serde_json::{Value as JsonValue, json};

use crate::align::{
    apply::{self, set_version},
    decide::{self, Decision, DecisionRow},
    enumerate::propose_from_graphs,
    propose::LinkSpec,
    sssom::MappingSet,
};
use crate::error::{Error, Result};
use crate::store::Store;

fn required_str<'a>(input: &'a JsonValue, key: &str) -> Result<&'a str> {
    input
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue(format!("missing '{key}' parameter")))
}

/// READ. Propose candidate alignments between two named graphs.
///
/// # Errors
/// Missing parameters, an unknown graph IRI, or a store error.
pub fn tool_align_propose(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let graph_a = required_str(input, "graph_a")?;
    let graph_b = required_str(input, "graph_b")?;

    // REFUSE an unknown graph instead of proposing across it.
    //
    // `align::enumerate` looks the IRI up and returns an EMPTY enumeration when
    // it is not a known term, so a typo — or a namespace prefix passed where a
    // graph IRI belongs — comes back as "0 candidates". That is
    // indistinguishable from two graphs which genuinely share nothing, and it
    // reads as a clean answer. An empty result that cannot be told from a
    // correct one is the failure this surface must not hand an operator.
    for (key, iri) in [("graph_a", graph_a), ("graph_b", graph_b)] {
        if store.lookup(iri)?.is_none() {
            return Err(Error::InvalidValue(format!(
                "{key} '{iri}' is not a known graph in this store; refusing rather than \
                 returning 0 candidates, which is indistinguishable from two graphs with \
                 nothing in common"
            )));
        }
    }

    let set_id = input
        .get("mapping_set_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("urn:quipu:align:mcp");

    // propose_from_graphs, NOT propose: the latter takes a prepared enumeration.
    let proposal = propose_from_graphs(
        store,
        graph_a,
        graph_b,
        &LinkSpec::default(),
        &MappingSet::default(),
        set_id,
    )?;

    Ok(json!({
        "summary": proposal.summary(),
        "candidates": proposal.set.mappings.len(),
        "set_aside": proposal.set_aside.len(),
        // Returned at the point it is computed. `apply` must be GIVEN this
        // value; deriving one there would hash the set being written and always
        // match, silently voiding optimistic concurrency.
        "expected_version": set_version(&proposal.set)?,
        "set_tsv": proposal.set.to_tsv()?,
    }))
}

/// READ. Apply operator decisions to a proposed set. Touches no store.
///
/// # Errors
/// Missing parameters, an unparseable set or decision row.
pub fn tool_align_decide(input: &JsonValue) -> Result<JsonValue> {
    let set = MappingSet::from_tsv(required_str(input, "set_tsv")?)?;
    let reviewer = required_str(input, "reviewer")?;

    let rows = input
        .get("decisions")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| Error::InvalidValue("missing 'decisions' array".into()))?;

    let mut decisions = Vec::with_capacity(rows.len());
    for row in rows {
        let verdict = required_str(row, "decision")?;
        decisions.push(DecisionRow {
            subject_id: required_str(row, "subject_id")?.to_string(),
            object_id: required_str(row, "object_id")?.to_string(),
            decision: match verdict {
                "accept" => Decision::Accept,
                "negate" => Decision::Negate,
                other => {
                    return Err(Error::InvalidValue(format!(
                        "unknown decision '{other}' (expected 'accept' or 'negate')"
                    )));
                }
            },
        });
    }

    let report = decide::decide(&set, &decisions, reviewer)?;
    Ok(json!({
        "set_tsv": report.set.to_tsv()?,
        // Carried forward for `apply`, from the set the operator actually decided.
        "expected_version": set_version(&report.set)?,
    }))
}

/// WRITE — the only writer of the three, hence the destructive annotation.
///
/// # Errors
/// Missing parameters, a version mismatch, or a store error.
pub fn tool_align_apply(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let set = MappingSet::from_tsv(required_str(input, "set_tsv")?)?;
    let graph_a = required_str(input, "graph_a")?;
    let graph_b = required_str(input, "graph_b")?;

    // REQUIRED, never defaulted — see the module docs on set_version.
    let expected = required_str(input, "expected_version").map_err(|_| {
        Error::InvalidValue(
            "missing 'expected_version'. It is returned by align_propose and align_decide and \
             must be carried from the decision being applied: computing it here would hash the \
             set about to be written, always match, and silently discard a concurrent \
             operator's decision"
                .into(),
        )
    })?;

    let graph_iri = apply::derived_graph_iri(graph_a, graph_b);

    // Create the alignment graph if it does not exist yet.
    //
    // The IRI is DERIVED from the two source graphs — a hash pair, so that the
    // target depends on the inputs rather than on somebody typing the same
    // string twice. That is the right design, and it means the operator cannot
    // know the IRI in advance to `graph_create` it: `apply` would fail with
    // "not interned; create it with graph_create first" against a name they
    // were never given. Requiring a manual create of a hash-derived IRI is a
    // usability trap with no safety benefit, so the wiring closes it here.
    //
    // This is the only graph this tool will ever create, and it cannot be
    // pointed elsewhere: the IRI is a pure function of graph_a and graph_b.
    store.graph_create(&graph_iri)?;

    let timestamp = crate::time::now_iso();
    let report = apply::apply(
        store,
        &set,
        &graph_iri,
        expected,
        &timestamp,
        input.get("actor").and_then(JsonValue::as_str),
    )?;

    Ok(json!({
        "graph": graph_iri,
        "written": report.written,
        "distinct_from": report.distinct_from,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace::{RDF_TYPE, RDFS_LABEL};
    use crate::store::Datum;
    use crate::types::{Op, Value};

    const T: &str = "2026-09-05T00:00:00Z";

    fn label(store: &mut Store, graph_iri: &str, entity: &str, text: &str) {
        let e = store.intern(entity).unwrap();
        let a = store.intern(RDFS_LABEL).unwrap();
        let g = store.intern(graph_iri).unwrap();
        store
            .transact_to_graph(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Str(text.into()),
                    valid_from: T.into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                T,
                None,
                None,
                g,
            )
            .unwrap();
    }

    fn typed(store: &mut Store, graph_iri: &str, entity: &str, type_iri: &str) {
        let t = store.intern(type_iri).unwrap();
        let e = store.intern(entity).unwrap();
        let a = store.intern(RDF_TYPE).unwrap();
        let g = store.intern(graph_iri).unwrap();
        store
            .transact_to_graph(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Ref(t),
                    valid_from: T.into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                T,
                None,
                None,
                g,
            )
            .unwrap();
    }

    /// Two graphs naming the same concept, WITH a shared type.
    ///
    /// The type is not decoration: `LinkSpec::default()` sets
    /// `require_shared_type: true` deliberately, because two graphs that both
    /// call something "bobbin" while meaning a Repository and a Host are
    /// exactly the false positive a name match cannot catch. A fixture without
    /// types proposes nothing — which is the engine being right, and was my
    /// first draft being wrong.
    fn two_graphs() -> (Store, &'static str, &'static str) {
        let mut store = Store::open_in_memory().unwrap();
        let (a, b) = ("http://example.org/graph/a", "http://example.org/graph/b");
        let kind = "http://example.org/Release";
        label(&mut store, a, "http://a.example/bobbin", "bobbin release");
        typed(&mut store, a, "http://a.example/bobbin", kind);
        label(&mut store, b, "http://b.example/bobbin", "bobbin release");
        typed(&mut store, b, "http://b.example/bobbin", kind);
        (store, a, b)
    }

    /// The OUTPUT test, not a shape test: propose must return a real candidate
    /// against known-present data. `archive_recent` was dark for every input
    /// while sharing its twin's implementation (aegis-7xfyjr) — sharing is
    /// necessary and not sufficient, so this asserts what comes back.
    #[test]
    fn propose_returns_candidates_and_the_version_apply_will_need() {
        let (store, a, b) = two_graphs();
        let out = tool_align_propose(&store, &json!({ "graph_a": a, "graph_b": b })).unwrap();

        assert!(
            out["candidates"].as_u64().unwrap() > 0,
            "a matching pair across two graphs must propose at least one candidate; got {out}"
        );
        assert!(
            out["expected_version"].as_str().unwrap().starts_with("sha256:"),
            "propose must return the version apply requires, or the operator cannot proceed"
        );
        assert!(out["set_tsv"].as_str().unwrap().contains("subject_id"));
    }

    /// An unknown graph must REFUSE, not return zero.
    ///
    /// `align::enumerate` returns an empty enumeration for an IRI it cannot
    /// look up, so without this the answer is `0 candidates` — which is
    /// indistinguishable from two graphs that genuinely share nothing. Both
    /// arms here: the control proves a real graph proposes, so the refusal is
    /// not simply "everything fails".
    #[test]
    fn an_unknown_graph_is_refused_rather_than_reported_as_zero_candidates() {
        let (store, a, b) = two_graphs();

        let err = tool_align_propose(
            &store,
            &json!({ "graph_a": a, "graph_b": "http://b.example/" }),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("not a known graph"),
            "expected a refusal naming the unknown graph, got: {err}"
        );

        // CONTROL, from OUTSIDE the failing input: the same call with the real
        // graph succeeds, so the refusal above is about the graph and not about
        // the tool being broken.
        let ok = tool_align_propose(&store, &json!({ "graph_a": a, "graph_b": b })).unwrap();
        assert!(ok["candidates"].as_u64().unwrap() > 0);
    }

    /// `expected_version` is required, and the message says why.
    ///
    /// `set_version` hashes the set itself, so a surface that computed the
    /// version at apply time would always match and silently void optimistic
    /// concurrency — one of two concurrent operators would lose their decision
    /// with both seeing success.
    #[test]
    fn apply_refuses_without_an_expected_version() {
        let (mut store, a, b) = two_graphs();
        let proposal =
            tool_align_propose(&store, &json!({ "graph_a": a, "graph_b": b })).unwrap();

        let err = tool_align_apply(
            &mut store,
            &json!({
                "set_tsv": proposal["set_tsv"].as_str().unwrap(),
                "graph_a": a,
                "graph_b": b,
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("expected_version"),
            "the refusal must name the missing parameter, got: {err}"
        );

        // CONTROL: with the version carried from propose, apply proceeds.
        let applied = tool_align_apply(
            &mut store,
            &json!({
                "set_tsv": proposal["set_tsv"].as_str().unwrap(),
                "graph_a": a,
                "graph_b": b,
                "expected_version": proposal["expected_version"].as_str().unwrap(),
            }),
        );
        assert!(applied.is_ok(), "carrying the version must work: {applied:?}");
    }

    /// All three tools are registered, and `propose` is not annotated with the
    /// writer. A moded tool would have forced one destructive annotation onto
    /// the read path and codex would refuse `propose` — the entry point.
    #[test]
    fn all_three_align_tools_are_separately_registered() {
        let defs = crate::mcp::definitions::tool_definitions();
        let names: Vec<&str> = defs.iter().filter_map(|t| t["name"].as_str()).collect();
        for want in [
            "quipu_align_propose",
            "quipu_align_decide",
            "quipu_align_apply",
        ] {
            assert!(names.contains(&want), "{want} not registered");
        }
    }
}

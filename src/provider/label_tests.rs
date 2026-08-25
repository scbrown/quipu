//! quipu-fd1: the declared label at the federation edge.
//!
//! The property under test, stated once: a deployment with a `[quipu.labels]`
//! floor must refuse a federated result exactly as it refuses a local one —
//! `federated: true` must not be the way around a configured floor — and a
//! remote's label is DECLARED by the local operator, never read from the
//! remote.

use super::*;
use crate::config::RemoteEndpoint;
use crate::lattice::Trust;
use crate::provider::{FederatedProvider, GraphProvider, ProviderStatus};
use crate::sparql::QueryResult;
use crate::store::labels::GraphLabel;

const TS: &str = "2026-01-01T00:00:00Z";
const CHAIN: &str = "urn:chain:ours";

fn declared(name: &str, rank: i64) -> RemoteEndpoint {
    RemoteEndpoint {
        trust: Some(format!("urn:trust:{name}")),
        trust_chain: Some(CHAIN.into()),
        trust_rank: Some(rank),
        ..RemoteEndpoint::new(name, "http://127.0.0.1:1")
    }
}

fn floor(min_rank: i64) -> LabelsConfig {
    LabelsConfig {
        min_trust_rank: Some(min_rank),
        min_trust_chain: Some(CHAIN.into()),
        ..LabelsConfig::default()
    }
}

/// A store with one registered, trust-labelled graph `urn:g:local`.
fn store_with_local_trust(rank: i64) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:local", 0).unwrap();
    store
        .set_graph_label(
            "urn:g:local",
            &GraphLabel {
                trust: Some(Trust::new("urn:trust:local", CHAIN, rank)),
                ..GraphLabel::default()
            },
            TS,
            None,
        )
        .unwrap();
    store
}

const FROM_LOCAL: &str = "SELECT ?s FROM <urn:g:local> WHERE { ?s ?p ?o }";

// ---------------------------------------------------------------------------
// Declaring: the config is the only source of a remote's label
// ---------------------------------------------------------------------------

#[test]
fn a_full_trust_declaration_parses_into_the_lattice_type() {
    let l = declared("partner", 30).declared_label().unwrap();
    assert_eq!(l.trust, Some(Trust::new("urn:trust:partner", CHAIN, 30)));
    assert!(l.freshness.is_none(), "freshness stays undeclared");
}

#[test]
fn a_partial_trust_declaration_is_refused_not_dropped() {
    // A typo'd declaration that silently vanished would leave the operator
    // believing a label flows when none does.
    let mut r = declared("partner", 30);
    r.trust_chain = None;
    let err = r.declared_label().expect_err("partial declaration");
    let msg = err.to_string();
    assert!(
        msg.contains("partner") && msg.contains("trust_chain"),
        "{msg}"
    );
}

#[test]
fn an_unparseable_freshness_declaration_is_refused() {
    let r = RemoteEndpoint {
        freshness: Some("freshish".into()),
        ..RemoteEndpoint::new("p", "http://x:1")
    };
    let err = r.declared_label().expect_err("bad freshness");
    assert!(err.to_string().contains("freshish"), "{err}");
}

#[test]
fn an_undeclared_remote_reads_as_undeclared_never_fabricated() {
    let l = RemoteEndpoint::new("open", "http://x:1")
        .declared_label()
        .unwrap();
    assert!(l.is_empty());
    assert_eq!(l.to_string(), "undeclared");
}

// ---------------------------------------------------------------------------
// The floor at the edge: check_member_floor
// ---------------------------------------------------------------------------

#[test]
fn no_floor_means_no_behaviour_change_even_for_an_undeclared_remote() {
    let ok = check_member_floor(&LabelsConfig::default(), "open", &DeclaredLabel::default());
    assert!(ok.is_ok(), "unset floors are a no-op");
}

#[test]
fn an_under_trust_remote_is_refused_and_the_refusal_names_it() {
    let label = declared("partner", 10).declared_label().unwrap();
    let err = check_member_floor(&floor(30), "partner", &label).expect_err("below floor");
    let msg = err.to_string();
    assert!(
        msg.contains("partner") && msg.contains("urn:trust:partner") && msg.contains("rank 10"),
        "the refusal names the remote and its declared label: {msg}"
    );
}

#[test]
fn a_remote_declared_at_or_above_the_floor_passes() {
    let at = declared("at", 30).declared_label().unwrap();
    let above = declared("above", 40).declared_label().unwrap();
    assert!(check_member_floor(&floor(30), "at", &at).is_ok());
    assert!(check_member_floor(&floor(30), "above", &above).is_ok());
}

#[test]
fn an_undeclared_remote_fails_a_configured_trust_floor() {
    // The documented choice: undeclared composes like an unlabelled local
    // graph — a configured floor refuses it rather than reading silence as
    // trust (graph-labels.md §2.1, fail-safe at enforcement).
    let err = check_member_floor(&floor(30), "open", &DeclaredLabel::default())
        .expect_err("undeclared must not pass a floor");
    let msg = err.to_string();
    assert!(
        msg.contains("open") && msg.contains("declares no trust"),
        "{msg}"
    );
}

#[test]
fn a_remote_ranked_in_another_chain_cannot_be_evaluated() {
    let label = DeclaredLabel {
        trust: Some(Trust::new("urn:trust:x", "urn:chain:theirs", 90)),
        freshness: None,
    };
    let err = check_member_floor(&floor(30), "x", &label).expect_err("cross-chain");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:chain:theirs") && msg.contains(CHAIN),
        "{msg}"
    );
    assert!(
        !msg.contains("below the configured floor"),
        "rank 90 must NOT be compared to 30 across chains: {msg}"
    );
}

#[test]
fn the_freshness_floor_applies_at_the_edge_too() {
    let cfg = LabelsConfig {
        min_freshness: Some("fresh".into()),
        ..LabelsConfig::default()
    };
    let stale = DeclaredLabel {
        trust: None,
        freshness: Some(Freshness::Stale),
    };
    let err = check_member_floor(&cfg, "cold", &stale).expect_err("stale below fresh");
    assert!(err.to_string().contains("cold"), "{err}");
    let err = check_member_floor(&cfg, "open", &DeclaredLabel::default())
        .expect_err("undeclared freshness fails a freshness floor");
    assert!(err.to_string().contains("declares no freshness"), "{err}");
    let fresh = DeclaredLabel {
        trust: None,
        freshness: Some(Freshness::Fresh),
    };
    assert!(check_member_floor(&cfg, "warm", &fresh).is_ok());
}

// ---------------------------------------------------------------------------
// The whole federated read: check_federated_floor
// ---------------------------------------------------------------------------

#[test]
fn a_floor_configured_store_refuses_a_federation_with_an_under_trust_remote() {
    let mut store = store_with_local_trust(50);
    *store.labels_config_mut() = floor(30);
    let fed = FederationConfig {
        remotes: vec![declared("weak", 10)],
    };
    let err = check_federated_floor(&store, FROM_LOCAL, &fed).expect_err("under-trust remote");
    assert!(err.to_string().contains("weak"), "{err}");
}

#[test]
fn a_declared_remote_at_the_floor_federates() {
    let mut store = store_with_local_trust(50);
    *store.labels_config_mut() = floor(30);
    let fed = FederationConfig {
        remotes: vec![declared("strong", 40)],
    };
    assert!(check_federated_floor(&store, FROM_LOCAL, &fed).is_ok());
}

#[test]
fn an_undeclared_remote_refuses_under_a_floor_and_passes_without_one() {
    let mut store = store_with_local_trust(50);
    let fed = FederationConfig {
        remotes: vec![RemoteEndpoint::new("open", "http://127.0.0.1:1")],
    };
    assert!(
        check_federated_floor(&store, FROM_LOCAL, &fed).is_ok(),
        "no floor: undeclared remotes federate exactly as before"
    );
    *store.labels_config_mut() = floor(30);
    let err = check_federated_floor(&store, FROM_LOCAL, &fed)
        .expect_err("with a floor, undeclared fails");
    assert!(err.to_string().contains("open"), "{err}");
}

#[test]
fn the_local_members_are_checked_on_the_federated_path_too() {
    // The other half of the widening: before quipu-fd1 the LOCAL floor check
    // was simply not on the federated path at all.
    let mut store = store_with_local_trust(10);
    *store.labels_config_mut() = floor(30);
    let fed = FederationConfig::default();
    let err = check_federated_floor(&store, FROM_LOCAL, &fed).expect_err("local below floor");
    assert!(err.to_string().contains("urn:g:local"), "{err}");
}

// ---------------------------------------------------------------------------
// The composed label: remotes fold in as members, never widening
// ---------------------------------------------------------------------------

#[test]
fn a_remote_below_the_local_trust_drags_the_composed_label_down() {
    let store = store_with_local_trust(50);
    let fed = FederationConfig {
        remotes: vec![declared("weak", 10)],
    };
    let labels = federated_dataset_labels(&store, FROM_LOCAL, &fed)
        .unwrap()
        .expect("declared members");
    let t = labels.trust.value.expect("composed trust");
    assert_eq!(t.rank, 10, "meet: the least-trusted member decides");
    assert_eq!(t.iri, "urn:trust:weak");
    assert_eq!(labels.trust.coverage, Coverage::Full);
}

#[test]
fn an_undeclared_remote_degrades_coverage_but_fabricates_nothing() {
    let store = store_with_local_trust(50);
    let fed = FederationConfig {
        remotes: vec![RemoteEndpoint::new("open", "http://127.0.0.1:1")],
    };
    let labels = federated_dataset_labels(&store, FROM_LOCAL, &fed)
        .unwrap()
        .expect("the local member declared");
    assert_eq!(
        labels.trust.coverage,
        Coverage::Partial,
        "one declared member + one undeclared = partial"
    );
    assert_eq!(
        labels.trust.value.as_ref().map(|t| t.rank),
        Some(50),
        "the undeclared remote contributes no value — only coverage"
    );
}

#[test]
fn a_cross_chain_remote_refuses_the_fold_naming_both_chains() {
    let store = store_with_local_trust(50);
    let fed = FederationConfig {
        remotes: vec![RemoteEndpoint {
            trust: Some("urn:trust:x".into()),
            trust_chain: Some("urn:chain:theirs".into()),
            trust_rank: Some(90),
            ..RemoteEndpoint::new("x", "http://127.0.0.1:1")
        }],
    };
    let err = federated_dataset_labels(&store, FROM_LOCAL, &fed).expect_err("cross-chain");
    let msg = err.to_string();
    assert!(
        msg.contains(CHAIN) && msg.contains("urn:chain:theirs"),
        "{msg}"
    );
}

// ---------------------------------------------------------------------------
// The stamp: rows carry the declared label beside _provider
// ---------------------------------------------------------------------------

/// One fixed row, with an optionally declared label — the smallest member that
/// can prove the stamping rules.
struct StampedProvider {
    name: &'static str,
    label: Option<DeclaredLabel>,
}

impl GraphProvider for StampedProvider {
    fn name(&self) -> &str {
        self.name
    }
    fn query(&self, _sparql: &str) -> crate::error::Result<QueryResult> {
        let mut row = crate::sparql::Bindings::new();
        row.insert(
            "s".into(),
            crate::types::Value::Str(format!("{}-row", self.name)),
        );
        Ok(QueryResult::Select {
            variables: vec!["s".into()],
            rows: vec![row],
        })
    }
    fn entities(&self, _t: Option<&str>, _l: usize) -> crate::error::Result<JsonValue> {
        Ok(serde_json::json!({"entities": []}))
    }
    fn health(&self) -> ProviderStatus {
        ProviderStatus {
            name: self.name.into(),
            healthy: true,
            fact_count: None,
            message: None,
            label: self.label.clone(),
        }
    }
    fn declared_label(&self) -> Option<&DeclaredLabel> {
        self.label.as_ref()
    }
}

#[test]
fn rows_are_stamped_with_the_declared_label_and_undeclared_rows_are_not() {
    let mut fed = FederatedProvider::new();
    fed.add(Box::new(StampedProvider {
        name: "plain",
        label: None,
    }));
    fed.add(Box::new(StampedProvider {
        name: "labelled",
        label: Some(DeclaredLabel {
            trust: Some(Trust::new("urn:trust:partner", CHAIN, 30)),
            freshness: Some(Freshness::Fresh),
        }),
    }));

    let fq = fed.query_all("SELECT ?s WHERE { ?s ?p ?o }");
    assert!(fq.complete, "{:?}", fq.providers);
    let vars = fq.result.variables();
    assert!(vars.contains(&"_trust".to_string()), "{vars:?}");
    assert!(vars.contains(&"_freshness".to_string()), "{vars:?}");

    let rows = fq.result.rows();
    assert_eq!(rows.len(), 2);
    assert!(
        !rows[0].contains_key("_trust") && !rows[0].contains_key("_freshness"),
        "an undeclared member's rows carry NO fabricated stamp"
    );
    assert_eq!(
        rows[1].get("_trust"),
        Some(&crate::types::Value::Str("urn:trust:partner".into()))
    );
    assert_eq!(
        rows[1].get("_freshness"),
        Some(&crate::types::Value::Str("fresh".into()))
    );

    // And the outcome carries the whole declaration — rank and chain included,
    // which the one-string row stamp cannot.
    let outcome = fq.providers.iter().find(|o| o.name == "labelled").unwrap();
    assert_eq!(
        outcome.label.as_ref().and_then(|l| l.trust.as_ref()),
        Some(&Trust::new("urn:trust:partner", CHAIN, 30))
    );
    let plain = fq.providers.iter().find(|o| o.name == "plain").unwrap();
    assert!(plain.label.is_none(), "undeclared reports as null");
}

#[test]
fn health_status_carries_the_declared_label() {
    let p = StampedProvider {
        name: "labelled",
        label: Some(DeclaredLabel {
            trust: Some(Trust::new("urn:trust:partner", CHAIN, 30)),
            freshness: None,
        }),
    };
    let status = p.health();
    assert_eq!(
        status
            .label
            .as_ref()
            .and_then(|l| l.trust.as_ref())
            .map(|t| t.rank),
        Some(30)
    );
}

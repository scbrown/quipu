//! `/update` records the caller's declared actor and source (aegis-7vlk7j).
//!
//! Every SPARQL update was attributed to the constant actor AND source
//! "sparql-update", so provenance read "someone, via SPARQL" and a seeds claim
//! could not answer "who claimed this". Measured on 0.11.0: tx 393264 carried
//! actor "sparql-update", source "sparql-update".

use std::sync::Arc;

use quipu::Store;

use super::super::{SharedStore, StoreHandle};
use super::{Attribution, apply_update_attributed};

fn fresh() -> SharedStore {
    Arc::new(StoreHandle::writer_only(Store::open_in_memory().unwrap()))
}

fn fields(pairs: &[(&'static str, &'static str)]) -> Result<Attribution, String> {
    Attribution::from_fields(pairs.iter().copied())
}

#[test]
fn undeclared_actor_is_unknown_not_the_endpoint_name() {
    let a = fields(&[("using-graph-uri", "http://ex.org/g")]).unwrap();
    assert_eq!(a.actor, None);
    assert_eq!(a.source, "sparql-update");
}

#[test]
fn declared_actor_and_source_are_taken() {
    let a = fields(&[("actor", "agent:wu"), ("source", "seeds:claim")]).unwrap();
    assert_eq!(a.actor.as_deref(), Some("agent:wu"));
    assert_eq!(a.source, "seeds:claim");
}

#[test]
fn malformed_attribution_is_refused() {
    for bad in [
        vec![("actor", "a"), ("actor", "b")],
        vec![("source", "")],
        vec![("actor", "line\nbreak")],
    ] {
        assert!(
            Attribution::from_fields(bad.clone().into_iter()).is_err(),
            "{bad:?}"
        );
    }
    let long = "x".repeat(257);
    assert!(Attribution::from_fields([("actor", long.as_str())].into_iter()).is_err());
}

#[test]
fn the_transaction_carries_the_declared_attribution() {
    let shared = fresh();
    let attribution = Attribution {
        actor: Some("agent:wu".into()),
        source: "seeds:claim".into(),
    };
    let applied = apply_update_attributed(
        &shared,
        "INSERT DATA { <http://ex.org/e/s> <http://ex.org/p/claimedBy> \"wu\" }",
        false,
        &attribution,
    )
    .unwrap();
    let (_, tx) = applied.txs[0];
    let t = shared.lock().get_transaction(tx).unwrap().unwrap();
    assert_eq!(t.actor.as_deref(), Some("agent:wu"));
    assert_eq!(t.source.as_deref(), Some("seeds:claim"));
}

#[test]
fn an_unattributed_update_records_no_actor() {
    let shared = fresh();
    let applied = apply_update_attributed(
        &shared,
        "INSERT DATA { <http://ex.org/e/s> <http://ex.org/p/q> 1 }",
        false,
        &Attribution::default(),
    )
    .unwrap();
    let t = shared
        .lock()
        .get_transaction(applied.txs[0].1)
        .unwrap()
        .unwrap();
    assert_eq!(t.actor, None, "no declared actor must not read as one");
    assert_eq!(t.source.as_deref(), Some("sparql-update"));
}

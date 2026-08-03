//! Authority-intersection tests. Size-exempt (`*tests.rs`).

use super::*;

const TS: &str = "2026-01-01T00:00:00Z";

fn auth(graphs: &[&str]) -> Authority {
    Authority::over(graphs.iter().copied())
}

#[test]
fn intersection_narrows_and_never_widens() {
    // SARC §9.3's monotonicity, stated as a property rather than a case: adding
    // any link can only remove graphs.
    let broad = auth(&["g:a", "g:b", "g:c"]);
    let narrow = auth(&["g:b"]);
    let both = broad.intersect(&narrow);
    assert_eq!(both.graphs(), vec!["g:b"]);
    assert!(both.graphs().len() <= broad.graphs().len());
    assert!(both.graphs().len() <= narrow.graphs().len());
}

#[test]
fn the_wildcard_is_the_identity_not_a_widening() {
    // This is what keeps a single-tenant deployment (everyone holds `*`)
    // behaving exactly as it did before authority existed — while a
    // wildcard-holding orchestrator delegating to a scoped worker still yields
    // the WORKER's scope.
    let scoped = auth(&["g:a"]);
    assert_eq!(Authority::any().intersect(&scoped), scoped);
    assert_eq!(scoped.intersect(&Authority::any()), scoped);
    assert!(Authority::any().intersect(&Authority::any()).is_any());
}

#[test]
fn a_delegate_cannot_use_authority_its_caller_lacks() {
    // The authority-escalation-via-tool-capability defence (SARC §9.5). A
    // sub-agent whose own credentials are broader cannot use them, because the
    // effective authority is the intersection and not the executor's own.
    let caller = auth(&["g:a"]);
    let overprivileged_tool = auth(&["g:a", "g:secrets"]);
    let effective = intersect_chain(&[caller, overprivileged_tool]);
    assert!(effective.permits("g:a"));
    assert!(
        !effective.permits("g:secrets"),
        "the tool's own breadth must not survive the intersection"
    );
}

#[test]
fn an_empty_intersection_permits_nothing() {
    // Fail-safe. A chain narrowed to nothing cannot act — and must not fall
    // back to the principal's authority, which would be the escalation the rule
    // exists to stop.
    let effective = intersect_chain(&[auth(&["g:a"]), auth(&["g:b"])]);
    assert!(effective.is_empty());
    assert!(!effective.permits("g:a"));
    assert!(!effective.permits("g:b"));
    assert!(!effective.permits("*"));
}

#[test]
fn an_empty_chain_is_none_not_any() {
    // "Nobody said who is acting" must not mean "anybody may act". The caller
    // decides whether an unattributed write is permitted rather than inheriting
    // a silent yes.
    assert!(intersect_chain(&[]).is_empty());
    assert!(!intersect_chain(&[]).is_any());
}

#[test]
fn a_longer_chain_is_never_more_permissive() {
    // The property, over a growing chain.
    let links = [
        auth(&["g:a", "g:b", "g:c"]),
        auth(&["g:a", "g:b"]),
        auth(&["g:a"]),
    ];
    let mut previous = usize::MAX;
    for depth in 1..=links.len() {
        let n = intersect_chain(&links[..depth]).graphs().len();
        assert!(n <= previous, "authority grew at depth {depth}");
        previous = n;
    }
}

#[test]
fn an_undeclared_principal_holds_nothing_not_everything() {
    // Reading an absent grant as permission is how an access-control layer
    // becomes decorative.
    let store = Store::open_in_memory().unwrap();
    let a = authority_of(&store, "nobody").unwrap();
    assert!(a.is_empty());
    assert!(!a.is_any());
}

#[test]
fn a_declared_principal_reads_back_its_graphs() {
    let mut store = Store::open_in_memory().unwrap();
    let iri = "http://ex/principal/weaver";
    let class = Value::Ref(
        store
            .intern(&format!("{DEFAULT_BASE_NS}Principal"))
            .unwrap(),
    );
    let d = |store: &Store, p: &str, v: Value| crate::store::Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    };
    let datums = vec![
        d(&store, crate::namespace::RDF_TYPE, class),
        d(
            &store,
            &format!("{DEFAULT_BASE_NS}principalId"),
            Value::Str("weaver".into()),
        ),
        d(
            &store,
            &format!("{DEFAULT_BASE_NS}authorityOver"),
            Value::Str("g:hank".into()),
        ),
        d(
            &store,
            &format!("{DEFAULT_BASE_NS}authorityOver"),
            Value::Str("g:quipu".into()),
        ),
    ];
    store.transact(&datums, TS, None, None).unwrap();

    let a = authority_of(&store, "weaver").unwrap();
    assert_eq!(a.graphs(), vec!["g:hank", "g:quipu"]);
    assert!(a.permits("g:hank"));
    assert!(!a.permits("g:secrets"));
}

#[test]
fn a_refusal_names_the_chain_the_graph_and_what_is_held() {
    // A refusal that says only "denied" leaves an operator guessing which link
    // narrowed it.
    let chain = vec!["orchestrator".to_string(), "worker".to_string()];
    let why = refusal(&chain, "g:secrets", &auth(&["g:a"]));
    assert!(why.contains("orchestrator → worker"), "{why}");
    assert!(why.contains("g:secrets"), "{why}");
    assert!(why.contains("g:a"), "names what IS held: {why}");
    assert!(why.contains("INTERSECTION"), "explains the rule: {why}");
    assert!(why.contains("narrowest link"), "names the remedy: {why}");
}

#[test]
fn an_empty_authority_refusal_says_the_intersection_emptied() {
    // Distinct from "you hold these graphs, just not that one" — the operator's
    // next move is different.
    let why = refusal(&["a".into(), "b".into()], "g:x", &Authority::none());
    assert!(
        why.contains("intersection along the chain is empty"),
        "{why}"
    );
}

// ── Liveness: the check fires through the REAL write path ────────────────────

/// Declare a principal holding `graphs`.
fn declare(store: &mut Store, id: &str, graphs: &[&str]) {
    let iri = format!("http://ex/principal/{id}");
    let class = Value::Ref(
        store
            .intern(&format!("{DEFAULT_BASE_NS}Principal"))
            .unwrap(),
    );
    let d = |store: &Store, p: &str, v: Value| crate::store::Datum {
        entity: store.intern(&iri).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    };
    let mut datums = vec![
        d(store, crate::namespace::RDF_TYPE, class),
        d(
            store,
            &format!("{DEFAULT_BASE_NS}principalId"),
            Value::Str(id.into()),
        ),
    ];
    for g in graphs {
        datums.push(d(
            store,
            &format!("{DEFAULT_BASE_NS}authorityOver"),
            Value::Str((*g).into()),
        ));
    }
    store.transact(&datums, TS, None, None).unwrap();
}

fn a_fact(store: &Store, iri: &str) -> Vec<crate::store::Datum> {
    vec![crate::store::Datum {
        entity: store.intern(iri).unwrap(),
        attribute: store.intern(crate::namespace::RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern("http://ex/Thing").unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: crate::types::Op::Assert,
    }]
}

#[test]
fn a_write_outside_the_chains_authority_is_refused_at_the_write_path() {
    let mut store = Store::open_in_memory().unwrap();
    declare(&mut store, "weaver", &["g:other"]);
    store.governance_config_mut().enforce_authority = true;
    store.set_principal_chain(vec!["weaver".into()]);

    let err = store.transact(&a_fact(&store, "http://ex/x"), TS, None, None);
    let Err(crate::error::Error::PolicyDenied(why)) = err else {
        panic!("expected an authority refusal, got {err:?}");
    };
    assert!(why.contains("weaver"), "{why}");
    assert!(why.contains("g:other"), "names what IS held: {why}");
}

#[test]
fn the_control_the_same_write_lands_with_the_flag_off() {
    // Without this the test above proves only that SOMETHING rejected the
    // write. Same store, same chain, flag off => accepted.
    let mut store = Store::open_in_memory().unwrap();
    declare(&mut store, "weaver", &["g:other"]);
    assert!(!store.governance_config_mut().enforce_authority);
    store.set_principal_chain(vec!["weaver".into()]);
    store
        .transact(&a_fact(&store, "http://ex/x"), TS, None, None)
        .expect("with the flag off the same write lands");
}

#[test]
fn an_unattributed_write_is_untouched() {
    // Every existing caller sets no chain. Making attribution a hard requirement
    // beneath a running deployment would break all of them at once, so the check
    // is inert without one — the flag makes a SUPPLIED chain binding.
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_authority = true;
    assert!(store.principal_chain().is_empty());
    store
        .transact(&a_fact(&store, "http://ex/x"), TS, None, None)
        .expect("an unattributed write must pass untouched");
}

#[test]
fn a_chain_holding_root_may_write_to_root() {
    // The GREEN case. A check that refuses everything is not a check.
    let mut store = Store::open_in_memory().unwrap();
    declare(&mut store, "weaver", &[crate::schema::ROOT_GRAPH_IRI]);
    store.governance_config_mut().enforce_authority = true;
    store.set_principal_chain(vec!["weaver".into()]);
    store
        .transact(&a_fact(&store, "http://ex/x"), TS, None, None)
        .expect("a principal granted ROOT may write to ROOT");
}

#[test]
fn a_delegate_narrows_the_chain_at_the_write_path() {
    // End-to-end §9.5: the orchestrator holds ROOT, the worker does not, and the
    // chain cannot write where the orchestrator alone could.
    let mut store = Store::open_in_memory().unwrap();
    declare(&mut store, "orchestrator", &[crate::schema::ROOT_GRAPH_IRI]);
    declare(&mut store, "worker", &["g:sandbox"]);
    store.governance_config_mut().enforce_authority = true;

    store.set_principal_chain(vec!["orchestrator".into()]);
    store
        .transact(&a_fact(&store, "http://ex/a"), TS, None, None)
        .expect("the orchestrator alone may write to ROOT");

    store.set_principal_chain(vec!["orchestrator".into(), "worker".into()]);
    let err = store.transact(&a_fact(&store, "http://ex/b"), TS, None, None);
    assert!(
        matches!(err, Err(crate::error::Error::PolicyDenied(_))),
        "delegating to a worker without ROOT must narrow the chain, got {err:?}"
    );
}

#[test]
fn a_wildcard_holder_still_narrows_to_its_delegate() {
    // The wildcard declines to narrow; it does not widen. An orchestrator
    // holding `*` delegating to a scoped worker gets the WORKER's scope.
    let mut store = Store::open_in_memory().unwrap();
    declare(&mut store, "root", &[ANY_GRAPH]);
    declare(&mut store, "scoped", &["g:sandbox"]);
    store.governance_config_mut().enforce_authority = true;

    store.set_principal_chain(vec!["root".into()]);
    store
        .transact(&a_fact(&store, "http://ex/a"), TS, None, None)
        .expect("a wildcard holder may write anywhere");

    store.set_principal_chain(vec!["root".into(), "scoped".into()]);
    assert!(
        matches!(
            store.transact(&a_fact(&store, "http://ex/b"), TS, None, None),
            Err(crate::error::Error::PolicyDenied(_))
        ),
        "the wildcard must not survive delegation to a scoped worker"
    );
}

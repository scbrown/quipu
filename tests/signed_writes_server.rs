//! Signed writes through the ACTUAL server binary (aegis-bys8d1): a write
//! authenticated by a session attestation, with no bearer on the wire.
//!
//! Every arm drives the real middleware stack over raw HTTP, so a pass here
//! means the gate a client meets, not a function it calls.
#![cfg(all(feature = "shacl", feature = "onnx", feature = "server"))]

mod signed_writes_support;

use signed_writes_support::*;

#[test]
fn a_signed_write_lands_with_no_bearer_and_is_attributed_to_the_session() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-a", 1);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let (subject, body) = knot(1);
    let (status, reply) = server
        .send("POST", "/knot", &client.sign("/knot", &body), &body)
        .unwrap();
    assert_eq!(status, 200, "{reply}");
    assert_eq!(server.count(&subject), 1);
    // CONTROL: the same kind of write with the shared bearer still works.
    let (s2, b2) = knot(2);
    assert_eq!(server.bearer("/knot", &b2).0, 200);
    assert_eq!(server.count(&s2), 1);
    drop(server);
    let store = quipu::Store::open(dir.path().join("store.db").to_str().unwrap()).unwrap();
    let evidence = (1..=store.latest_tx_id().unwrap())
        .filter_map(|tx| store.transaction_auth(tx).unwrap())
        .find(|e| e.auth_class == "attested_session")
        .expect("the signed write carries attested provenance");
    assert_eq!(evidence.principal, AGENT);
    assert_eq!(
        evidence.credential_id.as_deref(),
        Some(client.binding.key_id.as_str())
    );
}

#[test]
fn a_tampered_body_is_refused_as_badsig_and_nothing_lands() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-b", 2);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let (_, signed) = knot(1);
    let (subject, sent) = knot(9);
    let (status, reply) = server
        .send("POST", "/knot", &client.sign("/knot", &signed), &sent)
        .unwrap();
    assert_eq!(status, 401, "{reply}");
    assert_eq!(verdict(&reply), "badsig");
    assert_eq!(server.count(&subject), 0);
}

#[test]
fn a_replayed_request_is_refused_and_does_not_land_twice() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-c", 3);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let (subject, body) = knot(1);
    let headers = client.sign("/knot", &body);
    assert_eq!(
        server.send("POST", "/knot", &headers, &body).unwrap().0,
        200
    );
    let (status, reply) = server.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(status, 401, "{reply}");
    assert_eq!(verdict(&reply), "replay");
    assert_eq!(server.count(&subject), 1);
}

#[test]
fn unregistered_revoked_and_expired_keys_are_refused_with_their_verdicts() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut unregistered = Client::new("seeds-unknown", 4);
    let mut revoked = Client::new("seeds-revoked", 5);
    let mut expired = Client::new("seeds-expired", 6);
    let now = quipu::time::epoch_secs();
    expired.binding = SessionBinding::new(
        AGENT,
        "seeds-expired",
        expired.binding.public_key.clone(),
        INTRODUCER,
        now - 7200,
        now - 3600,
    )
    .unwrap();
    expired.binding.allow_write = true; // isolate expiry from the scope check
    let dir = fixture(&[&revoked, &expired]);
    quipu::Store::open(dir.path().join("store.db").to_str().unwrap())
        .unwrap()
        .attestation_revoke("seeds-revoked")
        .unwrap();
    let server = spawn(dir.path());
    for (client, want) in [
        (&mut unregistered, "unbound"),
        (&mut revoked, "revoked"),
        (&mut expired, "expired"),
    ] {
        let (subject, body) = knot(u32::from(client.nonce.to_be_bytes()[7]));
        let (status, reply) = server
            .send("POST", "/knot", &client.sign("/knot", &body), &body)
            .unwrap();
        assert_eq!((status, verdict(&reply)), (401, want.to_owned()), "{reply}");
        assert_eq!(server.count(&subject), 0);
    }
}

#[test]
fn an_invalid_attestation_never_falls_back_to_a_valid_bearer() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-d", 7);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let (_, signed) = knot(1);
    let (subject, sent) = knot(2);
    let mut headers = client.sign("/knot", &signed);
    headers.push(("Authorization", format!("Bearer {BEARER}")));
    let (status, reply) = server.send("POST", "/knot", &headers, &sent).unwrap();
    assert_eq!(status, 401, "{reply}");
    assert_eq!(server.count(&subject), 0);
}

#[test]
fn a_query_string_an_unlisted_endpoint_and_a_control_char_are_refused() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-e", 8);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let (_, body) = knot(1);
    let with_query = client.sign("/knot", &body);
    assert_eq!(
        verdict(
            &server
                .send("POST", "/knot?x=1", &with_query, &body)
                .unwrap()
                .1
        ),
        "invalid"
    );
    let unlisted = client.sign("/set", &body);
    assert_eq!(
        verdict(&server.send("POST", "/set", &unlisted, &body).unwrap().1),
        "invalid"
    );
    let bad = Client {
        binding: SessionBinding {
            session: "seeds\nintroducer=urn:crew:evil".into(),
            ..client.binding.clone()
        },
        key: Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap(),
        nonce: 0,
    };
    let control = bad.sign_with("/knot", &body, "00000000000000000000000000000001");
    let (status, reply) = server.send("POST", "/knot", &control, &body).unwrap();
    assert_eq!(
        (status, verdict(&reply)),
        (401, "invalid".to_owned()),
        "{reply}"
    );
}

#[test]
fn a_rejected_signed_write_still_spends_its_nonce() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("seeds-host-f", 9);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let body = serde_json::json!({ "turtle": "this is not turtle <" }).to_string();
    let headers = client.sign("/knot", &body);
    let (first, reply) = server.send("POST", "/knot", &headers, &body).unwrap();
    assert!(
        (400..500).contains(&first) && first != 401,
        "{first} {reply}"
    );
    let (status, reply) = server.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(
        (status, verdict(&reply)),
        (401, "replay".to_owned()),
        "{reply}"
    );
}

#[test]
fn a_no_op_signed_write_cannot_be_replayed_after_the_state_changes() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // malcolm's arm: a signed episode that changes nothing still spends its
    // nonce, so a replay after a retraction cannot re-create what was removed.
    let mut client = Client::new("seeds-host-g", 10);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let episode = serde_json::json!({
        "name": "signed-noop-probe",
        "episode_body": "probe",
        "source": "signed-writes-test",
        "group_id": "test",
        "nodes": [{"name": "signed-noop-entity", "type": "Thing"}]
    })
    .to_string();
    let (status, reply) = server.bearer("/episode", &episode);
    assert_eq!(status, 200, "{reply}");
    let headers = client.sign("/episode", &episode);
    let (status, reply) = server.send("POST", "/episode", &headers, &episode).unwrap();
    assert_eq!(status, 200, "{reply}");
    assert!(reply.contains("unchanged"), "{reply}");
    let entity = "http://aegis.gastown.local/ontology/signed-noop-entity";
    let before = server.count(entity);
    assert!(before > 0);
    let retract = serde_json::json!({ "entity": entity }).to_string();
    assert_eq!(server.bearer("/retract", &retract).0, 200);
    assert_eq!(server.count(entity), 0);
    let (status, reply) = server.send("POST", "/episode", &headers, &episode).unwrap();
    assert_eq!(
        (status, verdict(&reply)),
        (401, "replay".to_owned()),
        "{reply}"
    );
    assert_eq!(
        server.count(entity),
        0,
        "the replay must not re-create the entity"
    );
    let _ = &server.root;
}

#[test]
fn a_share_only_binding_is_refused_as_scope_until_write_is_granted() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // wu's arm: a key registered to trust a share PRODUCER must not thereby
    // be able to sign writes. The same key succeeds only after the grant.
    let mut client = Client::new("seeds-share-only", 11);
    client.binding.allow_write = false;
    let dir = fixture(&[&client]);
    {
        let server = spawn(dir.path());
        let (subject, body) = knot(1);
        let (status, reply) = server
            .send("POST", "/knot", &client.sign("/knot", &body), &body)
            .unwrap();
        assert_eq!(
            (status, verdict(&reply)),
            (401, "scope".to_owned()),
            "{reply}"
        );
        assert_eq!(server.count(&subject), 0);
    }
    quipu::Store::open(dir.path().join("store.db").to_str().unwrap())
        .unwrap()
        .attestation_set_write("seeds-share-only", true)
        .unwrap();
    let server = spawn(dir.path());
    let (subject, body) = knot(2);
    let (status, reply) = server
        .send("POST", "/knot", &client.sign("/knot", &body), &body)
        .unwrap();
    assert_eq!(status, 200, "{reply}");
    assert_eq!(server.count(&subject), 1);
}

#[test]
fn a_signed_graph_create_lands_and_its_replay_is_refused() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // wu's contract gap: seeds creates its graph before writing to it.
    let mut client = Client::new("seeds-graph", 12);
    let dir = fixture(&[&client]);
    let server = spawn(dir.path());
    let body = serde_json::json!({ "graph": "urn:signed:graph" }).to_string();
    let headers = client.sign("/graph/create", &body);
    let (status, reply) = server
        .send("POST", "/graph/create", &headers, &body)
        .unwrap();
    assert_eq!(status, 200, "{reply}");
    assert!(reply.contains("\"created\":true"), "{reply}");
    let (status, reply) = server
        .send("POST", "/graph/create", &headers, &body)
        .unwrap();
    assert_eq!(
        (status, verdict(&reply)),
        (401, "replay".to_owned()),
        "{reply}"
    );
    let (_, listing) = server.send("GET", "/graphs", &[], "").unwrap();
    assert!(listing.contains("urn:signed:graph"), "{listing}");
}

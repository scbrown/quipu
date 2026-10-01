//! quipu-write-v2 through the ACTUAL server binary (aegis-72cpbx): the
//! audience binds a signed write to the store it was signed for.
#![cfg(all(feature = "shacl", feature = "onnx", feature = "server"))]

mod signed_writes_support;

use signed_writes_support::*;

/// The defect v2 exists for, pinned so it stays visible: v1 names no server,
/// so one signed request is accepted by BOTH stores that trust the key. This
/// is v1's documented limit, not a regression to fix in v1.
#[test]
fn baseline_a_v1_write_accepted_by_one_store_is_accepted_by_another() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("relay-v1", 11);
    let (a_dir, b_dir) = (fixture(&[&client]), fixture(&[&client]));
    let (subject, body) = knot(1);
    let headers = client.sign("/knot", &body);
    let a = spawn(a_dir.path());
    assert_eq!(a.send("POST", "/knot", &headers, &body).unwrap().0, 200);
    drop(a);
    let b = spawn(b_dir.path());
    assert_eq!(b.send("POST", "/knot", &headers, &body).unwrap().0, 200);
    assert_eq!(
        b.count(&subject),
        1,
        "v1 relayed: the other store accepted it"
    );
}

#[test]
fn a_v2_write_signed_for_one_store_is_refused_by_another_as_invalid() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("relay-v2", 12);
    let (a_dir, b_dir) = (fixture(&[&client]), fixture(&[&client]));
    let a_id = store_id(&a_dir);
    assert_ne!(a_id, store_id(&b_dir), "two stores, two ids");
    let (subject, body) = knot(1);
    let headers = client.sign_v2("/knot", &body, Some(&a_id), Some(&a_id), WRITE_V2);

    let b = spawn(b_dir.path());
    let (status, reply) = b.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(status, 401, "{reply}");
    assert_eq!(verdict(&reply), "invalid");
    assert!(reply.contains("audience"), "{reply}");
    assert_eq!(b.count(&subject), 0);
    drop(b);

    // CONTROL: the same request is accepted by the store it was signed for,
    // so the refusal above is the audience and not a broken v2.
    let a = spawn(a_dir.path());
    let (status, reply) = a.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(status, 200, "{reply}");
    assert_eq!(a.count(&subject), 1);
}

/// The envelope's audience is only a claim. Rewriting it to the relay target
/// passes the comparison and then fails the signature, because the server
/// signs its OWN id into the message.
#[test]
fn swapping_the_envelope_audience_to_the_relay_target_fails_the_signature() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("relay-swap", 13);
    let (a_dir, b_dir) = (fixture(&[&client]), fixture(&[&client]));
    let (a_id, b_id) = (store_id(&a_dir), store_id(&b_dir));
    let (subject, body) = knot(1);
    let headers = client.sign_v2("/knot", &body, Some(&a_id), Some(&b_id), WRITE_V2);
    let b = spawn(b_dir.path());
    let (status, reply) = b.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(status, 401, "{reply}");
    assert_eq!(verdict(&reply), "badsig");
    assert_eq!(b.count(&subject), 0);
}

#[test]
fn a_v2_without_an_audience_and_a_v1_with_one_are_both_invalid() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut client = Client::new("audience-shape", 14);
    let dir = fixture(&[&client]);
    let id = store_id(&dir);
    let server = spawn(dir.path());
    let (subject, body) = knot(1);
    // v2 with no audience: signed over the v1 shape under the v2 tag.
    let headers = client.sign_v2("/knot", &body, None, None, WRITE_V2);
    let (status, reply) = server.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(
        (status, verdict(&reply).as_str()),
        (401, "invalid"),
        "{reply}"
    );
    // v1 carrying an audience: a field v1 never signs.
    let headers = client.sign_v2("/knot", &body, None, Some(&id), WRITE_V1);
    let (status, reply) = server.send("POST", "/knot", &headers, &body).unwrap();
    assert_eq!(
        (status, verdict(&reply).as_str()),
        (401, "invalid"),
        "{reply}"
    );
    assert_eq!(server.count(&subject), 0);
}

#[test]
fn stats_names_the_store_a_v2_client_signs_for() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = fixture(&[]);
    let id = store_id(&dir);
    let server = spawn(dir.path());
    let (status, body) = server.send("GET", "/stats", &[], "").unwrap();
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["store_id"].as_str(), Some(id.as_str()));
}

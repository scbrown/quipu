//! One test process: exact process-global deltas cannot race sibling unit tests.
#![cfg(not(target_arch = "wasm32"))]

use quipu::Store;
use quipu::session_attestation::{
    AttestationEnvelope, BindingRegistry, SessionBinding, ShareBinding, SignedBinding, WriteBinding,
};
use quipu::share::{ShareManifest, ShareOptions};
use quipu::share_import::{ShareImportRequest, import_share};
use ring::signature::{Ed25519KeyPair, KeyPair};

const TS: &str = "2026-09-05T12:00:00Z";
const NOW: u64 = 1_788_609_600;

fn count(metric: &str, labels: &str) -> u64 {
    let prefix = format!("{metric}{{{labels}}} ");
    quipu::metrics::metrics()
        .render(0, 0, 0, None)
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map_or(0, |n| n.parse().unwrap())
}

fn verification(domain: &str, result: &str) -> u64 {
    count(
        "quipu_attestation_verify_total",
        &format!("binding=\"{domain}\",result=\"{result}\""),
    )
}

fn imports(outcome: &str, tier: &str) -> u64 {
    count(
        "quipu_share_import_total",
        &format!("outcome=\"{outcome}\",tier=\"{tier}\""),
    )
}

fn key() -> Ed25519KeyPair {
    let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

fn envelope(
    key: &Ed25519KeyPair,
    binding: &SessionBinding,
    payload: &SignedBinding<'_>,
) -> AttestationEnvelope {
    let mut envelope = AttestationEnvelope {
        version: payload.version().into(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: NOW,
        nonce: "b".repeat(32),
        signature: String::new(),
    };
    envelope.signature = hex::encode(
        key.sign(&quipu::session_attestation::canonical_message(
            &envelope, payload,
        ))
        .as_ref(),
    );
    envelope
}

fn share_binding(m: &ShareManifest) -> SignedBinding<'_> {
    SignedBinding::Share(ShareBinding {
        share_id: &m.share_id,
        graph_hash: &m.graph_hash,
        shapes_hash: &m.shapes_hash,
        tx_anchor: m.tx_anchor,
    })
}

#[test]
fn real_imports_and_verifier_failures_increment_exactly_once() {
    let mut producer = Store::open_in_memory().unwrap();
    let catalogue = producer.overlay_create("urn:test:catalogue", 0).unwrap();
    quipu::rdf::ingest_rdf_to_graph(
        &mut producer,
        include_bytes!("fixtures/share-catalogue.ttl").as_slice(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-08-01T00:00:00Z",
        None,
        Some("test-catalogue"),
        catalogue,
    )
    .unwrap();
    quipu::rdf::ingest_rdf(
        &mut producer,
        &b"<urn:a> <urn:p> \"one\" .\n"[..],
        oxrdfio::RdfFormat::NTriples,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("share");
    quipu::share::share(
        &producer,
        path.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            ..Default::default()
        },
    )
    .unwrap();
    let read = |name| std::fs::read_to_string(path.join(name)).unwrap();
    let mut req = ShareImportRequest {
        manifest: serde_json::from_str(&read("manifest.json")).unwrap(),
        export_ntriples: read("export.nt"),
        shapes_turtle: read("shapes.ttl"),
        source: "https://example.org/metrics-proof".into(),
        actor: None,
        accept_exact: false,
        destination: Default::default(),
        attestation: None,
    };
    let mut consumer = Store::open_in_memory().unwrap();
    let transport = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(imports(&transport.outcome, "transport"), 1);
    assert_eq!(
        verification("share", "ok"),
        0,
        "unsigned imports are not signature verification"
    );

    let key = key();
    let binding = SessionBinding::new(
        "alice",
        "metrics-session",
        hex::encode(key.public_key().as_ref()),
        "introducer",
        NOW - 60,
        NOW + 3600,
    )
    .unwrap();
    consumer.attestation_register(&binding).unwrap();
    req.attestation = Some(envelope(&key, &binding, &share_binding(&req.manifest)));
    let imported = import_share(&mut consumer, &req, TS, None).unwrap();
    assert_eq!(imported.attestation.tier, "attested");
    assert_eq!(imports(&imported.outcome, "attested"), 1);
    assert_eq!(verification("share", "ok"), 1);
    assert!(import_share(&mut consumer, &req, TS, None).is_err());
    assert_eq!(verification("share", "replay"), 1);
    assert_eq!(verification("share", "ok"), 1);
    assert_eq!(
        imports("error", "unverified"),
        1,
        "a supplied envelope is not a verified tier"
    );

    // Malformed transport fails before the verifier; it cannot count as badsig.
    req.export_ntriples.push_str("corrupt");
    assert!(import_share(&mut consumer, &req, TS, None).is_err());
    assert_eq!(imports("error", "unverified"), 2);
    assert_eq!(verification("share", "badsig"), 0);

    // A signed share from an unregistered producer remains claimed, even on replay.
    let signed_path = dir.path().join("signed-share");
    quipu::share::share(
        &producer,
        signed_path.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            attest: Some(quipu::share::AttestOptions {
                key_path: dir.path().join("producer.pk8"),
                agent: "claimed-alice".into(),
                session: "claimed-session".into(),
                introducer: "self".into(),
                issued_at_epoch: NOW,
                expires_at_epoch: NOW + 3600,
                nonce: "c".repeat(32),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let read_signed = |name| std::fs::read_to_string(signed_path.join(name)).unwrap();
    let manifest: ShareManifest = serde_json::from_str(&read_signed("manifest.json")).unwrap();
    let mut claimed = ShareImportRequest {
        attestation: Some(manifest.attestation.as_ref().unwrap().envelope.clone()),
        manifest,
        export_ntriples: read_signed("export.nt"),
        shapes_turtle: read_signed("shapes.ttl"),
        source: "https://example.org/claimed".into(),
        actor: None,
        accept_exact: false,
        destination: Default::default(),
    };
    let mut unregistered = Store::open_in_memory().unwrap();
    let ok_before = verification("share", "ok");
    let out = import_share(&mut unregistered, &claimed, TS, None).unwrap();
    assert_eq!(out.attestation.tier, "claimed");
    assert_eq!(imports(&out.outcome, "claimed"), 1);
    assert_eq!(verification("share", "ok"), ok_before + 1);
    let repeat = import_share(&mut unregistered, &claimed, TS, None).unwrap();
    assert_eq!(repeat.outcome, "unchanged");
    assert_eq!(imports("unchanged", "claimed"), 1);
    assert_eq!(verification("share", "ok"), ok_before + 2);
    claimed.attestation.as_mut().unwrap().signature = "00".repeat(64);
    assert!(import_share(&mut unregistered, &claimed, TS, None).is_err());
    assert_eq!(verification("share", "invalid"), 1);
    assert_eq!(verification("share", "ok"), ok_before + 2);

    // Exercise the common verifier in the other domain, including every refusal.
    let registry = BindingRegistry::default();
    let payload = SignedBinding::Write(WriteBinding {
        method: "POST",
        path: "/episode",
        content_type: "application/json",
        body_sha256: "sha256:fixture",
    });
    let env = envelope(&key, &binding, &payload);
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
    assert_eq!(verification("write", "unbound"), 1);
    registry.register(binding.clone()).unwrap();
    let mut bad = env.clone();
    bad.signature = "00".repeat(64);
    assert!(registry.verify(&bad, &payload, NOW, 30).is_err());
    assert_eq!(verification("write", "badsig"), 1);
    bad = env.clone();
    bad.nonce = "bad nonce".into();
    assert!(registry.verify(&bad, &payload, NOW, 30).is_err());
    assert_eq!(verification("write", "invalid"), 1);
    assert!(registry.verify(&env, &payload, NOW + 31, 30).is_err());
    assert_eq!(verification("write", "skew"), 1);
    assert!(
        registry.verify(&env, &payload, NOW, 30).is_ok(),
        "rejected attempts must not consume the valid nonce"
    );
    assert_eq!(verification("write", "ok"), 1);
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
    assert_eq!(verification("write", "replay"), 1);
    registry.revoke(&binding.session).unwrap();
    assert!(registry.verify(&env, &payload, NOW, 30).is_err());
    assert_eq!(verification("write", "revoked"), 1);

    struct BrokenRegistry;
    impl quipu::session_attestation::AttestationBindings for BrokenRegistry {
        fn binding(&self, _: &str) -> quipu::Result<Option<SessionBinding>> {
            Err(quipu::Error::InvalidValue(
                "synthetic storage failure".into(),
            ))
        }
        fn consume_nonce(&self, _: &str, _: &str, _: u64) -> quipu::Result<bool> {
            unreachable!()
        }
    }
    assert!(
        quipu::session_attestation::verify_binding(&BrokenRegistry, &env, &payload, NOW, 30)
            .is_err()
    );
    assert_eq!(verification("write", "error"), 1);
    let text = quipu::metrics::metrics().render(0, 0, 0, None);
    for secret_or_identity in [
        &binding.session,
        &binding.public_key,
        &env.signature,
        &env.nonce,
    ] {
        assert!(
            !text.contains(secret_or_identity),
            "metric labels must not contain caller material"
        );
    }
}

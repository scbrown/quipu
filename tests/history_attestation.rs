use quipu::reasoner::history_attestation::*;
use quipu::signing::{public_key_hex, sign_hex};
use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fixture {
    key: Ed25519KeyPair,
    public_key: String,
    envelope: SignedHistoryAttestation,
}

impl Fixture {
    fn new() -> Self {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap();
        let public_key = public_key_hex(&key);
        let payload = HistoryAttestation {
            schema_version: 1,
            issuer_id: "synthetic-reviewer".into(),
            grant_policy_sha256: hash(b"private test policy v1"),
            reviewed_evidence_ref: "urn:test:coverage-review:1".into(),
            reviewed_evidence_sha256: hash(b"includes pre-window catalogue and premise history"),
            raw_artifact_sha256: hash(b"synthetic retained artifact"),
            artifact_kind: HistoryArtifactKind::RawSqliteSnapshot,
            provider_scope: HistoryProviderScope::NativeOnly,
            coverage: HistoryCoverage {
                source_store_id: "synthetic-lineage".into(),
                catalogue_graph: "urn:test:catalogue".into(),
                premise_graph: "urn:test:premises".into(),
                transaction_first: 1,
                transaction_last: 9,
                valid_time_from: 100,
                valid_time_until: 200,
            },
            issued_at: 300,
            expires_at: 400,
        };
        let signature = sign_hex(&key, &history_attestation_message(&payload).unwrap());
        Self {
            key,
            public_key,
            envelope: SignedHistoryAttestation { payload, signature },
        }
    }

    fn grant(&self) -> HistoryCoverageGrant<'_> {
        HistoryCoverageGrant {
            issuer_id: "synthetic-reviewer",
            public_key_hex: &self.public_key,
            policy_sha256: &self.envelope.payload.grant_policy_sha256,
            policy_bytes: b"private test policy v1",
            reviewed_evidence_ref: "urn:test:coverage-review:1",
            reviewed_evidence_sha256: &self.envelope.payload.reviewed_evidence_sha256,
            raw_artifact_sha256: &self.envelope.payload.raw_artifact_sha256,
            coverage: &self.envelope.payload.coverage,
        }
    }
}

const EVIDENCE: &[u8] = b"includes pre-window catalogue and premise history";

#[cfg(not(target_arch = "wasm32"))]
fn raw_fixture(frozen: bool) -> (Vec<u8>, String, i64) {
    use quipu::{
        store::{Datum, Store},
        types::{Op, Value},
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retained.db");
    let mut store = Store::open(path.to_str().unwrap()).unwrap();
    let lineage = store.store_id().unwrap();
    store.graph_create("urn:test:catalogue").unwrap();
    store.graph_create("urn:test:premises").unwrap();
    // Interning alone must not establish graph coverage.
    store.intern("urn:test:term-only").unwrap();
    let datum = Datum {
        entity: store.intern("urn:test:claim").unwrap(),
        attribute: store.intern("urn:test:deadline").unwrap(),
        value: Value::Ref(store.intern("urn:test:old-deadline").unwrap()),
        valid_from: "1970-01-01T00:00:00Z".into(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(
            std::slice::from_ref(&datum),
            "1970-01-01T00:00:01Z",
            None,
            None,
        )
        .unwrap();
    let premise_graph = store
        .registered_graph_id("urn:test:premises")
        .unwrap()
        .unwrap();
    store
        .transact_to_graph(&[datum], "1970-01-01T00:00:02Z", None, None, premise_graph)
        .unwrap();
    let tx = store.latest_tx_id().unwrap();
    drop(store);
    if frozen {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute("INSERT INTO frozen_packs (graph_iri,alias,path,space,content_hash,frozen_at) VALUES ('urn:test:frozen','external','missing.db',99,'unverified','1970-01-01T00:00:01Z')", []).unwrap();
    }
    assert!(!path.with_extension("db-wal").exists());
    (std::fs::read(path).unwrap(), lineage, tx)
}

#[cfg(not(target_arch = "wasm32"))]
fn authorize_raw(fixture: &mut Fixture, bytes: &[u8], lineage: String, tx: i64) {
    fixture.envelope.payload.raw_artifact_sha256 = hash(bytes);
    fixture.envelope.payload.coverage.source_store_id = lineage;
    fixture.envelope.payload.coverage.transaction_first = tx;
    fixture.envelope.payload.coverage.transaction_last = tx;
    fixture.envelope.signature = sign_hex(
        &fixture.key,
        &history_attestation_message(&fixture.envelope.payload).unwrap(),
    );
}

#[test]
fn signed_terms_bind_external_policy_and_actual_review_bytes() {
    let fixture = Fixture::new();
    let grant = fixture.grant();
    let receipt =
        verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), 350).unwrap();
    assert_eq!(receipt.payload(), &fixture.envelope.payload);
    assert_eq!(receipt.admitted_at(), 350);
    assert!(matches!(
        verify_history_attestation(&fixture.envelope, &grant, None, 350),
        Err(HistoryAttestationError::EvidenceUnavailable)
    ));
    assert!(matches!(
        verify_history_attestation(&fixture.envelope, &grant, Some(b"changed review"), 350),
        Err(HistoryAttestationError::EvidenceMismatch)
    ));
    let wrong_policy = HistoryCoverageGrant {
        policy_bytes: b"changed policy",
        ..fixture.grant()
    };
    assert!(matches!(
        verify_history_attestation(&fixture.envelope, &wrong_policy, Some(EVIDENCE), 350),
        Err(HistoryAttestationError::EvidenceMismatch)
    ));
    // The external grant cannot be widened by even a correctly signed issuer.
    let mut broader = fixture.envelope.clone();
    broader.payload.coverage.transaction_last = 10;
    broader.signature = sign_hex(
        &fixture.key,
        &history_attestation_message(&broader.payload).unwrap(),
    );
    assert!(matches!(
        verify_history_attestation(&broader, &grant, Some(EVIDENCE), 350),
        Err(HistoryAttestationError::Unauthorized)
    ));
    let mut changed = fixture.envelope.clone();
    changed.payload.raw_artifact_sha256 = hash(b"replacement artifact");
    changed.signature = sign_hex(
        &fixture.key,
        &history_attestation_message(&changed.payload).unwrap(),
    );
    assert!(matches!(
        verify_history_attestation(&changed, &grant, Some(EVIDENCE), 350),
        Err(HistoryAttestationError::Unauthorized)
    ));
    changed = fixture.envelope.clone();
    changed.payload.expires_at += 1;
    assert!(matches!(
        verify_history_attestation(&changed, &grant, Some(EVIDENCE), 350),
        Err(HistoryAttestationError::Signature)
    ));
}

#[test]
fn fresh_admission_expiry_does_not_mutate_a_retained_receipt() {
    let fixture = Fixture::new();
    let grant = fixture.grant();
    let original =
        verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), 300).unwrap();
    for time in [299, 400, 401] {
        assert!(matches!(
            verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), time),
            Err(HistoryAttestationError::NotCurrent)
        ));
    }
    assert_eq!(original.admitted_at(), 300);
    assert_eq!(original.payload().expires_at, 400);
}

#[test]
fn strict_versioned_serialization_refuses_ambiguous_or_extra_fields() {
    let fixture = Fixture::new();
    let bytes = serde_json::to_vec(&fixture.envelope).unwrap();
    let parsed: SignedHistoryAttestation = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        history_attestation_message(&parsed.payload).unwrap(),
        history_attestation_message(&fixture.envelope.payload).unwrap()
    );
    assert!(
        history_attestation_message(&parsed.payload)
            .unwrap()
            .starts_with(b"quipu-history-source/v1\0{\"schema_version\":1,\"issuer_id\":")
    );
    let mut value = serde_json::to_value(&fixture.envelope).unwrap();
    value["payload"]["coverage"]["complete"] = true.into();
    assert!(serde_json::from_value::<SignedHistoryAttestation>(value).is_err());
    let duplicate = String::from_utf8(bytes).unwrap().replace(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
    );
    assert!(serde_json::from_str::<SignedHistoryAttestation>(&duplicate).is_err());
    let mut provider = serde_json::to_value(&fixture.envelope).unwrap();
    provider["payload"]["provider_scope"] = "attached".into();
    assert!(serde_json::from_value::<SignedHistoryAttestation>(provider).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn raw_history_binds_actual_artifact_and_owns_an_immutable_copy() {
    use quipu::{config::QuipuConfig, reasoner::raw_history::*, store::Store};
    let (mut bytes, lineage, tx) = raw_fixture(false);
    let original_digest = hash(&bytes);
    let mut fixture = Fixture::new();
    authorize_raw(&mut fixture, &bytes, lineage.clone(), tx);
    let grant = fixture.grant();
    let terms = verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), 350).unwrap();
    let mut policy = Store::open_in_memory().unwrap();
    policy.set_base_ns("urn:test:policy:");
    policy.search_config_mut().max_sparql_rows = 7;
    let source = admit_raw_history(
        &bytes,
        &terms,
        RawHistoryRequest {
            valid_at: 150,
            as_of_tx: tx,
        },
        &QuipuConfig::default(),
        &policy,
        350,
    )
    .unwrap();
    assert_eq!(source.store().store_id().unwrap(), lineage);
    assert_eq!(source.terms().raw_artifact_sha256, original_digest);
    assert_eq!(source.admitted_at(), 350);
    assert_eq!(source.request().as_of_tx, tx);
    assert_eq!(source.store().base_ns(), "urn:test:policy:");
    assert_eq!(source.store().search_config().max_sparql_rows, 7);
    let graph = source
        .store()
        .registered_graph_id("urn:test:premises")
        .unwrap()
        .unwrap();
    assert_eq!(
        source.store().current_facts_in_graph(graph).unwrap().len(),
        1
    );
    for now in [349, 400] {
        assert!(matches!(
            admit_raw_history(
                &bytes,
                &terms,
                source.request().clone(),
                &QuipuConfig::default(),
                &policy,
                now
            ),
            Err(RawHistoryError::NotCurrent)
        ));
    }
    assert_eq!(source.store().current_facts().unwrap().len(), 1);
    assert_eq!(hash(&bytes), original_digest);
    bytes[0] ^= 1;
    assert_eq!(source.store().current_facts().unwrap().len(), 1);
    assert!(matches!(
        admit_raw_history(
            &bytes,
            &terms,
            source.request().clone(),
            &QuipuConfig::default(),
            &policy,
            350
        ),
        Err(RawHistoryError::ArtifactMismatch)
    ));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn raw_history_refuses_unknown_scope_lineage_transactions_and_providers() {
    use quipu::{
        config::{AttachmentConfig, QuipuConfig},
        reasoner::raw_history::*,
        store::Store,
    };
    let (bytes, lineage, tx) = raw_fixture(false);
    let policy = Store::open_in_memory().unwrap();
    let request = RawHistoryRequest {
        valid_at: 150,
        as_of_tx: tx,
    };
    for (case, expected) in [
        ("lineage", "lineage"),
        ("tx", "tx"),
        ("graph", "graph"),
        ("window", "window"),
    ] {
        let mut fixture = Fixture::new();
        authorize_raw(&mut fixture, &bytes, lineage.clone(), tx);
        match case {
            "lineage" => fixture.envelope.payload.coverage.source_store_id = "other".into(),
            "tx" => fixture.envelope.payload.coverage.transaction_last = tx + 99,
            "graph" => {
                fixture.envelope.payload.coverage.premise_graph = "urn:test:term-only".into();
            }
            "window" => fixture.envelope.payload.coverage.valid_time_until = 150,
            _ => unreachable!(),
        }
        fixture.envelope.signature = sign_hex(
            &fixture.key,
            &history_attestation_message(&fixture.envelope.payload).unwrap(),
        );
        let grant = fixture.grant();
        let terms =
            verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), 350).unwrap();
        let error = admit_raw_history(
            &bytes,
            &terms,
            request.clone(),
            &QuipuConfig::default(),
            &policy,
            350,
        )
        .err()
        .unwrap();
        assert!(matches!(
            (expected, error),
            ("lineage", RawHistoryError::LineageMismatch)
                | ("tx", RawHistoryError::UnknownTransaction)
                | ("graph", RawHistoryError::MissingGraph)
                | ("window", RawHistoryError::OutsideCoverage)
        ));
    }
    let mut fixture = Fixture::new();
    authorize_raw(&mut fixture, &bytes, lineage, tx);
    let grant = fixture.grant();
    let terms = verify_history_attestation(&fixture.envelope, &grant, Some(EVIDENCE), 350).unwrap();
    let config = QuipuConfig {
        attachments: vec![AttachmentConfig::new("unmounted", "missing.db")],
        ..QuipuConfig::default()
    };
    assert!(matches!(
        admit_raw_history(&bytes, &terms, request.clone(), &config, &policy, 350),
        Err(RawHistoryError::AttachedProviders)
    ));
    let (frozen_bytes, frozen_lineage, frozen_tx) = raw_fixture(true);
    let mut frozen_fixture = Fixture::new();
    authorize_raw(
        &mut frozen_fixture,
        &frozen_bytes,
        frozen_lineage,
        frozen_tx,
    );
    let frozen_grant = frozen_fixture.grant();
    let frozen_terms =
        verify_history_attestation(&frozen_fixture.envelope, &frozen_grant, Some(EVIDENCE), 350)
            .unwrap();
    assert!(matches!(
        admit_raw_history(
            &frozen_bytes,
            &frozen_terms,
            request,
            &QuipuConfig::default(),
            &policy,
            350
        ),
        Err(RawHistoryError::AttachedProviders)
    ));
}

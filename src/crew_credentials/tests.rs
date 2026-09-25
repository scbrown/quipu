use super::*;
use serde_json::{Value, json};

fn credential(id: &str, principal: &str, token: &str) -> Value {
    json!({
        "credential_id": id, "principal": principal, "audience": "quipu",
        "token_sha256": format!("{:x}", Sha256::digest(token.as_bytes()))
    })
}

fn parse(entries: &[Value]) -> Result<CredentialRegistry, &'static str> {
    CredentialRegistry::parse(
        &serde_json::to_vec(&json!({"version": 1, "credentials": entries})).unwrap(),
        "quipu",
    )
}

#[test]
fn two_credentials_resolve_only_their_issued_identity() {
    let registry = parse(&[
        credential("a-1", "urn:crew:alice", "alice-secret"),
        credential("b-1", "urn:crew:bob", "bob-secret"),
    ])
    .unwrap();
    assert_eq!(registry.len(), 2);
    for (token, iri, id) in [
        ("alice-secret", "urn:crew:alice", "a-1"),
        ("bob-secret", "urn:crew:bob", "b-1"),
    ] {
        let p = registry
            .authenticate(Some(&format!("Bearer {token}")))
            .unwrap();
        assert_eq!(p.iri, iri);
        assert_eq!(p.credential_id, id);
        assert!(!format!("{p:?}").contains(token));
    }
    for header in [
        None,
        Some("Bearer impostor"),
        Some("Bearer "),
        Some("Basic alice-secret"),
    ] {
        assert_eq!(registry.authenticate(header), None);
    }
}

#[test]
fn multiple_keys_for_one_identity_are_additive() {
    let registry = parse(&[
        credential("old", "urn:crew:alice", "old-secret"),
        credential("additional", "urn:crew:alice", "new-secret"),
    ])
    .unwrap();
    for token in ["old-secret", "new-secret"] {
        assert_eq!(
            registry
                .authenticate(Some(&format!("Bearer {token}")))
                .unwrap()
                .iri,
            "urn:crew:alice"
        );
    }
}

#[test]
fn duplicate_keys_and_wrong_audience_cannot_create_ambiguous_identity() {
    let a = credential("a", "urn:crew:alice", "a-secret");
    for b in [
        credential("a", "urn:crew:bob", "b-secret"),
        credential("b", "urn:crew:bob", "a-secret"),
    ] {
        assert!(parse(&[a.clone(), b]).is_err());
    }
    let mut other = a;
    other["audience"] = json!("other-service");
    assert!(parse(&[other]).is_err());
}

#[test]
fn invalid_registry_never_echoes_credential_material() {
    let secret = "SUPER_SECRET_MUST_NOT_APPEAR";
    for field in ["credential_id", "principal", "token_sha256", "audience"] {
        let mut entry = credential("a", "urn:crew:alice", "a-secret");
        entry[field] = json!(format!("{secret}\n"));
        let error = parse(&[entry]).err().unwrap();
        assert!(!error.contains(secret));
    }
    let error = CredentialRegistry::parse(format!("{{{secret}}}").as_bytes(), "quipu")
        .err()
        .unwrap();
    assert!(!error.contains(secret));
}

#[test]
fn registry_bounds_and_unknown_fields_are_refused() {
    assert!(CredentialRegistry::parse(&vec![b' '; 1024 * 1024 + 1], "quipu").is_err());
    for document in [
        json!({"version": 2, "credentials": []}),
        json!({"version": 1, "credentials": [], "plaintext_token": "secret"}),
        json!({"version": 1, "credentials": [credential("a", "urn:crew:alice", "a-secret")], "rotation": true}),
    ] {
        assert!(
            CredentialRegistry::parse(&serde_json::to_vec(&document).unwrap(), "quipu").is_err()
        );
    }
    assert!(CredentialRegistry::default().is_empty());
    assert!(
        parse(&[])
            .unwrap()
            .authenticate(Some("Bearer whatever"))
            .is_none()
    );
}

#[test]
fn additive_policy_preserves_shared_reads_and_read_only_precedence() {
    use crate::http_auth::{AccessDecision, AuthGeneration, BearerPolicy, authorize_bearers};
    let registry = parse(&[
        credential("a", "urn:crew:alice", "alice-secret"),
        credential("b", "urn:crew:bob", "bob-secret"),
    ])
    .unwrap();
    let policy = BearerPolicy::new(Some("shared".into()), None, None, 1)
        .unwrap()
        .with_named(registry.clone());
    for (header, generation) in [
        ("Bearer shared", AuthGeneration::Current),
        ("Bearer alice-secret", AuthGeneration::Named),
        ("Bearer bob-secret", AuthGeneration::Named),
    ] {
        let result = authorize_bearers(true, false, &policy, Some(header), 1);
        assert_eq!(result.decision, AccessDecision::Allow);
        assert_eq!(result.generation, Some(generation));
        assert_eq!(
            authorize_bearers(true, true, &policy, Some(header), 1).decision,
            AccessDecision::ReadOnly
        );
    }
    assert_eq!(
        authorize_bearers(true, false, &policy, Some("Bearer impostor"), 1).decision,
        AccessDecision::Unauthorized
    );
    assert_eq!(
        authorize_bearers(false, false, &policy, Some("bad header"), 1).generation,
        Some(AuthGeneration::NotRequired)
    );
    let open = BearerPolicy::new(None, None, None, 1)
        .unwrap()
        .with_named(registry);
    assert_eq!(
        authorize_bearers(true, false, &open, None, 1).generation,
        Some(AuthGeneration::OpenWrite)
    );
    assert_eq!(
        authorize_bearers(true, false, &open, Some("Bearer alice-secret"), 1).generation,
        Some(AuthGeneration::Named)
    );
}

#[test]
fn shared_credential_cannot_be_relabelled_as_a_named_principal() {
    use crate::http_auth::{AuthGeneration, BearerPolicy, authorize_bearers};
    let registry = parse(&[credential("a", "urn:crew:alice", "shared")]).unwrap();
    let policy = BearerPolicy::new(Some("shared".into()), None, None, 1)
        .unwrap()
        .with_named(registry);
    assert_eq!(
        authorize_bearers(true, false, &policy, Some("Bearer shared"), 1).generation,
        Some(AuthGeneration::Current)
    );
}

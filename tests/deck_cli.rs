//! `quipu deck` must let an operator take a Decision Deck from nothing to one
//! enrolled, enabled credential, and back out again, without raw SQL
//! (aegis-jjy). Before these verbs the native methods had only test callers.
#![cfg(feature = "shacl")]

use std::process::{Command, Output};

/// An Ed25519 `COSE_Key`: `{1: 1 (OKP), 3: -8 (EdDSA), -1: 6 (Ed25519), -2: x}`.
fn cose_key_b64url() -> String {
    use base64::Engine as _;
    let mut bytes = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
    bytes.extend(1..=32u8);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn deck(db: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quipu"))
        .arg("deck")
        .args(args)
        .arg("--db")
        .arg(db)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn far_future() -> String {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 86_400)
        .to_string()
}

#[test]
fn configure_enroll_enable_status_and_revoke_through_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("deck.db");

    // Nothing configured: status says so, and the switch has nothing to flip.
    assert!(stdout(&deck(&db, &["status"])).contains("not configured"));
    let early = deck(&db, &["enable"]);
    assert_eq!(early.status.code(), Some(1), "{early:?}");

    let configured = deck(
        &db,
        &[
            "configure",
            "--origin",
            "https://deck.example.org",
            "--rp-id",
            "deck.example.org",
            "--actor",
            "operator@example.org",
            "--author",
            "curator@example.org",
        ],
    );
    assert!(configured.status.success(), "configure: {configured:?}");
    assert!(stdout(&configured).contains("DISABLED"));

    let expires = far_future();
    let key = cose_key_b64url();
    let enrolled = deck(
        &db,
        &[
            "enroll",
            "--credential-id",
            "Y3JlZGVudGlhbC0x",
            "--cose-key",
            &key,
            "--scheme",
            "webauthn-eddsa",
            "--expires-at",
            &expires,
        ],
    );
    assert!(enrolled.status.success(), "enroll: {enrolled:?}");
    let fingerprint = stdout(&enrolled)
        .split("fingerprint ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .to_string();
    // Golden: computed independently (Python hashlib over the same bytes), so
    // the enrollment page has a fixed value to reproduce.
    assert_eq!(fingerprint, "9bb1-7e67-b268-ca8b");

    assert!(deck(&db, &["enable"]).status.success());
    let status = stdout(&deck(&db, &["status"]));
    assert!(status.contains("enabled=true"), "{status}");
    assert!(
        status.contains("origin=https://deck.example.org"),
        "{status}"
    );
    // The operator sees the SAME fingerprint enrollment printed.
    assert!(
        status.contains(&format!(
            "Y3JlZGVudGlhbC0x\tscheme=webauthn-eddsa\tfingerprint={fingerprint}"
        )),
        "{status}"
    );
    assert!(status.contains("revoked=false"), "{status}");

    let revoked = deck(&db, &["revoke", "Y3JlZGVudGlhbC0x"]);
    assert!(revoked.status.success(), "revoke: {revoked:?}");
    assert!(stdout(&deck(&db, &["status"])).contains("revoked=true"));

    assert!(deck(&db, &["disable"]).status.success());
    assert!(stdout(&deck(&db, &["status"])).contains("enabled=false"));
}

#[test]
fn refusals_exit_nonzero_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("deck.db");
    let configure = |origin: &str| {
        deck(
            &db,
            &[
                "configure",
                "--origin",
                origin,
                "--rp-id",
                "deck.example.org",
                "--actor",
                "operator@example.org",
                "--author",
                "curator@example.org",
            ],
        )
    };
    // An origin that is not exactly https://<rp-id> is refused by the native check.
    assert_eq!(configure("http://deck.example.org").status.code(), Some(1));
    assert!(stdout(&deck(&db, &["status"])).contains("not configured"));
    assert!(configure("https://deck.example.org").status.success());
    // The scope is immutable: a second configure is refused.
    assert_eq!(configure("https://deck.example.org").status.code(), Some(1));

    let expires = far_future();
    let enroll = |key: &str, scheme: &str| {
        deck(
            &db,
            &[
                "enroll",
                "--credential-id",
                "Y3JlZGVudGlhbC0x",
                "--cose-key",
                key,
                "--scheme",
                scheme,
                "--expires-at",
                &expires,
            ],
        )
    };
    // A key that does not match its scheme, and a key that is not COSE.
    assert_eq!(
        enroll(&cose_key_b64url(), "webauthn-es256").status.code(),
        Some(1)
    );
    assert_eq!(enroll("AAAA", "webauthn-eddsa").status.code(), Some(1));
    assert!(stdout(&deck(&db, &["status"])).contains("no enrolled credentials"));

    // Missing flags and an unknown credential are usage errors and failures.
    assert_eq!(deck(&db, &["enroll"]).status.code(), Some(2));
    assert_eq!(deck(&db, &["revoke"]).status.code(), Some(2));
    assert_eq!(deck(&db, &["revoke", "bm9uZQ"]).status.code(), Some(1));
    assert_eq!(deck(&db, &[]).status.code(), Some(2));
}

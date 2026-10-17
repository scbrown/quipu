use super::*;
use crate::config::EndpointPendant;
use crate::share::{PointerTarget, ShareDestination, ShareOptions, SharePointer};

fn declaration() -> (tempfile::TempDir, FederationConfig) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = crate::Store::open_in_memory().unwrap();
    crate::share_scrub::seed_test_catalogue(&mut store);
    let path = dir.path().join("declaration.pendant");
    crate::share::share(
        &store,
        path.to_str().unwrap(),
        &ShareOptions {
            no_shapes: true,
            destination: ShareDestination::Internal,
            pointers: vec![
                SharePointer {
                    graph: "urn:remote".into(),
                    target: PointerTarget::Endpoint {
                        endpoint: "https://example.org/sparql/".into(),
                    },
                },
                SharePointer {
                    graph: "urn:pack".into(),
                    target: PointerTarget::Pack {
                        url: "https://example.org/pack".into(),
                        sha256: "a".repeat(64),
                    },
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    let cfg = FederationConfig {
        pendants: vec![EndpointPendant {
            path,
            repo_remote: "https://github.com/foreign/project.git".into(),
        }],
        ..Default::default()
    };
    (dir, cfg)
}

#[test]
fn exact_verified_endpoint_requires_current_consent_and_has_no_authority() {
    let (_dir, cfg) = declaration();
    let mut calls = 0;
    let remote = cfg
        .pointer_endpoint_with("https://example.org/sparql/", None, |repo, endpoint| {
            assert_eq!(repo, "https://github.com/foreign/project.git");
            assert_eq!(endpoint, "https://example.org/sparql/");
            calls += 1;
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(remote.name, "urn:remote");
    assert_eq!(remote.url, "https://example.org/sparql/");
    assert!(remote.auth_token.is_none());
    assert!(remote.trust.is_none());
    assert!(remote.freshness.is_none());
    let err = cfg
        .pointer_endpoint_with("https://example.org/sparql/", None, |_, _| {
            Err(Error::InvalidValue("revoked".into()))
        })
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("revoked")
            && err.contains("github.com/foreign/project")
            && err.contains("https://example.org/sparql/"),
        "{err}"
    );
}

#[test]
fn url_neighbours_and_pack_pointers_never_call_policy_or_fetch() {
    let (_dir, cfg) = declaration();
    for url in [
        "https://example.org/sparql",
        "https://example.org/sparql/query",
        "https://example.org/pack",
        "https://other.example/query",
    ] {
        assert!(
            cfg.pointer_endpoint_with(url, None, |_, _| panic!("not declared"))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn forged_declaration_is_refused_before_trust() {
    let (_dir, cfg) = declaration();
    let path = cfg.pendants[0].path.join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["pointers"][0]["endpoint"] = "https://attacker.example/query".into();
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let err = cfg
        .pointer_endpoint_with("https://attacker.example/query", None, |_, _| {
            panic!("unverified")
        })
        .unwrap_err()
        .to_string();
    assert!(err.contains("share id mismatch"), "{err}");
}

#[test]
fn config_defaults_to_no_pointer_declarations() {
    let cfg: FederationConfig = toml::from_str("").unwrap();
    assert!(cfg.pendants.is_empty());
    let cfg: FederationConfig = toml::from_str("[[pendants]]\npath='local.pendant'\nrepo_remote='https://github.com/foreign/project.git'\n").unwrap();
    assert_eq!(cfg.pendants.len(), 1);
    assert!(
        toml::from_str::<FederationConfig>(
            "[[pendants]]\npath='local.pendant'\nrepo_remote='repo'\ntrust=true"
        )
        .is_err()
    );
}

#[test]
fn expired_query_never_reads_a_declaration_or_calls_policy() {
    let cfg = FederationConfig {
        pendants: vec![EndpointPendant {
            path: "missing-declaration.pendant".into(),
            repo_remote: "https://github.com/foreign/project.git".into(),
        }],
        ..Default::default()
    };
    let err = cfg
        .pointer_endpoint_with(
            "https://example.org/sparql/",
            Some(Deadline::after_millis(0)),
            |_, _| panic!("expired query reached policy"),
        )
        .unwrap_err();
    assert!(matches!(err, Error::RequestTimeout { .. }), "{err}");
}

#[test]
fn consent_that_finishes_after_the_query_deadline_is_not_admitted() {
    let (_dir, cfg) = declaration();
    let mut called = false;
    let err = cfg
        .pointer_endpoint_with(
            "https://example.org/sparql/",
            Some(Deadline::after_millis(100)),
            |_, _| {
                called = true;
                std::thread::sleep(std::time::Duration::from_millis(150));
                Ok(())
            },
        )
        .unwrap_err();
    assert!(called, "positive policy arm was not reached");
    assert!(matches!(err, Error::RequestTimeout { .. }), "{err}");
}

#[cfg(unix)]
#[test]
fn query_deadline_kills_and_reaps_the_policy_process() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let child = Command::new("sh")
        .args(["-c", "exec sleep 10"])
        .spawn()
        .unwrap();
    let pid = child.id().to_string();
    let alive = || {
        Command::new("kill")
            .args(["-0", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    assert!(alive(), "positive child visibility control failed");
    let started = Instant::now();
    let err = wait_for_policy(child, Some(Deadline::after_millis(20))).unwrap_err();
    assert!(matches!(err, Error::RequestTimeout { .. }), "{err}");
    assert!(started.elapsed() < Duration::from_millis(120));
    assert!(!alive(), "policy process survived the timed-out wait");
}

#[cfg(unix)]
#[test]
fn policy_wait_without_a_query_deadline_preserves_exit_status() {
    for code in [0, 3, 2] {
        let child = std::process::Command::new("sh")
            .args(["-c", &format!("exit {code}")])
            .spawn()
            .unwrap();
        assert_eq!(wait_for_policy(child, None).unwrap().code(), Some(code));
    }
}

use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};

fn fixture() -> (tempfile::TempDir, String, QueryCaptureConfig, Value, Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.db");
    std::fs::write(&path, b"owned private fixture").unwrap();
    let query = format!("ASK {{}} #{}", "x".repeat(3987));
    assert_eq!(query.len(), 3995);
    let hash = format!("{:x}", Sha256::digest(query.as_bytes()));
    let now = quipu::time::epoch_secs();
    let config = QueryCaptureConfig {
        enabled: true,
        query_sha256: hash.clone(),
        query_bytes: query.len(),
        window_id: "private-trial".into(),
        start_epoch_secs: now,
        end_epoch_secs: now + 30,
    };
    let input = json!({"query":query,"graph":"urn:owned:test","authorization":"DO_NOT_CAPTURE"});
    let identity =
        json!({"query_sha256":hash,"query_bytes":3995,"server_process_id":1,"request_id":7});
    (dir, path.to_str().unwrap().into(), config, input, identity)
}

fn result(dir: &std::path::Path) -> Value {
    let path = dir.join("query-captures/private-trial.json");
    for _ in 0..200 {
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            return value;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("private worker did not complete");
}

#[test]
fn capture_disabled_has_no_file_side_effects() {
    let (dir, path, mut config, _, _) = fixture();
    config.enabled = false;
    assert!(start(None, &path).is_none());
    assert!(start(Some(&config), &path).is_none());
    assert!(!dir.path().join("query-captures").exists());
    assert!(
        quipu::config::ServerConfig::default()
            .query_capture
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn capture_exact_post_once_without_headers_or_extra_input() {
    let (dir, path, config, input, identity) = fixture();
    let capture = start(Some(&config), &path).unwrap();
    capture.observe(&input, &identity, "seeds", false);
    capture.observe(&input, &identity, "agent-adhoc", true);
    let mut wrong = identity.clone();
    wrong["query_sha256"] = json!("f".repeat(64));
    capture.observe(&input, &wrong, "seeds", true);
    assert!(!capture.consumed.load(Ordering::Acquire));
    capture.observe(&input, &identity, "seeds", true);
    let first = result(dir.path());
    assert_eq!(first["query"], input["query"]);
    assert_eq!(first["identity"], identity);
    assert_eq!(first["omitted_input_fields"], true);
    assert!(!first.to_string().contains("DO_NOT_CAPTURE"));
    capture.observe(&json!({"query":"replacement"}), &identity, "seeds", true);
    assert_eq!(result(dir.path()), first);
    assert!(
        start(Some(&config), &path).is_none(),
        "restart cannot reuse claim"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.path().join("query-captures/private-trial.json");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn capture_record_budget_refuses_without_raw_prefix_or_retry() {
    let (dir, path, config, mut input, identity) = fixture();
    input["graph"] = json!("g".repeat(MAX_RECORD * 2));
    let capture = start(Some(&config), &path).unwrap();
    capture.observe(&input, &identity, "seeds", true);
    assert_eq!(result(dir.path()), json!({"outcome":"record_refused"}));
    assert!(capture.consumed.load(Ordering::Acquire));
}

#[test]
fn capture_absolute_window_and_selector_validation() {
    let (_, _, mut config, _, _) = fixture();
    let now = quipu::time::epoch_secs();
    assert!(validate(&config, now).is_ok());
    assert!(validate(&config, config.end_epoch_secs).is_err());
    config.end_epoch_secs = config.start_epoch_secs + 301;
    assert!(validate(&config, now).is_err());
    config.end_epoch_secs = now + 30;
    config.window_id = "../escape".into();
    assert!(validate(&config, now).is_err());
}

#[cfg(unix)]
#[test]
fn capture_refuses_symlink_directory_and_broad_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (dir, path, config, _, _) = fixture();
    let target = dir.path().join("other");
    std::fs::create_dir(&target).unwrap();
    let destination = dir.path().join("query-captures");
    symlink(&target, &destination).unwrap();
    assert!(start(Some(&config), &path).is_none());
    std::fs::remove_file(&destination).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(start(Some(&config), &path).is_none());
}

#[test]
fn capture_future_and_monotonic_expiry_do_not_consume() {
    let (_, _, mut config, input, identity) = fixture();
    let (sender, _receiver) = mpsc::sync_channel(1);
    let now = quipu::time::epoch_secs();
    config.start_epoch_secs = now + 60;
    config.end_epoch_secs = now + 90;
    let mut capture = Capture {
        config,
        starts: Instant::now(),
        expires: Instant::now() + Duration::from_secs(90),
        consumed: AtomicBool::new(false),
        sender,
    };
    capture.observe(&input, &identity, "seeds", true);
    assert!(!capture.consumed.load(Ordering::Acquire));
    capture.config.start_epoch_secs = now;
    capture.starts = Instant::now() + Duration::from_secs(60);
    capture.observe(&input, &identity, "seeds", true);
    assert!(
        !capture.consumed.load(Ordering::Acquire),
        "wall-clock advance cannot start the monotonic window early"
    );
    capture.starts = Instant::now();
    capture.expires = Instant::now() - Duration::from_secs(1);
    capture.observe(&input, &identity, "seeds", true);
    assert!(!capture.consumed.load(Ordering::Acquire));
}

#[cfg(unix)]
#[tokio::test]
async fn capture_preserves_direct_query_response() {
    use axum::{Json, extract::State, http::HeaderMap};
    let (dir, path, config, input, _) = fixture();
    async fn run(input: Value, capture: Option<Arc<Capture>>) -> Value {
        let mut handle =
            super::super::StoreHandle::writer_only(quipu::Store::open_in_memory().unwrap());
        handle.query_capture = capture;
        let mut headers = HeaderMap::new();
        headers.insert("x-quipu-client", "seeds".parse().unwrap());
        let response =
            super::super::query_endpoint::query(State(Arc::new(handle)), headers, Json(input))
                .await
                .unwrap();
        assert!(response.status().is_success());
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }
    let before = run(input.clone(), None).await;
    let after = run(input.clone(), start(Some(&config), &path)).await;
    assert_eq!(before, after);
    assert_eq!(result(dir.path())["query"], input["query"]);
}

#[cfg(unix)]
#[test]
#[ignore = "explicit private demonstration; no network or production store"]
fn query_capture_private_demo() {
    let (dir, path, mut config, input, identity) = fixture();
    config.enabled = false;
    assert!(start(Some(&config), &path).is_none());
    assert!(!dir.path().join("query-captures").exists());
    println!("BEFORE: same synthetic3995-byte query, capture disabled: no worker/artifact");
    config.enabled = true;
    let capture = start(Some(&config), &path).unwrap();
    capture.observe(&input, &identity, "seeds", true);
    let record = result(dir.path());
    assert_eq!(record["query"], input["query"]);
    assert!(!record.to_string().contains("DO_NOT_CAPTURE"));
    let bytes = std::fs::metadata(dir.path().join("query-captures/private-trial.json"))
        .unwrap()
        .len();
    assert!(bytes <= MAX_RECORD as u64);
    println!(
        "AFTER: exact query retained, private0600 artifact {bytes}bytes <=16384; secret extra excluded"
    );
    assert!(start(Some(&config), &path).is_none());
    println!("CONTROL: restart with same nonce refused; no second specimen");
}

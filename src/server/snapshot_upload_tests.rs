use super::*;

fn digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}

fn store() -> SharedStore {
    Arc::new(super::super::StoreHandle::writer_only(
        quipu::Store::open_in_memory().unwrap(),
    ))
}

struct HeldWriter {
    release: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for HeldWriter {
    fn drop(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}
fn hold_writer(store: SharedStore) -> HeldWriter {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _writer = store.writer.lock();
        ready_tx.send(()).unwrap();
        wait.recv().unwrap();
    });
    ready_rx.recv().unwrap();
    HeldWriter {
        release: Some(release),
        thread: Some(thread),
    }
}

async fn terminal(store: &SharedStore, id: &str) -> JsonValue {
    for _ in 0..100 {
        let result = promote(
            State(store.clone()),
            Json(serde_json::json!({"upload_id":id})),
        )
        .await
        .unwrap()
        .0;
        if result["pending"] != true {
            return result;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("cancelled queued worker did not exit within one second");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_exits_writer_queue_without_taking_writer() {
    let store = store();
    let writer = hold_writer(store.clone());
    let input = serde_json::json!({"upload_id":"queued-cancel"});
    assert_eq!(
        promote(State(store.clone()), Json(input.clone()))
            .await
            .unwrap()
            .0["pending"],
        true
    );
    assert_eq!(
        cancel(State(store.clone()), Json(input)).await.unwrap().0["cancellation_requested"],
        true
    );
    let result = terminal(&store, "queued-cancel").await;
    assert_eq!(result["cancelled"], true);
    drop(writer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_lease_expires_and_poll_does_not_restart_it() {
    let store = store();
    let writer = hold_writer(store.clone());
    let _ = promote_with_lease(
        store.clone(),
        serde_json::json!({"upload_id":"abandoned"}),
        Duration::from_millis(5),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(terminal(&store, "abandoned").await["cancelled"], true);
    assert_eq!(terminal(&store, "abandoned").await["cancelled"], true);
    drop(writer);
}

#[test]
fn guessed_upload_cannot_be_cancelled_by_another_identity() {
    let store = store();
    let identity = |name: &str| {
        Some(Identity {
            principal: name.into(),
            credential_id: Some(name.into()),
            auth_class: "named".into(),
            attestation: Default::default(),
        })
    };
    let p = Promotion {
        store: Arc::downgrade(&store),
        owner: identity("owner"),
        control: PromotionControl::new(LEASE),
        state: PromotionState::Running,
        touched: Instant::now(),
    };
    assert!(check_owner(&p, &identity("other")).is_err());
    assert!(check_owner(&p, &None).is_err());
    check_owner(&p, &identity("owner")).unwrap();
    assert!(!p.control.cancelled());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normal_promotion_commits_and_cancel_reports_complete() {
    let store = store();
    let turtle = "<http://ex/control> <http://www.w3.org/2000/01/rdf-schema#label> \"control\" .";
    let hash = digest(turtle.as_bytes());
    let id = snapshot_upload::snapshot_upload_id("control", &hash);
    let _ = stage_part(
        State(store.clone()),
        Json(serde_json::json!({
            "upload_id":id,"snapshot":"control","content_hash":hash,"part_hash":hash,
            "payload":turtle,"part_number":0,"total_parts":1,"total_bytes":turtle.len(),
        })),
    )
    .await
    .unwrap();
    let input = serde_json::json!({"upload_id":id});
    let _ = promote(State(store.clone()), Json(input.clone()))
        .await
        .unwrap();
    assert_eq!(terminal(&store, &id).await["promoted"], true);
    let result = cancel(State(store.clone()), Json(input)).await.unwrap().0;
    assert_eq!(result["cancellation_requested"], false);
    assert_eq!(result["state"], "complete");
}

/// Bounded scratch measurement; run individually with a process memory/time
/// limit. It never contacts a service or uses a production database.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit isolated promotion resource probe"]
async fn promotion_resource_probe() {
    let store = store();
    let n: usize = std::env::var("PROMOTION_PROBE_ROWS")
        .unwrap_or_else(|_| "5000".into())
        .parse()
        .unwrap();
    assert!((1..=100_000).contains(&n));
    let stop = std::env::var("PROMOTION_PROBE_STOP").is_ok_and(|v| v == "1");
    let turtle: String = (0..n)
        .map(|n| {
            format!(
                "<http://ex/probe/{n}> <http://www.w3.org/2000/01/rdf-schema#label> \"row {n}\" .\n"
            )
        })
        .collect();
    let hash = digest(turtle.as_bytes());
    let id = snapshot_upload::snapshot_upload_id("probe", &hash);
    let parts: Vec<_> = turtle.as_bytes().chunks(256 * 1024).collect();
    for (number, bytes) in parts.iter().enumerate() {
        let _ = stage_part(
            State(store.clone()),
            Json(serde_json::json!({
                "upload_id":id,"snapshot":"probe","content_hash":hash,
                "part_hash":digest(bytes),"payload":std::str::from_utf8(bytes).unwrap(),
                "part_number":number,"total_parts":parts.len(),"total_bytes":turtle.len(),
            })),
        )
        .await
        .unwrap();
    }
    fn memory(key: &str) -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find(|line| line.starts_with(key))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }
    let before = memory("VmRSS:");
    let started = Instant::now();
    let _ = promote_with_lease(
        store.clone(),
        serde_json::json!({"upload_id":id}),
        if stop {
            Duration::from_millis(50)
        } else {
            LEASE
        },
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            // Deliberately do not poll/renew the abandoned producer's lease.
            let result = {
                let states = promotions().lock().unwrap();
                response(&id, states.get(&key(&store, &id)).unwrap())
                    .unwrap()
                    .0
            };
            if result["pending"] != true {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("promotion exceeded bounded scratch probe timeout");
    if stop {
        assert_eq!(result["cancelled"], true);
    } else {
        assert_eq!(result["promoted"], true);
    }
    eprintln!(
        "PROMOTION_PROBE {}",
        serde_json::json!({
            "rows":n,"bytes":turtle.len(),"stop":stop,"elapsed_ms":started.elapsed().as_millis(),
            "rss_before_kib":before,"rss_after_kib":memory("VmRSS:"),"peak_kib":memory("VmHWM:"),
            "outcome":result,
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_cancel_needs_explicit_same_owner_resume() {
    let store = store();
    let turtle = "<http://ex/resume> <http://www.w3.org/2000/01/rdf-schema#label> \"resume\" .";
    let hash = digest(turtle.as_bytes());
    let id = snapshot_upload::snapshot_upload_id("resume", &hash);
    let _ = stage_part(
        State(store.clone()),
        Json(serde_json::json!({
            "upload_id":id,"snapshot":"resume","content_hash":hash,"part_hash":hash,
            "payload":turtle,"part_number":0,"total_parts":1,"total_bytes":turtle.len(),
        })),
    )
    .await
    .unwrap();
    let writer = hold_writer(store.clone());
    let input = serde_json::json!({"upload_id":id});
    let _ = promote(State(store.clone()), Json(input.clone()))
        .await
        .unwrap();
    let _ = cancel(State(store.clone()), Json(input.clone()))
        .await
        .unwrap();
    assert_eq!(terminal(&store, &id).await["cancelled"], true);
    drop(writer);
    assert_eq!(
        promote(State(store.clone()), Json(input)).await.unwrap().0["cancelled"],
        true
    );
    let writer = hold_writer(store.clone());
    let other = serde_json::json!({"upload_id":"other-queued"});
    let _ = promote(State(store.clone()), Json(other.clone()))
        .await
        .unwrap();
    assert!(
        promote(
            State(store.clone()),
            Json(serde_json::json!({"upload_id":id,"retry_cancelled":true}))
        )
        .await
        .is_err()
    );
    assert_eq!(
        terminal(&store, &id).await["cancelled"],
        true,
        "failed admission must retain the cancellation receipt"
    );
    let _ = cancel(State(store.clone()), Json(other)).await.unwrap();
    assert_eq!(terminal(&store, "other-queued").await["cancelled"], true);
    drop(writer);
    let _ = promote(
        State(store.clone()),
        Json(serde_json::json!({"upload_id":id,"retry_cancelled":true})),
    )
    .await
    .unwrap();
    assert_eq!(terminal(&store, &id).await["promoted"], true);
}

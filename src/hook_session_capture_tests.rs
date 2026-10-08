//! Every guard arm of the session-capture hook, against a temp state dir.
//! Nothing here reads or writes the real HOME: `Settings` is built directly.

use super::*;

fn settings(dir: &Path) -> Settings {
    Settings {
        crew: None,
        scope: "*".into(),
        state_dir: dir.to_path_buf(),
        server: "http://quipu.example:3030".into(),
        group: "test-group".into(),
    }
}

fn stop(session: &str, cwd: &str, active: bool) -> String {
    json!({"session_id": session, "cwd": cwd, "stop_hook_active": active}).to_string()
}

const CREW_CWD: &str = "/work/rig/crew/alpha/src";

fn log_lines(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("solicit-log.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn happy_path_blocks_once_with_the_payload_in_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let s = settings(tmp.path());
    let resp = respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).unwrap();
    // Round-trip through text: what is printed must parse as JSON.
    let back: Value = serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
    assert_eq!(back["decision"], "block");
    let reason = back["reason"].as_str().unwrap();
    assert!(!reason.is_empty());
    assert!(reason.contains("http://quipu.example:3030/episode"));
    assert!(reason.contains("group test-group"));
    assert!(reason.contains("(s1)"), "session id is named");
    assert!(reason.contains("X-Quipu-Client: session-capture"));
    assert!(reason.contains("/propose"));
    assert_eq!(back["systemMessage"], NOTE);
    assert!(tmp.path().join("solicited-s1").exists());
    let log = log_lines(tmp.path());
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["session_id"], "s1");
    assert_eq!(log[0]["crew"], "alpha");
    assert!(log[0]["ts"].as_str().unwrap().ends_with('Z'));
}

#[test]
fn an_unidentified_crew_is_silent_and_claims_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let s = settings(tmp.path());
    assert!(respond(&stop("s1", "/work/elsewhere", false), &s, SystemTime::now()).is_none());
    assert!(respond(&stop("s1", "", false), &s, SystemTime::now()).is_none());
    assert!(!tmp.path().join("solicited-s1").exists());
    assert!(log_lines(tmp.path()).is_empty());
}

#[test]
fn gt_crew_identifies_the_crew_without_a_crew_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = settings(tmp.path());
    s.crew = Some("beta".into());
    assert!(respond(&stop("s1", "/work/elsewhere", false), &s, SystemTime::now()).is_some());
    assert_eq!(log_lines(tmp.path())[0]["crew"], "beta");
}

#[test]
fn a_crew_outside_the_scope_list_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = settings(tmp.path());
    s.scope = "beta gamma".into();
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_none());
    assert!(log_lines(tmp.path()).is_empty());
    s.scope = "beta alpha".into();
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_some());
}

#[test]
fn stop_hook_active_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = settings(tmp.path());
    assert!(respond(&stop("s1", CREW_CWD, true), &s, SystemTime::now()).is_none());
    assert!(
        !tmp.path().join("solicited-s1").exists(),
        "marker not burned"
    );
}

#[test]
fn a_second_stop_in_the_same_session_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = settings(tmp.path());
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_some());
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_none());
    assert!(respond(&stop("s2", CREW_CWD, false), &s, SystemTime::now()).is_some());
    assert_eq!(
        log_lines(tmp.path()).len(),
        2,
        "one log line per solicitation"
    );
}

#[test]
fn an_unwritable_log_fails_closed_with_no_output() {
    let tmp = tempfile::tempdir().unwrap();
    // A directory where the log file should be: the append cannot open it.
    std::fs::create_dir(tmp.path().join("solicit-log.jsonl")).unwrap();
    let s = settings(tmp.path());
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_none());
    // The marker was claimed first, so the session stays unsolicited AND
    // uncounted rather than retrying on the next stop.
    assert!(tmp.path().join("solicited-s1").exists());
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_none());
}

#[test]
fn an_unusable_state_dir_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, "").unwrap();
    let s = settings(&file);
    assert!(respond(&stop("s1", CREW_CWD, false), &s, SystemTime::now()).is_none());
}

#[test]
fn malformed_input_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = settings(tmp.path());
    assert!(respond("not json", &s, SystemTime::now()).is_none());
    assert!(respond("", &s, SystemTime::now()).is_none());
}

#[test]
fn old_markers_are_reaped_and_recent_ones_and_the_log_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let now = SystemTime::now();
    let old = std::fs::File::create(dir.join("solicited-old")).unwrap();
    old.set_modified(now - Duration::from_secs(3 * 86_400))
        .unwrap();
    drop(old);
    std::fs::File::create(dir.join("solicited-recent")).unwrap();
    // An old log must never be reaped: it is the durable denominator.
    let log = std::fs::File::create(dir.join("solicit-log.jsonl")).unwrap();
    log.set_modified(now - Duration::from_secs(30 * 86_400))
        .unwrap();
    drop(log);

    assert!(respond(&stop("s1", CREW_CWD, false), &settings(dir), now).is_some());
    assert!(!dir.join("solicited-old").exists(), "old marker reaped");
    assert!(dir.join("solicited-recent").exists());
    assert!(dir.join("solicited-s1").exists());
    assert!(dir.join("solicit-log.jsonl").exists());
}

#[test]
fn a_session_id_cannot_escape_the_state_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let s = settings(&state);
    assert!(respond(&stop("../../evil", CREW_CWD, false), &s, SystemTime::now()).is_some());
    assert!(state.join("solicited-.._.._evil").exists());
    assert!(!tmp.path().join("evil").exists());
}

#[test]
fn crew_is_derived_from_the_innermost_crew_segment() {
    assert_eq!(crew_from_cwd("/a/crew/alpha"), Some("alpha".into()));
    assert_eq!(crew_from_cwd("/a/crew/alpha/b/c"), Some("alpha".into()));
    assert_eq!(crew_from_cwd("/a/crew/"), None);
    assert_eq!(crew_from_cwd("/a/crewless/x"), None);
}

#[test]
fn the_message_carries_no_deployment_specifics_beyond_its_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let msg = message(&settings(tmp.path()), "sid");
    // Spelled in pieces so this file does not itself trip the repo's scrubbers.
    for needle in [
        concat!(".", "svc"),
        concat!(".", "lan"),
        "/home/",
        concat!("192", ".168."),
    ] {
        assert!(!msg.contains(needle), "message leaks {needle}");
    }
    assert!(msg.contains("QUIPU_AUTH_TOKEN"));
    assert!(msg.contains("never print"));
}

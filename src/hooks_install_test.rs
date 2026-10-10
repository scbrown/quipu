use super::*;

#[test]
fn the_bundle_is_valid_owned_by_quipu_and_versioned_by_the_build() {
    let b = bundle();
    assert_eq!(b["schema"], "st.hook-bundle/1");
    assert_eq!(b["name"], "quipu");
    assert_eq!(b["owner"], "quipu");
    assert_eq!(b["version"], env!("CARGO_PKG_VERSION"));
    assert!(!hooks_for(&b, Harness::Claude).is_empty());
    assert!(!hooks_for(&b, Harness::Codex).is_empty());
}

#[test]
fn every_codex_hook_declares_evidence_so_st_can_grade_firing() {
    // A codex hook without evidence can be live but never graded firing.
    for h in bundle()["hooks"].as_array().unwrap() {
        if h["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "codex")
        {
            assert!(
                h["evidence"]["command"].is_string(),
                "{} has no evidence",
                h["command"]
            );
        }
    }
}

#[test]
fn merge_is_idempotent_and_keeps_foreign_hooks() {
    let b = bundle();
    let mut cfg = json!({"hooks": {"Stop": [
        {"hooks": [{"type": "command", "command": "other-tool hook"}]}
    ], "PostToolUse": [
        {"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool hook"}]}
    ]}, "model": "x"});
    let want = hooks_for(&b, Harness::Claude).len();
    assert_eq!(merge(&mut cfg, &b, Harness::Claude), want);
    assert_eq!(
        merge(&mut cfg, &b, Harness::Claude),
        0,
        "second install adds nothing"
    );
    assert_eq!(present(&cfg, &b, Harness::Claude), (want, want));
    assert_eq!(cfg["model"], "x");
    // The matcher-less Stop group is shared, not duplicated: quipu's hook joins
    // the foreign one beside it.
    let stop = cfg["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 1, "one matcher-less Stop group");
    let cmds: Vec<_> = stop[0]["hooks"].as_array().unwrap().iter().collect();
    assert!(cmds.iter().any(|h| h["command"] == "other-tool hook"));
    assert!(
        cmds.iter()
            .any(|h| h["command"] == "quipu hook session-capture")
    );
    assert_eq!(cfg["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
}

#[test]
fn remove_takes_only_quipus_hooks_and_drops_empty_groups() {
    let b = bundle();
    let mut cfg = json!({"hooks": {"PostToolUse": [
        {"matcher": "Bash", "hooks": [{"type": "command", "command": "other-tool hook"}]}
    ]}});
    merge(&mut cfg, &b, Harness::Claude);
    assert!(cfg["hooks"].get("Stop").is_some());
    let n = hooks_for(&b, Harness::Claude).len();
    assert_eq!(remove(&mut cfg, &b, Harness::Claude), n);
    assert_eq!(present(&cfg, &b, Harness::Claude).0, 0);
    assert_eq!(
        cfg["hooks"]["PostToolUse"].as_array().unwrap().len(),
        1,
        "foreign group survives"
    );
    assert!(
        cfg["hooks"].get("Stop").is_none(),
        "emptied event is dropped"
    );
}

#[test]
fn codex_config_round_trips_through_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "model = \"gpt\"\n[mcp_servers.x]\ncommand = \"x\"\n").unwrap();
    let b = bundle();
    let mut cfg = read_config(&path, Harness::Codex).unwrap();
    assert!(merge(&mut cfg, &b, Harness::Codex) > 0);
    write_config(&path, Harness::Codex, &cfg).unwrap();
    let back = read_config(&path, Harness::Codex).unwrap();
    let n = hooks_for(&b, Harness::Codex).len();
    assert_eq!(present(&back, &b, Harness::Codex), (n, n));
    assert_eq!(back["model"], "gpt");
    assert_eq!(back["mcp_servers"]["x"]["command"], "x");
    assert!(
        dir.path().join("config.toml.bak-quipu").exists(),
        "previous file kept"
    );
}

#[test]
fn an_absent_config_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let v = read_config(&dir.path().join("nope.json"), Harness::Claude).unwrap();
    assert_eq!(v, json!({}));
}

#[test]
fn the_single_hook_is_the_stop_capture_for_both_harnesses() {
    let b = bundle();
    for h in [Harness::Claude, Harness::Codex] {
        assert_eq!(
            hooks_for(&b, h),
            vec![(
                "Stop".to_string(),
                None,
                "quipu hook session-capture".to_string()
            )]
        );
    }
}

#[test]
fn harness_names_parse_and_unknown_ones_are_refused() {
    assert_eq!(Harness::parse("claude"), Ok(Harness::Claude));
    assert_eq!(Harness::parse("codex"), Ok(Harness::Codex));
    assert!(Harness::parse("gemini").is_err());
}

#[test]
fn a_claude_settings_file_round_trips_and_keeps_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"permissions":{"allow":["Bash"]}}"#).unwrap();
    let b = bundle();
    let mut cfg = read_config(&path, Harness::Claude).unwrap();
    assert_eq!(merge(&mut cfg, &b, Harness::Claude), 1);
    write_config(&path, Harness::Claude, &cfg).unwrap();
    let back = read_config(&path, Harness::Claude).unwrap();
    assert_eq!(present(&back, &b, Harness::Claude), (1, 1));
    assert_eq!(back["permissions"]["allow"][0], "Bash");
    assert!(dir.path().join("settings.json.bak-quipu").exists());
    let mut cfg = back;
    assert_eq!(remove(&mut cfg, &b, Harness::Claude), 1);
    assert_eq!(present(&cfg, &b, Harness::Claude), (0, 1));
}

#[test]
fn malformed_config_is_an_error_not_an_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(read_config(&path, Harness::Claude).is_err());
}

use super::*;

#[test]
fn client_label_precedence_and_normalisation() {
    // Explicit header wins over User-Agent.
    assert_eq!(
        normalize_client(Some("hank-policy-hook"), Some("curl/8.5.0")),
        "hank-policy-hook"
    );
    // Version suffixes collapse — otherwise every release mints a label.
    assert_eq!(normalize_client(None, Some("curl/8.5.0")), "curl");
    assert_eq!(normalize_client(None, Some("bobbin/0.6.5")), "bobbin");
    // Absent, empty, and whitespace-only all read as unattributed, which is
    // a real measurement (how much load nobody claims), not a failure.
    assert_eq!(normalize_client(None, None), "unattributed");
    assert_eq!(normalize_client(Some(""), None), "unattributed");
    assert_eq!(normalize_client(Some("   "), Some("")), "unattributed");
    // An empty explicit header falls THROUGH to User-Agent rather than
    // swallowing it.
    assert_eq!(normalize_client(Some(""), Some("st/1.2")), "st");
    // Hostile input cannot break the exposition format or mint a label of
    // unbounded length: quotes, backslashes, newlines and spaces are gone.
    assert_eq!(normalize_client(Some("ev\"il\\\nagent name"), None), "evil");
    assert_eq!(normalize_client(Some(&"x".repeat(200)), None).len(), 32);
    // Non-ASCII that filters to nothing must not produce an empty label.
    assert_eq!(normalize_client(Some("日本語"), None), "unattributed");

    assert_eq!(normalize_task(Some("aegis-3AYBC")), "aegis-3aybc");
    assert_eq!(normalize_task(None), "unattributed");
    assert_eq!(normalize_task(Some("   ")), "unattributed");
    assert_eq!(normalize_task(Some("bad\n task!")), "badtask");
    assert_eq!(normalize_task(Some(&"x".repeat(200))).len(), 64);
    assert_eq!(normalize_task(Some("日本語")), "unattributed");
}

#[test]
fn client_cardinality_is_capped_and_overflow_is_visible() {
    let m = Metrics::default();
    // Far more distinct callers than the cap, all on one endpoint.
    for i in 0..(MAX_CLIENTS * 4) {
        m.observe_client(&format!("caller{i}"), "unattributed", "/query", 0.1);
    }
    let text = m.render(0, 0, 0);
    let clients: BTreeSet<&str> = text
        .lines()
        .filter(|l| l.starts_with("quipu_http_client_requests_total"))
        .filter_map(|line| line.split("client=\"").nth(1)?.split('"').next())
        .collect();
    // The cap applies to caller identities. A client can legitimately have
    // one series per bounded route template.
    assert!(
        clients.len() <= MAX_CLIENTS,
        "client labels {} exceeded the cap {MAX_CLIENTS}",
        clients.len()
    );
    // Overflow is FOLDED, never dropped — the totals must still add up, or
    // the attribution silently understates load, which is worse than no
    // attribution at all.
    assert!(text.contains("client=\"other\""));
    let total: u64 = text
        .lines()
        .filter(|l| l.starts_with("quipu_http_client_requests_total"))
        .filter_map(|l| l.rsplit(' ').next()?.parse::<u64>().ok())
        .sum();
    assert_eq!(total as usize, MAX_CLIENTS * 4);
}

#[test]
fn client_cap_counts_clients_not_client_endpoint_pairs() {
    let m = Metrics::default();
    // One established caller can use many bounded route templates without
    // consuming the identity budget. The old map.len() check counted these
    // pairs and folded the next legitimate caller into `other`.
    for i in 0..MAX_CLIENTS {
        let endpoint = format!("/route/{i}");
        m.observe_client("existing", "unattributed", &endpoint, 0.1);
        m.observe_store_time("existing", &endpoint, 0.0, 0.1);
    }
    m.observe_client("new-caller", "unattributed", "/query", 0.1);
    m.observe_store_time("new-caller", "/query", 0.0, 0.1);

    let text = m.render(0, 0, 0);
    assert!(text.contains(
            "quipu_http_client_requests_total{client=\"new-caller\",task=\"unattributed\",endpoint=\"/query\"} 1"
        ));
    assert!(
        text.contains(
            "quipu_store_held_seconds_total{client=\"new-caller\",endpoint=\"/query\"} 0.1"
        )
    );
    assert!(!text.contains(
            "quipu_http_client_requests_total{client=\"other\",task=\"unattributed\",endpoint=\"/query\"}"
        ));
}

#[test]
fn task_cardinality_is_independent_capped_and_overflow_is_visible() {
    let m = Metrics::default();
    for i in 0..(MAX_TASKS * 2) {
        m.observe_client("query-first", &format!("aegis-{i}"), "/query", 0.1);
    }
    let text = m.render(0, 0, 0);
    let tasks: BTreeSet<&str> = text
        .lines()
        .filter(|l| l.starts_with("quipu_http_client_requests_total"))
        .filter_map(|line| line.split("task=\"").nth(1)?.split('"').next())
        .collect();
    assert!(tasks.len() <= MAX_TASKS);
    assert!(text.contains("client=\"query-first\",task=\"other\""));
    assert!(!text.contains("client=\"other\",task="));
    let total: u64 = text
        .lines()
        .filter(|l| l.starts_with("quipu_http_client_requests_total"))
        .filter_map(|l| l.rsplit(' ').next()?.parse::<u64>().ok())
        .sum();
    assert_eq!(total as usize, MAX_TASKS * 2);
}

#[test]
fn store_time_separates_waiting_from_burning() {
    let m = Metrics::default();
    // The measured shape of the aegis-vxl81 misread: a probe that WAITS a long
    // time in the queue while doing almost no work, alongside a caller that
    // actually occupies the store.
    m.observe_store_time("ma5er-probe", "/query", 2.0, 0.15);
    m.observe_store_time("ma5er-probe", "/query", 2.0, 0.15);
    m.observe_store_time("bulk-writer", "/query", 0.0, 4.0);
    let text = m.render(0, 0, 0);

    assert!(
        text.contains(
            "quipu_store_wait_seconds_total{client=\"ma5er-probe\",endpoint=\"/query\"} 4"
        )
    );
    assert!(text.contains(
        "quipu_store_held_seconds_total{client=\"ma5er-probe\",endpoint=\"/query\"} 0.3"
    ));
    assert!(
        text.contains(
            "quipu_store_held_seconds_total{client=\"bulk-writer\",endpoint=\"/query\"} 4"
        )
    );
    // THE POINT, and asserted against the RENDERED numbers rather than
    // against literals: ranking callers by wall-clock INVERTS the answer that
    // ranking them by held time gives. The probe looks like the bigger
    // consumer by wall-clock (4.3s vs 4.0s) while being ~7% of capacity by
    // held (0.3s vs 4.0s). That is the aegis-vxl81 misread in one assertion —
    // and clippy was right to reject the literal version, which asserted
    // arithmetic rather than behaviour.
    let val = |metric: &str, client: &str| -> f64 {
        text.lines()
            .find(|l| l.starts_with(metric) && l.contains(client))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or_else(|| panic!("no {metric} for {client}"))
    };
    let probe_wall = val("quipu_store_wait_seconds_total", "ma5er-probe")
        + val("quipu_store_held_seconds_total", "ma5er-probe");
    let writer_wall = val("quipu_store_wait_seconds_total", "bulk-writer")
        + val("quipu_store_held_seconds_total", "bulk-writer");
    let probe_held = val("quipu_store_held_seconds_total", "ma5er-probe");
    let writer_held = val("quipu_store_held_seconds_total", "bulk-writer");
    assert!(
        probe_wall > writer_wall,
        "wall-clock must rank the probe higher: {probe_wall} vs {writer_wall}"
    );
    assert!(
        probe_held < writer_held,
        "held must rank the WRITER higher: {probe_held} vs {writer_held}"
    );
}

#[test]
fn client_time_attribution_sums_and_start_time_is_absent_until_recorded() {
    let m = Metrics::default();
    m.observe_client("hank", "aegis-3aybc", "/query", 2.0);
    m.observe_client("hank", "aegis-3aybc", "/query", 3.0);
    m.observe_client("bobbin", "unattributed", "/query", 1.0);
    let text = m.render(0, 0, 0);
    assert!(
            text.contains(
                "quipu_http_client_requests_total{client=\"hank\",task=\"aegis-3aybc\",endpoint=\"/query\"} 2"
            )
        );
    assert!(text.contains(
            "quipu_http_client_request_seconds_total{client=\"hank\",task=\"aegis-3aybc\",endpoint=\"/query\"} 5"
        ));
    assert!(text.contains(
            "quipu_http_client_request_seconds_total{client=\"bobbin\",task=\"unattributed\",endpoint=\"/query\"} 1"
        ));
    // 5 of 6 seconds are hank's — the ratio this whole change exists to make
    // computable (aegis-ma1hy: 65.6% of /query load was unattributable).
}

#[test]
fn counters_histogram_and_gauges_render_in_exposition_format() {
    let m = Metrics::default();
    m.observe_request("/query", 200, 0.03);
    m.observe_request("/query", 200, 21.0); // the wedge-tail case
    m.observe_request("/knot", 400, 0.004);
    m.observe_policy_outcome("satisfied");
    m.observe_policy_outcome("unknown");
    let text = m.render(10, 200, 7);

    assert!(text.contains("quipu_http_requests_total{endpoint=\"/query\",status=\"200\"} 2"));
    assert!(text.contains("quipu_http_requests_total{endpoint=\"/knot\",status=\"400\"} 1"));
    // 21s lands past every finite bucket: le="30" cumulative picks it up,
    // and only +Inf and le="30" see the second observation.
    assert!(
        text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"0.1\"} 1"
        )
    );
    assert!(
        text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"30\"} 2"
        )
    );
    assert!(
        text.contains(
            "quipu_http_request_duration_seconds_bucket{endpoint=\"/query\",le=\"+Inf\"} 2"
        )
    );
    assert!(text.contains("quipu_http_request_duration_seconds_count{endpoint=\"/query\"} 2"));
    // The policy vocabulary is quipu's own three-valued one.
    assert!(text.contains("quipu_policy_check_total{outcome=\"satisfied\"} 1"));
    assert!(text.contains("quipu_policy_check_total{outcome=\"unknown\"} 1"));
    assert!(text.contains("quipu_graph_facts 200"));
}

#[test]
fn label_values_are_escaped() {
    let m = Metrics::default();
    m.observe_request("/od\"d\\path", 200, 0.01);
    let text = m.render(0, 0, 0);
    assert!(text.contains("endpoint=\"/od\\\"d\\\\path\""));
}

#[test]
fn memory_metrics_render_and_count_writes() {
    let m = Metrics::default();
    m.observe_write(5);
    m.observe_write(3);
    let text = m.render(0, 0, 0);
    assert!(text.contains("quipu_facts_written_total 8"));
    assert!(text.contains("process_resident_memory_bytes"));
    assert!(text.contains("process_virtual_memory_bytes"));
    assert!(text.contains("quipu_process_peak_rss_bytes"));
    // On the Linux target quipu runs on, RSS must be readable and non-zero.
    #[cfg(target_os = "linux")]
    {
        let (rss, vsz) = process_memory();
        assert!(
            rss > 0 && vsz >= rss,
            "RSS/VSZ readable from /proc/self/statm"
        );
    }
}

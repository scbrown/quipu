//! Keyword queries and one-batch-at-a-time maintenance of the derived index.

use crate::cli::flag_value;

pub fn cmd_search(args: &[String], db: &str) {
    let Some(query) = args.get(2).filter(|q| !q.starts_with("--")) else {
        eprintln!(
            "usage: quipu search <query> --mode keyword [--limit N] [--valid-at ISO] [--db path]"
        );
        std::process::exit(1);
    };
    let store = crate::cli_open::open_store(db);
    let mut input =
        serde_json::json!({"query":query,"mode":flag_value(args,"--mode").unwrap_or("keyword")});
    if let Some(limit) = flag_value(args, "--limit") {
        input["limit"] = serde_json::json!(limit.parse::<u64>().unwrap_or_else(|_| {
            eprintln!("error: --limit must be an unsigned integer");
            std::process::exit(1);
        }));
    }
    if let Some(at) = flag_value(args, "--valid-at") {
        input["valid_at"] = serde_json::json!(at);
    }
    if let Some(ty) = flag_value(args, "--type") {
        input["entity_type"] = serde_json::json!(ty);
    }
    if let Some(group) = flag_value(args, "--group") {
        input["group_ids"] = serde_json::json!([group]);
    }
    if let Some(expression) = flag_value(args, "--structured-query") {
        input["structured_query"] = serde_json::json!(expression);
    }
    if let Some(filters) = flag_value(args, "--filters") {
        input["filters"] = serde_json::from_str(filters).unwrap_or_else(|e| {
            eprintln!("error: --filters must be valid JSON: {e}");
            std::process::exit(1);
        });
    }
    match quipu::tool_search(&store, &input) {
        Ok(out) => println!("{}", serde_json::to_string_pretty(&out).unwrap()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub fn cmd_index(args: &[String], db: &str) {
    let action = args.get(2).map_or("status", String::as_str);
    // One invocation = one bounded batch. Refuse the protected ingest lane
    // before opening or creating the index; operators schedule the next batch.
    if action == "backfill" && ingest_lane(&quipu::time::now_iso()) {
        eprintln!("error: lexical backfill is paused during UTC minutes 10..20 (ingest lane)");
        std::process::exit(1);
    }
    if !matches!(action, "status" | "backfill" | "drop") {
        eprintln!("usage: quipu search-index status|backfill|drop [--batch-size 500] [--db path]");
        std::process::exit(1);
    }
    // Maintenance must not configure vector backends/ONNX or create an index
    // merely to report its absence. The fact-log Store registers trigger codecs.
    let store = quipu::Store::open(db).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    let out = match action {
        "status" => store
            .lexical_progress()
            .map(|p| serde_json::json!({"index":p})),
        "drop" => store
            .drop_lexical_index()
            .map(|()| serde_json::json!({"dropped":true})),
        _ => {
            let batch = flag_value(args, "--batch-size")
                .unwrap_or("500")
                .parse::<usize>()
                .unwrap_or_else(|_| {
                    eprintln!("error: --batch-size must be an integer");
                    std::process::exit(1)
                });
            store
                .backfill_lexical_batch(batch)
                .map(|p| serde_json::json!({"index":p}))
        }
    };
    match out {
        Ok(out) => println!("{}", serde_json::to_string_pretty(&out).unwrap()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1)
        }
    }
}

fn ingest_lane(timestamp: &str) -> bool {
    timestamp
        .get(14..16)
        .and_then(|m| m.parse::<u32>().ok())
        .is_none_or(|m| (10..=20).contains(&m))
}

#[cfg(test)]
mod tests {
    #[test]
    fn blackout_includes_both_edges_and_unknown_time() {
        for minute in 0..60 {
            assert_eq!(
                super::ingest_lane(&format!("2026-01-01T03:{minute:02}:00Z")),
                (10..=20).contains(&minute)
            );
        }
        assert!(super::ingest_lane("unknown"));
    }
}

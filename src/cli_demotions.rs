//! Read preserved unsupported demotions without materializing a closure.

pub fn run(args: &[String], db_path: &str) {
    let store = crate::cli_open::open_store(db_path);
    if let Err(error) = store.ensure_demotion_query(&quipu::time::now_iso()) {
        eprintln!("error preparing demotion query: {error}");
        std::process::exit(1);
    }
    let premise = crate::cli::flag_value(args, "--graph").unwrap_or(quipu::schema::ROOT_GRAPH_IRI);
    let companion = quipu::store::inferred::companion_iri_for(premise);
    let input = serde_json::json!({
        "name": quipu::store::demotions::QUERY_NAME,
        "params": {"graph": companion}
    });
    match quipu::tool_ask(&store, &input) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(error) => {
            eprintln!("error listing demotions: {error}");
            std::process::exit(1);
        }
    }
}

//! Narrow typed answer command; opening a reader never migrates or creates a store.

use std::io::Read;

pub(crate) fn run(args: &[String], path: &str) {
    if let Err(error) = execute(args, path) {
        eprintln!("rule-query: {error}");
        std::process::exit(1);
    }
}

fn execute(args: &[String], path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = args
        .get(2)
        .filter(|value| !value.starts_with('-'))
        .ok_or("usage: quipu rule-query request.json --db existing.db")?;
    let mut bytes = Vec::new();
    std::fs::File::open(file)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("request exceeds 65536 bytes".into());
    }
    let request: quipu::rule_answers::Request = serde_json::from_slice(&bytes)?;
    request.validate()?;
    if !crate::cli_open::config().attachments.is_empty() {
        return Err("rule-query does not support attached/composed scope".into());
    }
    let mut store = quipu::Store::open_read_only(path)?;
    store
        .labels_config_mut()
        .clone_from(&crate::cli_open::config().label_floors);
    let answer = quipu::rule_answers::query(&store, &request)?;
    println!("{}", serde_json::to_string(&answer)?);
    Ok(())
}

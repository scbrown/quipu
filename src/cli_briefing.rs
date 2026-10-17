//! Explicit task briefings; open an existing store read-only.
use std::io::Read;
pub fn run(args: &[String], db: &str, config: &quipu::SearchConfig) {
    let result = (|| -> quipu::Result<serde_json::Value> {
        let file = args.get(2).ok_or_else(|| {
            quipu::Error::InvalidValue(
                "usage: quipu task-briefing request.json --db existing.db".into(),
            )
        })?;
        let mut raw = Vec::new();
        std::fs::File::open(file)
            .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?
            .take(16385)
            .read_to_end(&mut raw)
            .map_err(|e| quipu::Error::InvalidValue(e.to_string()))?;
        if raw.len() > 16384 {
            return Err(quipu::Error::InvalidValue(
                "briefing request byte cap".into(),
            ));
        }
        let input =
            serde_json::from_slice(&raw).map_err(|e| quipu::Error::InvalidValue(e.to_string()))?;
        let mut store = quipu::Store::open_read_only(db)?;
        store.search_config_mut().clone_from(config);
        quipu::context::briefing::task_briefing(&store, &input)
    })();
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}

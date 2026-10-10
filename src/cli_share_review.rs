//! Explicit import-review decisions and policy-driven aging notices.
use crate::cli::{chrono_now, flag_value};

pub fn run(args: &[String], db: &str) {
    let mut store = crate::cli_open::open_store(db);
    let timestamp = chrono_now();
    let after = flag_value(args, "--after").unwrap_or("");
    let limit = flag_value(args, "--limit")
        .unwrap_or("100")
        .parse::<usize>();
    let result = (|| -> quipu::Result<serde_json::Value> {
        let limit = limit
            .map_err(|_| quipu::Error::InvalidValue("review --limit must be an integer".into()))?;
        match args.get(3).map(String::as_str) {
            Some("pending")=>quipu::share_review::pending(&store,after,limit,&timestamp),
            Some("notify")=> {
                let age=flag_value(args,"--age-seconds").and_then(|v|v.parse().ok()).ok_or_else(||quipu::Error::InvalidValue("notify requires --age-seconds integer".into()))?;
                let route=flag_value(args,"--route").ok_or_else(||quipu::Error::InvalidValue("notify requires --route".into()))?;
                quipu::share_review::notify_due(&mut store,after,limit,&timestamp,age,route)
            }
            Some(decision @ ("rejected"|"expired"|"reopen"))=> {
                let id=args.get(4).filter(|v|!v.starts_with('-')).ok_or_else(||quipu::Error::InvalidValue("review decision needs share_id".into()))?;
                let actor=flag_value(args,"--actor").ok_or_else(||quipu::Error::InvalidValue("review decision needs --actor".into()))?;
                let reason=flag_value(args,"--reason").ok_or_else(||quipu::Error::InvalidValue("review decision needs --reason".into()))?;
                quipu::share_review::decide(&mut store,id,decision,actor,reason,&timestamp)?;
                Ok(serde_json::json!({"share_id":id,"decision":decision}))
            }
            _=>Err(quipu::Error::InvalidValue("usage: import review pending|notify|rejected|expired|reopen [share_id] [--db PATH]".into()))
        }
    })();
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => {
            eprintln!("review error: {error}");
            std::process::exit(1)
        }
    }
}

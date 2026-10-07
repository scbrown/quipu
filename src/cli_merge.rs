//! `quipu merge` decision flags: emit, propose and apply (aegis-yavo9c).

use crate::cli::{chrono_now, flag_value};

pub(crate) const MERGE_USAGE: &str = "usage: quipu merge <share-dir> [--actor <id>] [--db <path>]\n\
       quipu merge <share-dir> --emit-decisions <file.json> [--propose] [--db <path>]\n\
       quipu merge <share-dir> --decisions <file.json> --reviewer <who> [--actor <id>] [--db <path>]";

/// Handle `--emit-decisions` / `--decisions`; false when neither was given.
pub fn cmd_merge_decisions(args: &[String], db_path: &str, dir: &str) -> bool {
    if let Some(out) = flag_value(args, "--emit-decisions") {
        let store = crate::cli_open::open_store(db_path);
        let emitted =
            quipu::share_merge_decisions::emit(&store, std::path::Path::new(dir)).map(|file| {
                if args.iter().any(|a| a == "--propose") {
                    quipu::share_merge_decisions::propose(file)
                } else {
                    file
                }
            });
        match emitted.and_then(|file| {
            let body = serde_json::to_string_pretty(&file)
                .map_err(|e| quipu::Error::Serialization(e.to_string()))?;
            std::fs::write(out, body + "\n")
                .map_err(|e| quipu::Error::Store(format!("write {out}: {e}")))?;
            Ok(file.rows.len())
        }) {
            Ok(rows) => println!(
                "wrote {rows} decision row(s) to {out}; nothing was merged. Set each row's \"decision\", then: quipu merge {dir} --decisions {out} --reviewer <who>"
            ),
            Err(error) => {
                eprintln!("merge error: {error}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if let Some(path) = flag_value(args, "--decisions") {
        let Some(reviewer) = flag_value(args, "--reviewer") else {
            eprintln!("merge: --decisions requires --reviewer <who>\n{MERGE_USAGE}");
            std::process::exit(1);
        };
        let mut store = crate::cli_open::open_store(db_path);
        let applied = std::fs::read(path)
            .map_err(|e| quipu::Error::Store(format!("read {path}: {e}")))
            .and_then(|bytes| {
                let file = serde_json::from_slice(&bytes).map_err(|e| {
                    quipu::Error::InvalidValue(format!("{path} is not a decisions file: {e}"))
                })?;
                quipu::share_merge_decisions::apply(
                    &mut store,
                    std::path::Path::new(dir),
                    &file,
                    &bytes,
                    reviewer,
                    &chrono_now(),
                    flag_value(args, "--actor"),
                )
            });
        match applied {
            Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
            Err(error) => {
                eprintln!("merge error: {error}");
                std::process::exit(1);
            }
        }
        return true;
    }
    false
}

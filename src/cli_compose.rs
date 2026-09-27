//! Compose local verified packs into a named inspection dataset.

use crate::cli::{chrono_now, flag_value};

pub fn cmd_compose(args: &[String], db_path: &str) {
    let result = run(args, db_path);
    match result {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            if result.outcome == "quarantined" {
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("composition error: {error}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[String], db_path: &str) -> quipu::Result<quipu::share_compose::Composition> {
    let mut references = Vec::new();
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--db" | "--shapes-from" | "--actor" | "--destination" => {
                if args.get(index + 1).is_none() {
                    return Err(quipu::Error::InvalidValue(format!(
                        "{} needs a value",
                        args[index]
                    )));
                }
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(quipu::Error::InvalidValue(format!(
                    "unknown compose option: {value}"
                )));
            }
            value => {
                references.push(value);
                index += 1;
            }
        }
    }
    let authority = flag_value(args, "--shapes-from")
        .map(|reference| {
            references
                .iter()
                .position(|r| *r == reference)
                .ok_or_else(|| {
                    quipu::Error::InvalidValue(
                        "--shapes-from must name one of the input paths".into(),
                    )
                })
        })
        .transpose()?;
    let destination = match flag_value(args, "--destination") {
        None | Some("outward") => quipu::share::ShareDestination::Outward,
        Some("internal") => quipu::share::ShareDestination::Internal,
        Some(value) => {
            return Err(quipu::Error::InvalidValue(format!(
                "unknown destination: {value}"
            )));
        }
    };
    let requests = references
        .into_iter()
        .map(|reference| {
            let mut request = quipu::share_transport::read_local(reference)?;
            request.destination = destination;
            Ok(request)
        })
        .collect::<quipu::Result<Vec<_>>>()?;
    let mut store = crate::cli_open::open_store(db_path);
    quipu::share_compose::compose(
        &mut store,
        &requests,
        authority,
        &chrono_now(),
        flag_value(args, "--actor"),
    )
}

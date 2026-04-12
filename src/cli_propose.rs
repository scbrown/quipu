//! CLI command: `quipu propose` — schema evolution proposal management.

use crate::cli::chrono_now;

pub fn cmd_propose(args: &[String], db_path: &str) {
    let action = args.get(2).map_or("list", std::string::String::as_str);

    let store = match quipu::Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error opening store: {e}");
            std::process::exit(1);
        }
    };

    match action {
        "submit" => cmd_propose_submit(args, &store),
        "accept" => cmd_propose_accept(args, &store),
        "reject" => cmd_propose_reject(args, &store),
        _ => cmd_propose_list(args, &store),
    }
}

fn cmd_propose_submit(args: &[String], store: &quipu::Store) {
    let kind = match args.get(3) {
        Some(k) if !k.starts_with("--") => k.as_str(),
        _ => {
            eprintln!(
                "usage: quipu propose submit <kind> <target> <file.ttl> --proposer <id> \
                 [--rationale <text>] [--trigger <ref>] [--db <path>]"
            );
            std::process::exit(1);
        }
    };
    let target = match args.get(4) {
        Some(t) if !t.starts_with("--") => t.as_str(),
        _ => {
            eprintln!("usage: quipu propose submit <kind> <target> <file.ttl> ...");
            std::process::exit(1);
        }
    };
    let file_path = match args.get(5) {
        Some(p) if !p.starts_with("--") => p.as_str(),
        _ => {
            eprintln!("usage: quipu propose submit <kind> <target> <file.ttl> ...");
            std::process::exit(1);
        }
    };
    let diff = match std::fs::read_to_string(file_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error reading {file_path}: {e}");
            std::process::exit(1);
        }
    };
    let proposer = match args.windows(2).find(|w| w[0] == "--proposer") {
        Some(w) => w[1].as_str(),
        None => {
            eprintln!("--proposer is required");
            std::process::exit(1);
        }
    };
    let rationale = args
        .windows(2)
        .find(|w| w[0] == "--rationale")
        .map(|w| w[1].as_str());
    let trigger_ref = args
        .windows(2)
        .find(|w| w[0] == "--trigger")
        .map(|w| w[1].as_str());

    let proposal_kind = match quipu::ProposalKind::from_json(kind) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let now = chrono_now();
    match store.insert_proposal(&quipu::proposal::NewProposal {
        kind: &proposal_kind,
        target,
        diff: &diff,
        rationale,
        proposer,
        trigger_ref,
        created_at: &now,
    }) {
        Ok(id) => println!("proposal {id} created (status: pending)"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_propose_accept(args: &[String], store: &quipu::Store) {
    let id_str = match args.get(3) {
        Some(s) if !s.starts_with("--") => s.as_str(),
        _ => {
            eprintln!("usage: quipu propose accept <id> [--note <text>] [--db <path>]");
            std::process::exit(1);
        }
    };
    let id: i64 = id_str.parse().unwrap_or_else(|_| {
        eprintln!("invalid proposal id: {id_str}");
        std::process::exit(1);
    });
    let note = args
        .windows(2)
        .find(|w| w[0] == "--note")
        .map(|w| w[1].as_str());
    let now = chrono_now();
    match store.accept_proposal(id, "cli-user", &now, note) {
        Ok(p) => println!(
            "proposal {} accepted (target: {}, kind: {:?})",
            p.id, p.target, p.kind
        ),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_propose_reject(args: &[String], store: &quipu::Store) {
    let id_str = match args.get(3) {
        Some(s) if !s.starts_with("--") => s.as_str(),
        _ => {
            eprintln!("usage: quipu propose reject <id> --note <reason> [--db <path>]");
            std::process::exit(1);
        }
    };
    let id: i64 = id_str.parse().unwrap_or_else(|_| {
        eprintln!("invalid proposal id: {id_str}");
        std::process::exit(1);
    });
    let note = match args.windows(2).find(|w| w[0] == "--note") {
        Some(w) => w[1].as_str(),
        None => {
            eprintln!("--note is required for rejection");
            std::process::exit(1);
        }
    };
    let now = chrono_now();
    match store.reject_proposal(id, "cli-user", &now, note) {
        Ok(p) => println!("proposal {} rejected (target: {})", p.id, p.target),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_propose_list(args: &[String], store: &quipu::Store) {
    let status_filter = args
        .windows(2)
        .find(|w| w[0] == "--status")
        .map(|w| w[1].as_str());
    let status = status_filter
        .map(quipu::ProposalStatus::from_json)
        .transpose()
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    match store.list_proposals(status.as_ref()) {
        Ok(proposals) => {
            if proposals.is_empty() {
                println!("no proposals found");
            } else {
                for p in &proposals {
                    println!(
                        "  #{} [{}] {:?} {} -- {} (by {})",
                        p.id,
                        match p.status {
                            quipu::ProposalStatus::Pending => "pending",
                            quipu::ProposalStatus::Accepted => "accepted",
                            quipu::ProposalStatus::Rejected => "rejected",
                        },
                        p.kind,
                        p.target,
                        p.rationale.as_deref().unwrap_or("(no rationale)"),
                        p.proposer,
                    );
                }
                println!("\n{} proposal(s)", proposals.len());
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

//! `quipu attest` — the shipped registration surface for session bindings
//! (aegis-tadzdf, condition 1).
//!
//! # Why this command has to exist in the same change as the `claimed` tier
//!
//! `attestation_register` existed, worked, was tested, and had **zero callers on
//! any shipped path**. So `attested` was unreachable: every real import verified
//! against an empty registry and could only ever be `transport`. A three-value
//! vocabulary whose third value cannot occur is a two-value vocabulary plus
//! documentation, and this fleet has shipped that shape more than once.
//!
//! malcolm made it a gate rather than a companion: the tier is not real until a
//! test registers out-of-band and REACHES `attested`.
//!
//! # Out-of-band is the whole point
//!
//! This command takes a binding the operator obtained some other way. It must
//! never be fed from the share being imported: registering a producer's key from
//! the bundle that key vouches for makes `attested` mean "arrived with a
//! self-signed claim", and a whole-bundle substitution swaps the key along with
//! the data. `signing.rs` settled the same question for the governance plane —
//! "quipu never self-registers (that would let it vouch for itself)".

use quipu::session_attestation::SessionBinding;

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

fn need(args: &[String], name: &str) -> String {
    match flag(args, name) {
        Some(v) => v.to_string(),
        None => {
            eprintln!("attest register requires {name}");
            std::process::exit(2);
        }
    }
}

pub fn cmd_attest(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("register") => register(args, db_path),
        Some("list") => list(db_path),
        Some(verb @ ("allow-write" | "deny-write")) => {
            set_write(args, db_path, verb == "allow-write");
        }
        Some("revoke") => revoke(args, db_path),
        _ => {
            eprintln!(
                "quipu attest register --agent A --session S --public-key HEX \\
                 --introducer I --issued-at EPOCH --expires-at EPOCH [--allow-write] [--db PATH]\n\
                 quipu attest allow-write|deny-write <SESSION> [--db PATH]\n\
                 quipu attest revoke <SESSION> [--db PATH]\n\
                 quipu attest list [--db PATH]\n\n\
                 A binding is SHARE-ONLY unless granted write: --allow-write at\n\
                 registration, or allow-write later. Only then may its key sign\n\
                 HTTP writes to /knot, /update, /episode (aegis-bys8d1).\n\n\
                 Registers a producer session binding OUT OF BAND, which is what a\n\
                 share import needs to reach tier=attested. Do NOT populate this from\n\
                 a share you are importing: a key that vouches for the bundle it\n\
                 arrived in vouches for nothing (aegis-tadzdf)."
            );
            std::process::exit(2);
        }
    }
}

fn register(args: &[String], db_path: &str) {
    let issued: u64 = need(args, "--issued-at").parse().unwrap_or_else(|_| {
        eprintln!("--issued-at must be seconds since the epoch");
        std::process::exit(2);
    });
    let expires: u64 = need(args, "--expires-at").parse().unwrap_or_else(|_| {
        eprintln!("--expires-at must be seconds since the epoch");
        std::process::exit(2);
    });
    let mut binding = match SessionBinding::new(
        need(args, "--agent"),
        need(args, "--session"),
        need(args, "--public-key"),
        need(args, "--introducer"),
        issued,
        expires,
    ) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("invalid binding: {e}");
            std::process::exit(1);
        }
    };
    binding.allow_write = args.iter().any(|a| a == "--allow-write");
    let store = crate::cli_open::open_store(db_path);
    match store.attestation_register(&binding) {
        Ok(()) => {
            println!(
                "registered session {} for agent {} (key_id {})",
                binding.session, binding.agent, binding.key_id
            );
            println!("  an import carrying an envelope for this session now reaches tier=attested");
        }
        Err(e) => {
            eprintln!("register failed: {e}");
            std::process::exit(1);
        }
    }
}

fn set_write(args: &[String], db_path: &str, allow: bool) {
    let Some(session) = args.get(3).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: quipu attest allow-write|deny-write <SESSION> [--db PATH]");
        std::process::exit(2);
    };
    let store = crate::cli_open::open_store(db_path);
    match store.attestation_set_write(session, allow) {
        Ok(()) => println!(
            "session {session}: write {}",
            if allow {
                "GRANTED"
            } else {
                "withdrawn (share-only)"
            }
        ),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Revoke a session binding. Rotation is "register the new binding, revoke the
/// old one", and before this verb the second half had no operator path: the
/// store function existed with tests as its only callers, so a revoke on a live
/// store meant raw SQL (aegis-bys8d1, measured on the first production probe).
/// The row is kept: a revoked binding refuses as `revoked`, which is a
/// different finding from an unbound one.
fn revoke(args: &[String], db_path: &str) {
    let Some(session) = args.get(3).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: quipu attest revoke <SESSION> [--db PATH]");
        std::process::exit(2);
    };
    let store = crate::cli_open::open_store(db_path);
    match store.attestation_revoke(session) {
        Ok(()) => println!("session {session}: REVOKED"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn list(db_path: &str) {
    let store = crate::cli_open::open_store(db_path);
    match store.attestation_bindings() {
        Ok(bindings) if bindings.is_empty() => {
            println!("no registered session bindings");
            println!("  every import carrying an envelope will report tier=claimed, not attested");
        }
        Ok(bindings) => {
            for b in bindings {
                println!(
                    "{}\t{}\tkey_id={}\tintroducer={}\tallow_write={}\texpires_at={}\trevoked={}",
                    b.agent,
                    b.session,
                    b.key_id,
                    b.introducer,
                    b.allow_write,
                    b.expires_at_epoch,
                    b.revoked
                );
            }
        }
        Err(e) => {
            eprintln!("list failed: {e}");
            std::process::exit(1);
        }
    }
}

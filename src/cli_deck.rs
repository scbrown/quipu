//! `quipu deck` — the operator's enrollment surface for the Decision Deck
//! (aegis-jjy).
//!
//! `deck_configure`, `deck_enroll`, `deck_set_enabled` and `deck_revoke`
//! existed, were tested, and had no caller outside the tests, so a real
//! enrollment needed raw SQL. These verbs are that missing caller, and nothing
//! more: each one runs the existing native method against the store file.
//!
//! # Store-file access is the authority
//!
//! There is deliberately no HTTP route for any of this. Configuring the deck,
//! enrolling a credential and flipping the switch are operator acts, and the
//! operator is whoever holds the database file. A browser, a graph write or a
//! carried share cannot reach these methods.
//!
//! # What enrollment proves
//!
//! The credential ID and COSE key come from the ceremony page on the dedicated
//! origin. `enroll` prints the key's fingerprint, and `status` prints it again;
//! the human compares it with the fingerprint their device showed. A match
//! names the key that will sign. It does not prove which person holds it: that
//! is the ceremony's job, and why the operator, not the page, runs `enroll`.

use quipu::store::decision_deck::{Config, Credential, credential_fingerprint};

const USAGE: &str = "\
quipu deck configure --origin https://HOST --rp-id HOST --actor WHO --author CURATOR [--db PATH]
quipu deck enroll --credential-id B64URL --cose-key B64URL --scheme webauthn-es256|webauthn-eddsa --expires-at EPOCH [--db PATH]
quipu deck enable|disable [--db PATH]
quipu deck revoke <CREDENTIAL-ID> [--db PATH]
quipu deck status [--db PATH]

Operator-only: these act on the store file directly and have no HTTP route.
configure installs the deck's one immutable scope, disabled. enroll records a
public credential from the enrollment ceremony and prints its fingerprint:
compare it with the one the device showed before enabling. Decisions also need
the server's [quipu.server] deck_http switch; enable is the store's half.";

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

fn need(args: &[String], verb: &str, name: &str) -> String {
    if let Some(v) = flag(args, name) {
        return v.to_string();
    }
    eprintln!("deck {verb} requires {name}");
    std::process::exit(2);
}

fn fail(what: &str, e: impl std::fmt::Display) -> ! {
    eprintln!("{what} failed: {e}");
    std::process::exit(1);
}

pub fn cmd_deck(args: &[String], db_path: &str) {
    match args.get(2).map(String::as_str) {
        Some("configure") => configure(args, db_path),
        Some("enroll") => enroll(args, db_path),
        Some(verb @ ("enable" | "disable")) => set_enabled(db_path, verb == "enable"),
        Some("revoke") => revoke(args, db_path),
        Some("status") => status(db_path),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn configure(args: &[String], db_path: &str) {
    let config = Config {
        origin: need(args, "configure", "--origin"),
        rp_id: need(args, "configure", "--rp-id"),
        actor: need(args, "configure", "--actor"),
        author: need(args, "configure", "--author"),
    };
    let mut store = crate::cli_open::open_store(db_path);
    if let Err(e) = store.deck_configure(&config) {
        fail("configure", e);
    }
    println!(
        "deck scope installed for {} (actor {}, curator {}); DISABLED until `quipu deck enable`",
        config.origin, config.actor, config.author
    );
}

fn enroll(args: &[String], db_path: &str) {
    let cose_key = quipu::verdict_schemes::b64url_decode(&need(args, "enroll", "--cose-key"))
        .unwrap_or_else(|e| {
            eprintln!("--cose-key must be unpadded base64url: {e}");
            std::process::exit(2);
        });
    let expires_at: i64 = need(args, "enroll", "--expires-at")
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("--expires-at must be seconds since the epoch");
            std::process::exit(2);
        });
    let credential = Credential {
        id: need(args, "enroll", "--credential-id"),
        scheme: need(args, "enroll", "--scheme"),
        cose_key,
        expires_at,
    };
    let mut store = crate::cli_open::open_store(db_path);
    if let Err(e) = store.deck_enroll(&credential) {
        fail("enroll", e);
    }
    println!(
        "enrolled credential {} ({}), fingerprint {}",
        credential.id,
        credential.scheme,
        credential_fingerprint(&credential.cose_key)
    );
    println!("  compare this fingerprint with the one the enrolling device showed");
}

fn set_enabled(db_path: &str, enabled: bool) {
    let mut store = crate::cli_open::open_store(db_path);
    if let Err(e) = store.deck_set_enabled(enabled) {
        fail(if enabled { "enable" } else { "disable" }, e);
    }
    println!(
        "deck {} in this store (the server also needs deck_http = true)",
        if enabled { "ENABLED" } else { "DISABLED" }
    );
}

fn revoke(args: &[String], db_path: &str) {
    let Some(id) = args.get(3).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: quipu deck revoke <CREDENTIAL-ID> [--db PATH]");
        std::process::exit(2);
    };
    let mut store = crate::cli_open::open_store(db_path);
    match store.deck_revoke(id) {
        Ok(true) => println!("credential {id}: REVOKED"),
        Ok(false) => fail("revoke", format!("no enrolled credential {id}")),
        Err(e) => fail("revoke", e),
    }
}

fn status(db_path: &str) {
    let store = crate::cli_open::open_store(db_path);
    let status = match store.deck_status() {
        Ok(Some(status)) => status,
        Ok(None) => {
            println!("deck not configured (every decision route refuses)");
            return;
        }
        Err(e) => fail("status", e),
    };
    println!(
        "origin={}\trp_id={}\tactor={}\tauthor={}\tenabled={}",
        status.origin, status.rp_id, status.actor, status.author, status.enabled
    );
    if status.credentials.is_empty() {
        println!("no enrolled credentials");
    }
    for c in status.credentials {
        println!(
            "{}\tscheme={}\tfingerprint={}\texpires_at={}\trevoked={}\tsign_count={}",
            c.id, c.scheme, c.fingerprint, c.expires_at, c.revoked, c.sign_count
        );
    }
}

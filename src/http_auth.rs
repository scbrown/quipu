//! Access-control decisions for the Quipu REST server (hq-azs).
//!
//! The server bin (`quipu-server`) is feature-gated behind `onnx`, so its axum
//! wiring isn't exercised by the default CI matrix. The *policy* — is this a
//! write? is it allowed under read-only mode? does the bearer token match? — is
//! pure and lives here so it can be unit-tested without standing up a server.

/// The set of write endpoints. A request to one of these mutates the fact log
/// (or schema), so it is subject to read-only mode and bearer auth. Everything
/// else (query, search, entity reads, UI, health) is treated as read-only and
/// stays open.
///
/// WHY THIS IS A HAND-KEPT LIST AND NOT DERIVED FROM A MACRO (aegis-2f4n).
/// The obvious idea — "the write set is the `rw_handler!` routes in server.rs" —
/// is WRONG here, and wrong in the dangerous direction. Write-ness in this crate
/// is not visible in the handler's type or its registration macro, because
/// `Store` writes through an `&self` method (interior mutability over the
/// `SQLite` connection). So a route can be `ro_handler!`, take `&Store`, and
/// still commit a transaction. Measured 2026-07-20, five such routes do exactly
/// that: `/shapes` (`load_shapes`), `/propose` (`insert_proposal`),
/// `/proposal/accept` (`accept_proposal`), `/proposal/reject` (`reject_proposal`),
/// and `/overlay/create` (`overlay_create` writes the graphs registry). Deriving
/// the set from `rw_handler!` would drop all five from protection while looking
/// principled.
///
/// The only sound invariant is therefore COMPLETENESS, not derivation: every
/// route the server registers must be classified as exactly one of write / read,
/// and `write_endpoints_cover_every_route` (below) fails the build if any route
/// is left unclassified. That test parses the router source directly, so it runs
/// in the default matrix even though `server.rs` is a separate `onnx`-gated bin
/// that the matrix never compiles — which is the gap that let this drift: the
/// list had a "kept in sync" comment and nothing enforcing it, and it had drifted
/// to omit `/project`, `/overlay/write` and `/overlay/create` (all writing).
///
/// Adding a route to `server.rs`? You must classify it here or in `READ_ENDPOINTS`,
/// or the test fails. That forced decision is the whole point — write-ness cannot
/// be inferred for you.
pub const WRITE_ENDPOINTS: &[&str] = &[
    "/knot",
    "/knot/stage",
    "/knot/promote",
    "/episode",
    "/import",
    "/import/promote",
    "/episodes/complete",
    "/retract",
    "/set",
    "/episode/retract",
    "/shapes",
    "/impact",
    "/propose",
    "/proposal/accept",
    "/proposal/reject",
    "/embed_backfill",
    // aegis-5qmg3r: alignment. `apply` takes &mut Store, materialises
    // owl:sameAs / quipu:distinctFrom, AND creates the derived alignment graph
    // (a graphs-registry write, the same reason /overlay/create is here).
    "/align/apply",
    // aegis-2f4n: registered write routes that WRITE_ENDPOINTS had silently
    // omitted, so read-only mode and bearer auth did not cover them.
    "/project", // rw_handler; louvain persists quipu:memberOfCommunity when persist:true
    "/overlay/write", // &mut handler -> store.overlay_write, returns a tx_id
    "/overlay/create", // ro_handler by signature, but writes the graphs registry
    // camayoc-s0h: registering a graph writes the graphs registry; labelling
    // one writes the label meta-graph. Both are writes and both are gated.
    "/graph/create",
    "/graph/label",
    "/graph/freeze",  // deep freeze: relocates rows, mutates registry + attachments
    "/graph/thaw",    // restores rows, mutates registry + attachments
    "/events/commit", // durable consumer cursor upsert (event-log P1)
    "/subscriptions", // push-subscription registry create/list/delete (event-log P2)
    "/datasets",      // named-dataset registry create/remove (quipu #69) + meta-graph mirror
    "/update",        // SPARQL 1.1 Update mutates default and named graphs
    "/queries",       // stored named-query registry load/remove (quipu #79)
    // aegis-06q1r: OWL ontology load/list/remove. `load` both PERSISTS the
    // ontology and MATERIALIZES entailments (new rdf:type / inverse facts), so it
    // is emphatically a write — and `remove` drops a stored ontology. Listed
    // unconditionally even though the handler is cfg(feature = "owl"), because
    // the enforcer scans server.rs as TEXT and sees the route either way.
    "/ontology",
    // quipu-923: /reason runs a Datalog ruleset and PERSISTS its derivations
    // (assert + retract through the fact log) — a write however read-shaped
    // "run the reasoner" sounds.
    "/reason",
];

/// The set of read endpoints: every registered route that does NOT mutate state.
/// Explicit, not "everything not in `WRITE_ENDPOINTS`", so that a NEW route is
/// unclassified until a human puts it in one list or the other — see the
/// completeness test. Parameterized paths keep their axum `{param}` form so they
/// match the router source verbatim.
pub const READ_ENDPOINTS: &[&str] = &[
    // aegis-5qmg3r: alignment reads. `propose` takes &Store and only queries
    // (lookup + a prepared SELECT); `decide` touches no store at all. The
    // writer of the three is /align/apply, above.
    "/align/propose",
    "/align/decide",
    // Method-sensitive: GET/HEAD are reads; PUT/POST/DELETE are writes.
    "/rdf-graph-store",
    "/graphs",        // registry listing + kind capability probe (pooled read)
    "/path/cone",     // golden-path provenance cone (ro_handler, quipu-gp2)
    "/explain",       // derivation-chain walk (quipu-923) — reads provenance, commits nothing
    "/path/backtest", // golden-path candidate backtest (ro_handler, quipu-gp3)
    "/",
    "/ui",
    "/quipu-components.js",
    "/graph-canvas.js",
    "/datalinks.js",
    // Vendored three.js for the 3D Datalinks view. A static asset, like the
    // other UI files — served unauthenticated so the page loads.
    "/vendor/three.module.min.js",
    "/health",
    "/version",
    "/stats",
    "/.well-known/void",
    // Prometheus scrape: renders in-memory counters + one SQL COUNT aggregate.
    // Reads the store, mutates nothing.
    "/metrics",
    "/query",
    "/cord",
    // Render-ready node-link projection for the UI. Reads the fact log only.
    "/graph",
    "/unravel",
    "/validate",
    "/search",
    "/hybrid_search",
    "/unified_search",
    "/ask",
    "/search_nodes",
    "/search_facts",
    "/search/nodes",
    "/proposals",
    "/overlay/compose",
    "/cooccurrence",
    "/policy/check",
    "/verifier/authorized",
    "/verdict/verify",
    "/report",
    "/context",
    "/entity/{iri}",
    "/entity",
    "/entity/{iri}/json",
    "/entity/{iri}/ttl",
    "/entity/{iri}/html",
    "/entity_history",
    "/transactions",
    "/events",  // pull-batch event log read (event-log P1); the commit half is a write
    "/changes", // fact-level change feed (quipu-2ae): pull-only, cursor is a tx id
    "/spotlight",
    "/fragments",
    "/reconcile",
    "/preview/{iri}",
    // Subset export (quipu #36): serializes one named graph's (or ROOT's) facts
    // to RDF. Pure read.
    "/export",
    // Canonical share serialization: returns the export and manifest without
    // writing either to the server filesystem.
    "/share",
    // Resolution dry-run: reads labels + vector store, writes
    // nothing — the read-only twin of the resolution /episode performs.
    "/resolve",
];

/// Whether `path` is a write endpoint subject to auth / read-only policy.
pub fn is_write_endpoint(path: &str) -> bool {
    WRITE_ENDPOINTS.contains(&path)
}

/// Classify routes whose write-ness depends on the HTTP method.
pub fn is_write_request(path: &str, method: &str) -> bool {
    if path == "/rdf-graph-store" {
        matches!(method, "PUT" | "POST" | "DELETE")
    } else {
        is_write_endpoint(path)
    }
}

/// Outcome of an access-control check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessDecision {
    /// Proceed with the request.
    Allow,
    /// Reject: a bearer token is required or did not match (HTTP 401).
    Unauthorized,
    /// Reject: the server is read-only and this is a write (HTTP 403).
    ReadOnly,
}

/// Server-established identity attached to an authenticated write request.
///
/// The shared bearer is deliberately not a crew identity. Until session
/// attestation lands, writes using it receive this explicit legacy principal
/// instead of trusting an actor supplied in the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedPrincipal(&'static str);

impl AuthenticatedPrincipal {
    pub const LEGACY_SHARED_BEARER: Self = Self("legacy-shared-bearer");

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Decide whether a request may proceed.
///
/// Reads (`is_write == false`) are always allowed. Writes are rejected when the
/// server is read-only, and — when an `auth_token` is configured — require a
/// matching `Authorization: Bearer <token>` header. With no token configured,
/// writes are open (today's LAN-trusted default).
pub fn authorize(
    is_write: bool,
    read_only: bool,
    auth_token: Option<&str>,
    auth_header: Option<&str>,
) -> AccessDecision {
    if !is_write {
        return AccessDecision::Allow;
    }
    if read_only {
        return AccessDecision::ReadOnly;
    }
    match auth_token {
        None => AccessDecision::Allow,
        Some(expected) => match auth_header.and_then(parse_bearer) {
            Some(presented) if constant_time_eq(presented.as_bytes(), expected.as_bytes()) => {
                AccessDecision::Allow
            }
            _ => AccessDecision::Unauthorized,
        },
    }
}

/// Extract the token from an `Authorization: Bearer <token>` header value.
/// Case-insensitive on the scheme; trims surrounding whitespace on the token.
pub fn parse_bearer(header: &str) -> Option<&str> {
    let header = header.trim_start();
    let (scheme, rest) = header.split_at(header.find(' ')?);
    if scheme.eq_ignore_ascii_case("Bearer") {
        let token = rest.trim();
        if token.is_empty() { None } else { Some(token) }
    } else {
        None
    }
}

/// Length-checked, constant-time byte comparison so token validation does not
/// leak length/prefix information through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_always_allowed() {
        // Even read-only + token-required, a read needs no auth.
        assert_eq!(
            authorize(false, true, Some("secret"), None),
            AccessDecision::Allow
        );
    }

    #[test]
    fn read_only_blocks_writes() {
        assert_eq!(authorize(true, true, None, None), AccessDecision::ReadOnly);
        // Read-only wins even with a valid token.
        assert_eq!(
            authorize(true, true, Some("s"), Some("Bearer s")),
            AccessDecision::ReadOnly
        );
    }

    #[test]
    fn writes_open_when_no_token_configured() {
        assert_eq!(authorize(true, false, None, None), AccessDecision::Allow);
    }

    #[test]
    fn writes_require_matching_bearer() {
        assert_eq!(
            authorize(true, false, Some("secret"), None),
            AccessDecision::Unauthorized
        );
        assert_eq!(
            authorize(true, false, Some("secret"), Some("Bearer wrong")),
            AccessDecision::Unauthorized
        );
        assert_eq!(
            authorize(true, false, Some("secret"), Some("Bearer secret")),
            AccessDecision::Allow
        );
    }

    #[test]
    fn parse_bearer_forms() {
        assert_eq!(parse_bearer("Bearer abc"), Some("abc"));
        assert_eq!(parse_bearer("bearer abc"), Some("abc")); // case-insensitive scheme
        assert_eq!(parse_bearer("Bearer   abc  "), Some("abc")); // trimmed
        assert_eq!(parse_bearer("Basic abc"), None);
        assert_eq!(parse_bearer("Bearer "), None);
        assert_eq!(parse_bearer("abc"), None);
    }

    #[test]
    fn write_endpoint_classification() {
        assert!(is_write_endpoint("/episode"));
        assert!(is_write_endpoint("/retract"));
        assert!(is_write_endpoint("/set"));
        assert!(is_write_endpoint("/proposal/accept"));
        // aegis-2f4n: the three routes that were open under read-only mode and
        // bearer auth because WRITE_ENDPOINTS omitted them. Named so a regression
        // that drops any of them is a loud, specific failure.
        assert!(
            is_write_endpoint("/project"),
            "/project persists communities on persist:true"
        );
        assert!(
            is_write_endpoint("/overlay/write"),
            "/overlay/write commits a tx"
        );
        assert!(
            is_write_endpoint("/overlay/create"),
            "/overlay/create writes the graphs registry"
        );
        // ro_handler routes that nonetheless write via &Store interior mutability —
        // must stay writes; removing them (they "look" like reads) reopens them.
        assert!(is_write_endpoint("/shapes"));
        assert!(is_write_endpoint("/propose"));
        assert!(is_write_endpoint("/proposal/reject"));
        // Reads / unknown paths are not writes.
        assert!(!is_write_endpoint("/query"));
        assert!(!is_write_endpoint("/search"));
        assert!(!is_write_endpoint("/health"));
    }

    #[test]
    fn graph_store_classification_is_method_sensitive() {
        assert!(!is_write_request("/rdf-graph-store", "GET"));
        assert!(!is_write_request("/rdf-graph-store", "HEAD"));
        for method in ["PUT", "POST", "DELETE"] {
            assert!(is_write_request("/rdf-graph-store", method), "{method}");
        }
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn write_and_read_sets_are_disjoint() {
        for w in WRITE_ENDPOINTS {
            assert!(
                !READ_ENDPOINTS.contains(w),
                "{w} is in BOTH WRITE_ENDPOINTS and READ_ENDPOINTS — a route is one or the other"
            );
        }
    }

    /// Extract every path passed to `.route("<path>", ...)` in the router source.
    ///
    /// This reads `server.rs` via `include_str!`, so it embeds the router text at
    /// COMPILE TIME and runs in the default `cargo test` matrix — even though the
    /// server itself is a separate `onnx`-gated binary the matrix never builds.
    /// That is deliberate: the wiring being outside default CI is exactly how the
    /// endpoint list drifted (aegis-2f4n), so the enforcer must not need the
    /// wiring to be compiled.
    fn routes_in_server_source() -> Vec<String> {
        // Keep this source-level invariant independent of the server feature
        // matrix, but include router fragments that `server.rs` merges. A
        // route moved into one of those fragments is still a registered route.
        let sources = [
            ("server.rs", include_str!("server.rs")),
            ("server/align.rs", include_str!("server/align.rs")),
            ("server/assets.rs", include_str!("server/assets.rs")),
            (
                "server/graph_store.rs",
                include_str!("server/graph_store.rs"),
            ),
            (
                "server/snapshot_upload.rs",
                include_str!("server/snapshot_upload.rs"),
            ),
        ];
        // The list above is hand-maintained, and a fragment MISSING from it is
        // invisible to both tests here: its routes are never seen, so they are
        // never reported unclassified either. That is the failure this guard
        // must not have, so make the drift loud — every `#[path = "server/..."]`
        // module that server.rs declares must either be scanned above or be
        // listed as route-free.
        const NO_ROUTES: &[&str] = &[
            "server/admission.rs",
            "server/base.rs",
            "server/entity.rs",
            "server/handle.rs",
            "server/publication.rs",
            "server/query_usage.rs",
            "server/reason.rs",
            "server/request_middleware.rs",
            "server/service_description.rs",
            "server/tests.rs",
            "server/tools.rs",
            "server/update.rs",
            // WAL reset + passive checkpoint maintenance (aegis-raq1ok): a
            // startup call and a background tick, no routes.
            "server/wal_maintenance.rs",
        ];
        let server_rs = include_str!("server.rs");
        let mut declared = Vec::new();
        let mut rest = server_rs;
        while let Some(i) = rest.find("#[path = \"") {
            rest = &rest[i + "#[path = \"".len()..];
            if let Some(end) = rest.find('"') {
                declared.push(&rest[..end]);
                rest = &rest[end..];
            }
        }
        let scanned: std::collections::HashSet<&str> = sources.iter().map(|(n, _)| *n).collect();
        let unscanned: Vec<&str> = declared
            .iter()
            .copied()
            .filter(|m| !scanned.contains(m) && !NO_ROUTES.contains(m))
            .collect();
        assert!(
            unscanned.is_empty(),
            "server.rs declares router fragment(s) {unscanned:?} that this test does \
             not scan. Add each to `sources` (if it registers routes) or to NO_ROUTES \
             (if it does not). A fragment absent from both is UNCHECKED: its routes are \
             neither classified nor reported as unclassified."
        );
        let mut paths = Vec::new();
        for (_, src) in sources {
            let mut remaining = src;
            while let Some(idx) = remaining.find(".route(") {
                remaining = &remaining[idx + ".route(".len()..];
                let candidate = remaining.trim_start();
                if let Some(path) = candidate.strip_prefix('"')
                    && let Some(end) = path.find('"')
                {
                    paths.push(path[..end].to_string());
                    remaining = &path[end + 1..];
                }
            }
        }
        paths
    }

    #[test]
    fn write_endpoints_cover_every_route() {
        // THE ENFORCER (aegis-2f4n). Every route the server registers must be
        // classified as exactly one of write / read. A new route that is neither
        // fails this test, forcing a human to decide — because write-ness is not
        // inferable from the handler type in this crate (interior mutability).
        let routes = routes_in_server_source();
        assert!(
            routes.len() >= 40,
            "only found {} routes in server.rs — the .route() parse likely broke; \
             refusing to pass on a scan that found almost nothing",
            routes.len()
        );

        let mut unclassified = Vec::new();
        for r in &routes {
            let is_w = WRITE_ENDPOINTS.contains(&r.as_str());
            let is_r = READ_ENDPOINTS.contains(&r.as_str());
            if is_w && is_r {
                panic!("{r} is classified as BOTH write and read");
            }
            if !is_w && !is_r {
                unclassified.push(r.clone());
            }
        }
        assert!(
            unclassified.is_empty(),
            "these routes are registered in server.rs but classified in NEITHER \
             WRITE_ENDPOINTS nor READ_ENDPOINTS: {unclassified:?}. Classify each one. \
             If it mutates the store — including via an &Store method (interior \
             mutability) — it is a WRITE. When unsure, it is a write: that is fail-safe."
        );
    }

    #[test]
    fn no_stale_classification_entries() {
        // The other drift direction: a listed endpoint that is no longer a route.
        // Harmless for security, but it is how the original "kept in sync" comment
        // rotted — four ro_handler paths were listed as writes and nobody noticed,
        // which proved nothing was checking. Keep the lists honest.
        let routes = routes_in_server_source();
        let known: std::collections::HashSet<&str> = routes.iter().map(String::as_str).collect();
        let mut stale = Vec::new();
        for e in WRITE_ENDPOINTS.iter().chain(READ_ENDPOINTS.iter()) {
            if !known.contains(e) {
                stale.push(*e);
            }
        }
        assert!(
            stale.is_empty(),
            "these paths are classified but no longer registered as routes in \
             server.rs: {stale:?}. Remove them, or restore the route."
        );
    }

    /// Map each handler ident named in a `.route("<path>", get(h)/post(h))` line
    /// to its path. A handler may appear under more than one path only in theory;
    /// here each is unique, so last-wins is fine.
    fn handler_to_path() -> std::collections::HashMap<String, String> {
        let src = include_str!("server.rs");
        let mut map = std::collections::HashMap::new();
        for line in src.lines() {
            let Some(idx) = line.find(".route(\"") else {
                continue;
            };
            let rest = &line[idx + ".route(\"".len()..];
            let Some(end) = rest.find('"') else { continue };
            let path = rest[..end].to_string();
            // Pull the handler ident out of every `get(<ident>` / `post(<ident>`.
            for kw in ["get(", "post("] {
                let mut from = 0;
                while let Some(p) = rest[from..].find(kw) {
                    let start = from + p + kw.len();
                    let ident: String = rest[start..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !ident.is_empty() {
                        map.insert(ident, path.clone());
                    }
                    from = start;
                }
            }
        }
        map
    }

    /// Every `rw_handler!(<name>, ...)` / `ro_handler!(<name>, ...)` invocation in
    /// server.rs, as `(is_rw, name)`. Tolerant of the macro spanning lines (the
    /// name may be on the line after `ro_handler!(`).
    fn macro_tiers() -> Vec<(bool, String)> {
        let src = include_str!("server.rs");
        let mut out = Vec::new();
        for (marker, is_rw) in [("rw_handler!(", true), ("ro_handler!(", false)] {
            let mut from = 0;
            while let Some(p) = src[from..].find(marker) {
                let after = from + p + marker.len();
                let ident: String = src[after..]
                    .chars()
                    .skip_while(|c| c.is_whitespace())
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() {
                    out.push((is_rw, ident));
                }
                from = after;
            }
        }
        out
    }

    #[test]
    fn macro_tier_matches_write_classification() {
        // THE "ONE LAYER DOWN" ENFORCER (aegis-e163). The ro_handler!/rw_handler!
        // tier is a naming convention, not a type guarantee — `&Store` writes via
        // interior mutability, so an ro_handler! route CAN write. Five did
        // (shapes, propose, accept/reject proposal, overlay_create) and were open
        // to exactly the aegis-2f4n bypass one layer up. This pins the macro tier
        // to the read-only/auth classification so the two cannot silently diverge:
        //   - every rw_handler! route MUST be a write endpoint;
        //   - no ro_handler! route may be a write endpoint.
        // It cannot prove an ro_handler! tool never writes (only a real read-only
        // handle could); it makes a MISMATCH between the tier and the auth list a
        // build failure, which is the concrete drift this bug was.
        let paths = handler_to_path();
        let mut problems = Vec::new();
        for (is_rw, name) in macro_tiers() {
            let Some(path) = paths.get(&name) else {
                // A macro handler that is never routed is dead code; flag it so the
                // parser breaking (or a genuinely orphaned handler) is not silent.
                problems.push(format!(
                    "{name}: rw/ro handler is not registered on any .route()"
                ));
                continue;
            };
            let is_write = WRITE_ENDPOINTS.contains(&path.as_str());
            match (is_rw, is_write) {
                (true, false) => problems.push(format!(
                    "{path} ({name}) is rw_handler! but NOT in WRITE_ENDPOINTS — \
                     classify it as a write or it bypasses read-only/auth"
                )),
                (false, true) => problems.push(format!(
                    "{path} ({name}) is ro_handler! but IS in WRITE_ENDPOINTS — a \
                     write route registered read-only is the aegis-e163 mis-tier; \
                     register it rw_handler!"
                )),
                _ => {}
            }
        }
        assert!(
            problems.is_empty(),
            "macro tier disagrees with the write classification:\n  {}",
            problems.join("\n  ")
        );
    }

    #[test]
    fn auth_refusals_carry_a_json_body_not_a_bare_status() {
        // aegis-zodg0. `StatusCode::X.into_response()` yields a ZERO-LENGTH body,
        // so `curl -s` prints NOTHING and exits 0 and the caller reads a refusal
        // as "no results" / "the graph is empty". Measured on /project: it took
        // two round trips to establish it was auth at all, and /shapes had the
        // same silent 401 — so there was no correct per-route body to copy and
        // the defect was in this middleware, not in any one handler.
        //
        // Guarded as TEXT for the same reason write_endpoints_cover_every_route
        // is: server.rs sits behind the `onnx` feature and the default matrix
        // never compiles it, so a behavioural test here would not run. This does.
        let src = include_str!("server.rs");
        for bare in [
            "StatusCode::UNAUTHORIZED.into_response()",
            "StatusCode::FORBIDDEN.into_response()",
        ] {
            assert!(
                !src.contains(bare),
                "server.rs reintroduced `{bare}` — a bare status has an EMPTY body, \
                 which curl -s renders as silence and exit 0. Return \
                 (StatusCode::X, axum::Json(json!({{\"error\": ...}}))) instead."
            );
        }
        // And the replacement must actually be there — a file that stopped
        // refusing at all would pass the checks above vacuously.
        assert!(
            src.contains("missing_or_invalid_bearer_token"),
            "the 401 arm no longer emits its JSON reason code"
        );
        assert!(
            src.contains("server_is_read_only"),
            "the 403 arm no longer emits its JSON reason code"
        );
    }
}

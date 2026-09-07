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
    // aegis-rz75m6: retract-only repair path. Writes a retraction transaction
    // stamped repair:<ticket> when `apply` is true.
    "/retract/source",
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

/// Low-cardinality bearer generation for security telemetry. Never contains
/// token material or a caller-controlled label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthGeneration {
    /// A read endpoint required no credential.
    NotRequired,
    /// Writes are open because no current bearer is configured.
    OpenWrite,
    /// The current (primary) bearer authenticated the request.
    Current,
    /// The temporary previous bearer authenticated within its grace window.
    Previous,
}

/// Hard ceiling for a previous bearer grace window. Rotation is an operational
/// bridge, not a second permanent credential.
pub const MAX_PREVIOUS_BEARER_GRACE_SECONDS: u64 = 86_400;

/// Access decision plus the bounded generation label used by request logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authorization {
    /// Allow, unauthorized, or read-only.
    pub decision: AccessDecision,
    /// Present only for allowed requests.
    pub generation: Option<AuthGeneration>,
}

/// Startup-captured bearer policy. The previous token has an absolute expiry
/// taken directly from config, so restarts cannot renew a forgotten grace
/// credential into a permanent second key.
#[derive(Clone)]
pub struct BearerPolicy {
    current: Option<String>,
    previous: Option<ExpiringBearer>,
}

#[derive(Clone)]
struct ExpiringBearer {
    token: String,
    expires_at_epoch_secs: u64,
}

impl BearerPolicy {
    /// Build and validate a startup policy without ever formatting token values
    /// into an error. `now_epoch_secs` is injected for deterministic tests.
    pub fn new(
        current: Option<String>,
        previous: Option<String>,
        previous_expires_at_epoch_secs: Option<u64>,
        now_epoch_secs: u64,
    ) -> Result<Self, &'static str> {
        let previous = match (current.as_deref(), previous, previous_expires_at_epoch_secs) {
            (_, None, None) => None,
            (_, None, Some(_)) => {
                return Err("previous bearer expiry requires previous_auth_token");
            }
            (None, Some(_), _) => return Err("previous bearer requires current auth_token"),
            (Some(_), Some(_), None) => {
                return Err("previous bearer requires an absolute expiry");
            }
            (Some(current_token), Some(previous_token), Some(_))
                if constant_time_eq(current_token.as_bytes(), previous_token.as_bytes()) =>
            {
                return Err("current and previous bearer must be distinct");
            }
            (Some(_), Some(_), Some(expires_at)) if expires_at <= now_epoch_secs => {
                // Expiry completes rotation; stale cleanup must not prevent a restart.
                None
            }
            (Some(_), Some(_), Some(expires_at))
                if expires_at - now_epoch_secs > MAX_PREVIOUS_BEARER_GRACE_SECONDS =>
            {
                return Err("previous bearer expiry exceeds the 24-hour maximum");
            }
            (Some(_), Some(previous_token), Some(expires_at_epoch_secs)) => Some(ExpiringBearer {
                token: previous_token,
                expires_at_epoch_secs,
            }),
        };
        Ok(Self { current, previous })
    }

    /// The retained previous bearer's deadline, absent when it has expired.
    #[must_use]
    pub fn previous_expiry(&self) -> Option<u64> {
        self.previous
            .as_ref()
            .map(|previous| previous.expires_at_epoch_secs)
    }

    /// Whether write endpoints require authentication.
    #[must_use]
    pub fn requires_auth(&self) -> bool {
        self.current.is_some()
    }
}

/// Authorize against the startup bearer policy and identify which bounded key
/// generation matched. Both comparisons are constant-time.
#[must_use]
pub fn authorize_bearers(
    is_write: bool,
    read_only: bool,
    policy: &BearerPolicy,
    auth_header: Option<&str>,
    now_epoch_secs: u64,
) -> Authorization {
    if !is_write {
        return Authorization {
            decision: AccessDecision::Allow,
            generation: Some(AuthGeneration::NotRequired),
        };
    }
    if read_only {
        return Authorization {
            decision: AccessDecision::ReadOnly,
            generation: None,
        };
    }
    let Some(current) = policy.current.as_deref() else {
        return Authorization {
            decision: AccessDecision::Allow,
            generation: Some(AuthGeneration::OpenWrite),
        };
    };
    let presented = auth_header.and_then(parse_bearer);
    if presented.is_some_and(|token| constant_time_eq(token.as_bytes(), current.as_bytes())) {
        return Authorization {
            decision: AccessDecision::Allow,
            generation: Some(AuthGeneration::Current),
        };
    }
    if let (Some(presented), Some(previous)) = (presented, policy.previous.as_ref())
        && now_epoch_secs < previous.expires_at_epoch_secs
        && constant_time_eq(presented.as_bytes(), previous.token.as_bytes())
    {
        return Authorization {
            decision: AccessDecision::Allow,
            generation: Some(AuthGeneration::Previous),
        };
    }
    Authorization {
        decision: AccessDecision::Unauthorized,
        generation: None,
    }
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
mod tests;

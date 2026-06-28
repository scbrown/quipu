//! Access-control decisions for the Quipu REST server (hq-azs).
//!
//! The server bin (`quipu-server`) is feature-gated behind `onnx`, so its axum
//! wiring isn't exercised by the default CI matrix. The *policy* — is this a
//! write? is it allowed under read-only mode? does the bearer token match? — is
//! pure and lives here so it can be unit-tested without standing up a server.

/// The set of write endpoints. A request to one of these mutates the fact log
/// (or schema), so it is subject to read-only mode and bearer auth. Everything
/// else (query, search, entity reads, UI, health) is treated as read-only and
/// stays open. Kept in sync with the `rw_handler!` routes in `server.rs`.
pub const WRITE_ENDPOINTS: &[&str] = &[
    "/knot",
    "/episode",
    "/episodes/complete",
    "/retract",
    "/shapes",
    "/impact",
    "/propose",
    "/proposal/accept",
    "/proposal/reject",
    "/embed_backfill",
];

/// Whether `path` is a write endpoint subject to auth / read-only policy.
pub fn is_write_endpoint(path: &str) -> bool {
    WRITE_ENDPOINTS.contains(&path)
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
        assert!(is_write_endpoint("/proposal/accept"));
        // Reads / unknown paths are not writes.
        assert!(!is_write_endpoint("/query"));
        assert!(!is_write_endpoint("/search"));
        assert!(!is_write_endpoint("/health"));
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}

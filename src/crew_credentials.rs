//! Additive, audience-bound credential identity. Registries contain verifiers,
//! never bearer plaintext. Existing shared authentication is a separate policy.

use std::collections::HashSet;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Identity established by possession of an administrator-issued credential.
/// This does not attest which person/process actually possessed the credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrewPrincipal {
    pub iri: String,
    pub credential_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    version: u32,
    credentials: Vec<Credential>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credential {
    credential_id: String,
    principal: String,
    audience: String,
    token_sha256: String,
}

/// A validated immutable snapshot. A caller can replace its active registry only
/// after construction succeeds, retaining the previous snapshot on any failure.
#[derive(Clone, Default)]
pub struct CredentialRegistry {
    credentials: Vec<Credential>,
}

impl CredentialRegistry {
    /// Parse a bounded registry for a single service. Error strings never include
    /// file contents: even a misplaced plaintext token must not leak in a parser
    /// diagnostic. Issuance checks the principal against the operator's graph;
    /// loading checks syntax only and does not make a network request.
    pub fn parse(bytes: &[u8], audience: &str) -> Result<Self, &'static str> {
        if bytes.len() > 1024 * 1024 {
            return Err("credential registry exceeds one MiB");
        }
        let file: RegistryFile =
            serde_json::from_slice(bytes).map_err(|_| "invalid credential registry JSON")?;
        if file.version != 1 {
            return Err("unsupported credential registry version");
        }
        if file.credentials.len() > 1024 {
            return Err("credential registry exceeds 1024 entries");
        }
        let mut ids = HashSet::new();
        let mut hashes = HashSet::new();
        for c in &file.credentials {
            if c.credential_id.is_empty()
                || c.credential_id.len() > 128
                || !c
                    .credential_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err("invalid credential identifier");
            }
            if c.principal.len() > 2048 || oxrdf::NamedNode::new(&c.principal).is_err() {
                return Err("principal must be an absolute IRI of at most 2048 bytes");
            }
            if audience.is_empty() || c.audience != audience {
                return Err("credential audience does not match this service");
            }
            if c.token_sha256.len() != 64
                || !c
                    .token_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("credential verifier must be lowercase SHA-256 hex");
            }
            if !ids.insert(&c.credential_id) || !hashes.insert(&c.token_sha256) {
                return Err("duplicate credential identifier or verifier");
            }
        }
        Ok(Self {
            credentials: file.credentials,
        })
    }

    /// Resolve a bearer to its registry-bound identity. Scan every verifier so
    /// lookup does not reveal which registry position matched. No caller-supplied
    /// actor, task, source, or identity header participates in this decision.
    #[must_use]
    pub fn authenticate(&self, header: Option<&str>) -> Option<CrewPrincipal> {
        let token = crate::http_auth::parse_bearer(header?)?;
        // The issuer uses 32 random bytes encoded as 64 hex characters. Accept
        // other nonempty opaque presentations for verification without echoing
        // them; hashing does not turn a low-entropy password into a safe token.
        if token.len() > 4096 {
            return None;
        }
        let digest = format!("{:x}", Sha256::digest(token.as_bytes()));
        let mut matched = None;
        for c in &self.credentials {
            let difference = digest
                .bytes()
                .zip(c.token_sha256.bytes())
                .fold(0u8, |diff, (a, b)| diff | (a ^ b));
            if difference == 0 {
                matched = Some(CrewPrincipal {
                    iri: c.principal.clone(),
                    credential_id: c.credential_id.clone(),
                });
            }
        }
        matched
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.credentials.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }
}

#[cfg(test)]
mod tests;

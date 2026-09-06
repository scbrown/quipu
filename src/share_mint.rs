//! Minting a producer attestation into a share manifest (aegis-tadzdf).
//!
//! Extracted from `share.rs` rather than grandfathered past the 500-line ratchet
//! — the call malcolm made on `cli.rs` and I made on `share_attestation.rs` for
//! #171. Minting is a subject of its own besides: it is the only place in the
//! producer path that touches a PRIVATE KEY.
//!
//! The consumer half is `share_attestation`. The two must agree on exactly which
//! fields are signed, which is why both build the payload from the same four
//! manifest identity fields and nothing else.

use crate::error::{Error, Result};
use crate::share::ShareManifest;
use serde::{Deserialize, Serialize};

/// Who the producer says it is, for a minted attestation.
///
/// The key comes from `signing::load_or_generate` -- the SAME host-file custody
/// the governance plane already uses -- rather than a second scheme invented
/// here. Custody is a file at `QUIPU_SIGNING_KEY`, created 0600 on first use;
/// that is v1 and is documented as such in `signing.rs`, not hardened here.
#[derive(Debug, Clone)]
pub struct AttestOptions {
    /// PKCS#8 key path.
    pub key_path: std::path::PathBuf,
    /// Producer agent identity.
    pub agent: String,
    /// Producer session identity.
    pub session: String,
    /// Who introduced this session. Self-asserted in a share, hence `claimed`.
    pub introducer: String,
    /// Seconds since epoch at signing.
    pub issued_at_epoch: u64,
    /// Seconds since epoch at which the binding expires.
    pub expires_at_epoch: u64,
    /// Envelope nonce (32+ chars).
    pub nonce: String,
}

/// A producer attestation carried inside a share manifest (aegis-tadzdf).
///
/// The binding is the PUBLIC half only — `SessionBinding` holds no private key.
/// It travels so a consumer can verify the signature at the `claimed` tier
/// without having been told to trust the key; it does NOT cause the key to be
/// registered, and an import that registered it would make `attested` mean
/// "arrived with a self-signed claim" (see `share_attestation`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareAttestation {
    /// The signature envelope over the manifest's four identity fields.
    pub envelope: crate::session_attestation::AttestationEnvelope,
    /// The producer's self-asserted public binding. Self-asserted is the point:
    /// it is what makes this `claimed` and not `attested`.
    pub binding: crate::session_attestation::SessionBinding,
}

/// Sign this manifest's identity with the producer's key (aegis-tadzdf).
///
/// Signs exactly the four fields `share_attestation::verify_attestation` rebuilds
/// on the consumer side. Building the payload from anything else would produce a
/// signature over a statement about a different share.
pub fn mint_attestation(
    manifest: &ShareManifest,
    opts: &AttestOptions,
) -> Result<ShareAttestation> {
    use crate::session_attestation::{
        AttestationEnvelope, SHARE_V1, SessionBinding, ShareBinding, SignedBinding,
        canonical_message,
    };
    let key = crate::signing::load_or_generate(&opts.key_path)?;
    let public = crate::signing::public_key_hex(&key);
    let binding = SessionBinding::new(
        opts.agent.clone(),
        opts.session.clone(),
        public,
        opts.introducer.clone(),
        opts.issued_at_epoch,
        opts.expires_at_epoch,
    )?;
    // VALIDATE AT MINT TIME, not only at import. Measured 2026-09-05: a share was
    // minted with a non-hex nonce, written to disk and reported as success, and
    // the defect surfaced only when a DIFFERENT store tried to import it --
    // "attestation nonce must be 128-bit lowercase hex", from the consumer, about
    // bytes the producer chose. A producer that cannot be imported should learn
    // that from its own command.
    if opts.nonce.len() != 32
        || opts.nonce != opts.nonce.to_ascii_lowercase()
        || !opts.nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::InvalidValue(
            "attestation nonce must be 128-bit lowercase hex (32 hex characters): a share \
             minted with any other nonce is refused by every importer"
                .into(),
        ));
    }
    let mut envelope = AttestationEnvelope {
        version: SHARE_V1.into(),
        key_id: binding.key_id.clone(),
        session: binding.session.clone(),
        introducer: binding.introducer.clone(),
        issued_at_epoch: opts.issued_at_epoch,
        nonce: opts.nonce.clone(),
        signature: String::new(),
    };
    let payload = SignedBinding::Share(ShareBinding {
        share_id: &manifest.share_id,
        graph_hash: &manifest.graph_hash,
        shapes_hash: &manifest.shapes_hash,
        tx_anchor: manifest.tx_anchor,
    });
    envelope.signature = crate::signing::sign_hex(&key, &canonical_message(&envelope, &payload));
    Ok(ShareAttestation { envelope, binding })
}

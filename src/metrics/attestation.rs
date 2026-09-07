//! Bounded, process-local observations of imports and signature verification.

use super::*;

type Counts = Mutex<BTreeMap<(&'static str, &'static str), u64>>;

#[derive(Default)]
pub(super) struct AttestationMetrics {
    imports: Counts,
    verifications: Counts,
}

impl AttestationMetrics {
    pub(super) fn render(&self, out: &mut String) {
        out.push_str(
            "# HELP quipu_share_import_total Import decisions by outcome and verified trust tier.\n\
             # TYPE quipu_share_import_total counter\n",
        );
        for ((outcome, tier), n) in self.imports.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_share_import_total{{outcome=\"{outcome}\",tier=\"{tier}\"}} {n}"
            );
        }
        out.push_str(
            "# HELP quipu_attestation_verify_total Verification decisions by signed binding domain and result.\n\
             # TYPE quipu_attestation_verify_total counter\n",
        );
        for ((binding, result), n) in self.verifications.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_attestation_verify_total{{binding=\"{binding}\",result=\"{result}\"}} {n}"
            );
        }
    }
}

/// Counts once on every exit, including errors after verification but before staging.
pub(crate) struct ImportObservation {
    tier: &'static str,
    outcome: &'static str,
}

impl ImportObservation {
    pub(crate) fn new() -> Self {
        Self {
            tier: "unverified",
            outcome: "error",
        }
    }

    pub(crate) fn tier(&mut self, tier: &str) {
        self.tier = match tier {
            "transport" => "transport",
            "claimed" => "claimed",
            "attested" => "attested",
            _ => "unverified",
        };
    }

    pub(crate) fn outcome(&mut self, outcome: &str) {
        self.outcome = match outcome {
            "staged" => "staged",
            "quarantined" => "quarantined",
            "unchanged" => "unchanged",
            _ => "error",
        };
    }
}

impl Drop for ImportObservation {
    fn drop(&mut self) {
        *metrics()
            .attestation
            .imports
            .lock()
            .unwrap()
            .entry((self.outcome, self.tier))
            .or_default() += 1;
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
pub(crate) enum VerificationResult {
    Ok,
    Badsig,
    Replay,
    Revoked,
    Unbound,
    Skew,
    Invalid,
    Error,
}

#[cfg(not(target_arch = "wasm32"))]
impl VerificationResult {
    const fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Badsig => "badsig",
            Self::Replay => "replay",
            Self::Revoked => "revoked",
            Self::Unbound => "unbound",
            Self::Skew => "skew",
            Self::Invalid => "invalid",
            Self::Error => "error",
        }
    }
}

/// Set the verdict at the check that decides it, never by parsing error prose.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct VerificationObservation {
    binding: &'static str,
    pub(crate) result: VerificationResult,
}

#[cfg(not(target_arch = "wasm32"))]
impl VerificationObservation {
    pub(crate) fn new(payload: &crate::session_attestation::SignedBinding<'_>) -> Self {
        use crate::session_attestation::SignedBinding;
        Self {
            binding: match payload {
                SignedBinding::Write(_) => "write",
                SignedBinding::Share(_) => "share",
            },
            result: VerificationResult::Error,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for VerificationObservation {
    fn drop(&mut self) {
        *metrics()
            .attestation
            .verifications
            .lock()
            .unwrap()
            .entry((self.binding, self.result.label()))
            .or_default() += 1;
    }
}

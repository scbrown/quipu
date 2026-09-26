//! `quipu audit replay <verdict>` — re-derive one recorded gate decision
//! against the store as of its transaction (GS6).
//!
//! Three bases, chosen by what the store holds, and named in the report so a
//! reader never mistakes one for another:
//!
//! - **Committed.** The judged write landed, so its evidence is in the facts.
//!   Rebuild the store as of that write's transaction and judge the verdict's
//!   policy over its target again — rules and data both as-of.
//! - **Quarantined.** The judged write was refused (GS2 rolled it back), and a
//!   quarantine entry backs the verdict (`quarantine.rs`). Rebuild the store as
//!   of the entry's `base_tx`, apply the attempted delta — the sealed one under
//!   full retention, or one PRESENTED and verified against the digest — and run
//!   the real write path with the gate on. Re-derived means all four agree:
//!   the write is refused again, the verdict's outcome comes back, the rule-set
//!   digest matches (same rules), and the post-state digest matches (same
//!   state, byte for byte).
//! - **Attestation only.** Nothing to re-run: a refusal recorded before the
//!   quarantine existed, a digest-only entry with no delta presented, or one
//!   whose content was purged. What CAN be checked still is — that the policy
//!   was in force at the instant, and the entry's seal.
//!
//! **The rebuild is a throwaway in-memory copy** (`sqlite3_serialize` of the
//! live store, then every fact and transaction after the instant removed and
//! every later retraction undone). Nothing a replay does reaches the live
//! store, the attempted delta included — it is applied only to the copy, and
//! the copy is dropped. The cost is the store's size in memory, once per
//! replay; this is an audit path, not a write path.
//!
//! **Only an attempt can be replayed, not a writer's intent.** A refused write
//! inside a multi-graph batch judged a post-state holding the batch's earlier,
//! also-rolled-back graphs, which no quarantine entry carries. The post-state
//! digest catches it and the replay reports the divergence rather than
//! claiming a re-derivation it did not do.

use super::guard::PolicyRegistry;
use super::quarantine::{self, AttemptedDelta, Entry, ReplayCapture};
use crate::error::{Error, Result};
use crate::namespace::DEFAULT_BASE_NS;
use crate::sparql::{self, QueryResult, TemporalContext};
use crate::store::Store;
use crate::types::Value;

/// What a replay was able to rest on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Basis {
    /// The judged write committed at this transaction.
    Committed {
        /// The write's transaction.
        write_tx: i64,
    },
    /// The judged write was refused; this quarantine entry backs the verdict.
    Quarantined {
        /// The entry's row id.
        entry: i64,
        /// Where the delta came from: `sealed` or `presented`.
        delta: &'static str,
    },
    /// Nothing could be re-run; the reason says why.
    AttestationOnly(String),
}

/// Whether a quarantine entry's seal verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seal {
    /// Valid under a key registered (or held) for the entry's verifier.
    Valid,
    /// A key is known and the seal does not verify under any of them.
    Invalid,
    /// No key is known for the verifier, so nothing can be said.
    Unverifiable,
}

/// The replay of one verdict (one attempt, for a quarantined verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictReplay {
    /// The verdict IRI.
    pub verdict: String,
    /// Its policy.
    pub policy: String,
    /// Its target.
    pub target: String,
    /// The outcome it records.
    pub recorded: String,
    /// What the replay rested on.
    pub basis: Basis,
    /// The outcome the replay produced, when it ran.
    pub replayed: Option<String>,
    /// Quarantined: whether the replayed write was refused again.
    pub refused: Option<bool>,
    /// Quarantined: whether the rule-set digest matched.
    pub same_rules: Option<bool>,
    /// Quarantined: whether the post-state digest matched.
    pub same_post_state: Option<bool>,
    /// Whether the verdict's policy was in force at the instant.
    pub rules_in_force: Option<bool>,
    /// Quarantined or attestation from an entry: the entry's seal.
    pub seal: Option<Seal>,
}

impl VerdictReplay {
    fn new(info: &VerdictInfo, basis: Basis) -> Self {
        Self {
            verdict: info.iri.clone(),
            policy: info.policy.clone(),
            target: info.target.clone(),
            recorded: info.outcome.clone(),
            basis,
            replayed: None,
            refused: None,
            same_rules: None,
            same_post_state: None,
            rules_in_force: None,
            seal: None,
        }
    }

    /// Did the replay re-derive the recorded verdict?
    #[must_use]
    pub fn rederived(&self) -> bool {
        let outcome = self.replayed.as_deref() == Some(self.recorded.as_str());
        match self.basis {
            Basis::Committed { .. } => outcome,
            Basis::Quarantined { .. } => {
                outcome
                    && self.refused == Some(true)
                    && self.same_rules == Some(true)
                    && self.same_post_state == Some(true)
                    && self.seal != Some(Seal::Invalid)
            }
            Basis::AttestationOnly(_) => false,
        }
    }

    /// Does the replay CONTRADICT the record? Only this fails the CLI: an
    /// attestation-only replay is incomplete, not wrong.
    #[must_use]
    pub fn contradicts(&self) -> bool {
        if self.seal == Some(Seal::Invalid) || self.rules_in_force == Some(false) {
            return true;
        }
        !matches!(self.basis, Basis::AttestationOnly(_)) && !self.rederived()
    }

    /// The operator-facing line.
    #[must_use]
    pub fn line(&self) -> String {
        let status = if self.contradicts() {
            "DIVERGED"
        } else if self.rederived() {
            "re-derived"
        } else {
            "attestation only"
        };
        let basis = match &self.basis {
            Basis::Committed { write_tx } => format!("committed write, tx {write_tx}"),
            Basis::Quarantined { entry, delta } => {
                format!("refused write, quarantine entry {entry}, {delta} delta")
            }
            Basis::AttestationOnly(why) => why.clone(),
        };
        let mut checks = Vec::new();
        if let Some(r) = &self.replayed {
            checks.push(format!("outcome {} (recorded {})", r, self.recorded));
        }
        for (name, v) in [
            ("refused again", self.refused),
            ("same rules", self.same_rules),
            ("same post-state", self.same_post_state),
            ("policy in force", self.rules_in_force),
        ] {
            if let Some(v) = v {
                checks.push(format!("{name}: {}", if v { "yes" } else { "NO" }));
            }
        }
        if let Some(seal) = self.seal {
            checks.push(format!("seal: {seal:?}").to_lowercase());
        }
        format!(
            "{status}: {verdict} [{policy} on {target}] — {basis}; {checks}",
            verdict = self.verdict,
            policy = self.policy,
            target = self.target,
            checks = checks.join(", "),
        )
    }

    /// The JSON form.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let (basis, detail) = match &self.basis {
            Basis::Committed { write_tx } => ("committed", serde_json::json!(write_tx)),
            Basis::Quarantined { entry, delta } => (
                "quarantined",
                serde_json::json!({ "entry": entry, "delta": delta }),
            ),
            Basis::AttestationOnly(why) => ("attestation-only", serde_json::json!(why)),
        };
        serde_json::json!({
            "verdict": self.verdict,
            "policy": self.policy,
            "target": self.target,
            "recorded": self.recorded,
            "basis": basis,
            "basis_detail": detail,
            "replayed": self.replayed,
            "refused_again": self.refused,
            "same_rules": self.same_rules,
            "same_post_state": self.same_post_state,
            "rules_in_force": self.rules_in_force,
            "seal": self.seal.map(|s| format!("{s:?}").to_lowercase()),
            "rederived": self.rederived(),
            "contradicts": self.contradicts(),
        })
    }
}

/// The recorded verdict, read back from its facts.
struct VerdictInfo {
    iri: String,
    policy: String,
    target: String,
    outcome: String,
    at: String,
    recorded_tx: i64,
}

/// Replay the verdict `verdict` (an IRI, or its local `verdict_…` name).
///
/// `presented` is a delta in [`AttemptedDelta`] JSON. With one, only the
/// quarantine entries whose attempt digest it matches are replayed, from it;
/// a delta matching none is refused as not the attempt that was judged — the
/// tampered-delta case. Returns one replay per quarantined attempt, or one
/// for a committed or attestation-only verdict.
///
/// # Errors
/// No such verdict; a presented delta that matches no entry; a store with
/// attachments mounted (the rebuild copies the main database only); store
/// and SPARQL errors.
pub fn replay_verdict(
    store: &Store,
    verdict: &str,
    presented: Option<&str>,
) -> Result<Vec<VerdictReplay>> {
    let info = verdict_info(store, verdict)?;
    let entries = quarantine::entries(store, Some(&info.iri))?;

    if let Some(json) = presented {
        let delta = AttemptedDelta::parse(json)?;
        let hash = delta.hash();
        let matching: Vec<&Entry> = entries.iter().filter(|e| e.attempt == hash).collect();
        if matching.is_empty() {
            return Err(Error::InvalidValue(format!(
                "the presented delta hashes to {hash}, which is not the digest of any attempt \
                 quarantined for {} ({} on record) — it is not the delta the gate judged",
                info.iri,
                entries.len()
            )));
        }
        return matching
            .into_iter()
            .map(|e| replay_attempt(store, &info, e, &delta, "presented"))
            .collect();
    }

    if !entries.is_empty() {
        let mut out = Vec::new();
        for entry in &entries {
            let sealed = quarantine::sealed_delta(store, &entry.attempt)?;
            let delta = match sealed.as_deref().map(AttemptedDelta::parse) {
                Some(Ok(d)) if d.hash() == entry.attempt => d,
                Some(_) => {
                    // A held delta that no longer hashes to the sealed digest was
                    // altered after the fact. Replaying it would re-derive a
                    // different attempt, so it is reported, not used.
                    let mut r = attestation(
                        store,
                        &info,
                        entry,
                        "sealed delta does not match its digest (tampered)",
                    )?;
                    r.seal = Some(Seal::Invalid);
                    out.push(r);
                    continue;
                }
                None => {
                    let why = if entry.purged_at.is_some() {
                        "refused, content purged — present the delta with --delta to re-derive"
                    } else {
                        "refused, digest-only retention — present the delta with --delta to re-derive"
                    };
                    out.push(attestation(store, &info, entry, why)?);
                    continue;
                }
            };
            out.push(replay_attempt(store, &info, entry, &delta, "sealed")?);
        }
        return Ok(out);
    }

    // No quarantine entry: the verdict judged a write that committed, or a
    // refusal recorded before the quarantine existed (or with it off).
    let Some(write_tx) = committed_write(store, &info)? else {
        let mut r = VerdictReplay::new(
            &info,
            Basis::AttestationOnly(
                "refused before the quarantine recorded it: the attempt was rolled back and \
                 nothing retained it"
                    .into(),
            ),
        );
        let copy = rebuild_as_of(store, info.recorded_tx - 1)?;
        r.rules_in_force = Some(PolicyRegistry::build(&copy)?.in_force(&info.policy));
        return Ok(vec![r]);
    };
    let copy = rebuild_as_of(store, write_tx)?;
    let mut r = VerdictReplay::new(&info, Basis::Committed { write_tx });
    let replayed = PolicyRegistry::build(&copy)?.judge(&copy, &info.policy, &info.target)?;
    r.rules_in_force = Some(replayed.is_some());
    r.replayed = replayed;
    Ok(vec![r])
}

/// Rebuild the store as of `entry.base_tx`, apply `delta`, run the gate.
fn replay_attempt(
    store: &Store,
    info: &VerdictInfo,
    entry: &Entry,
    delta: &AttemptedDelta,
    source: &'static str,
) -> Result<VerdictReplay> {
    let mut copy = rebuild_as_of(store, entry.base_tx)?;
    copy.governance_config.enforce_on_write = true;
    copy.governance_config.quarantine.enabled = true;
    copy.principal_chain.clone_from(&entry.chain);
    copy.gate_clock = Some(entry.gate_now);
    copy.replay_capture = Some(ReplayCapture::default());
    let (datums, graph) = delta.to_datums(&copy)?;
    let result = copy.transact_to_graph(
        &datums,
        &entry.at,
        entry.actor.as_deref(),
        entry.source.as_deref(),
        graph,
    );
    let captured = copy.replay_capture.take().unwrap_or_default();

    let mut r = VerdictReplay::new(
        info,
        Basis::Quarantined {
            entry: entry.id,
            delta: source,
        },
    );
    r.refused = Some(matches!(result, Err(Error::PolicyDenied(_))));
    r.replayed = captured
        .verdicts
        .iter()
        .find(|v| v.predicate_id == info.policy && v.target_ref == info.target)
        .map(|v| v.outcome.clone());
    // No capture means the replayed write was not refused by the policy gate,
    // so there is no post-state it judged — which is itself the divergence.
    r.same_rules = Some(
        captured
            .quarantine
            .as_ref()
            .is_some_and(|c| c.rules_digest == entry.rules_digest),
    );
    r.same_post_state = Some(
        captured
            .quarantine
            .as_ref()
            .is_some_and(|c| c.post_digest == entry.post_digest),
    );
    r.seal = Some(check_seal(store, entry)?);
    Ok(r)
}

/// An attestation-only replay of a quarantined entry: the policy in force as
/// of the pre-state, and the seal.
fn attestation(
    store: &Store,
    info: &VerdictInfo,
    entry: &Entry,
    why: &str,
) -> Result<VerdictReplay> {
    let mut r = VerdictReplay::new(info, Basis::AttestationOnly(why.to_string()));
    let copy = rebuild_as_of(store, entry.base_tx)?;
    let registry = PolicyRegistry::build(&copy)?;
    r.rules_in_force = Some(registry.in_force(&info.policy));
    r.seal = Some(check_seal(store, entry)?);
    Ok(r)
}

/// A throwaway in-memory copy of `store` as of transaction `tx`.
///
/// Exactly the store's own as-of predicate (`tx <= N AND (valid_to IS NULL OR
/// retracted_tx > N)`, quipu #83), applied destructively to a copy: later facts
/// and transactions removed, later retractions undone. The quarantine tables
/// are emptied in the copy too — a replay must not be able to read its own
/// answer. Runtime configuration is carried over, since the gate reads it.
pub(crate) fn rebuild_as_of(store: &Store, tx: i64) -> Result<Store> {
    if store.has_attachments() {
        return Err(Error::InvalidValue(
            "verdict replay rebuilds the main database only; open the store without \
             attachments to replay"
                .into(),
        ));
    }
    let mut copy = Store::open_from_bytes(&store.serialize_db()?)?;
    copy.conn.execute_batch(&format!(
        "DELETE FROM facts WHERE tx > {tx};
         UPDATE facts SET valid_to = NULL, retracted_tx = NULL WHERE retracted_tx > {tx};
         DELETE FROM transaction_auth WHERE tx > {tx};
         DELETE FROM transactions WHERE id > {tx};
         DELETE FROM denial_quarantine;
         DELETE FROM quarantine_deltas;"
    ))?;
    copy.invalidate_read_model();
    copy.governance_config.clone_from(&store.governance_config);
    copy.owl_config.clone_from(&store.owl_config);
    copy.labels_config.clone_from(&store.labels_config);
    copy.search_config.clone_from(&store.search_config);
    copy.shacl_config.clone_from(&store.shacl_config);
    copy.base_ns.clone_from(&store.base_ns);
    Ok(copy)
}

/// Read a verdict's recorded fields back from its facts.
fn verdict_info(store: &Store, verdict: &str) -> Result<VerdictInfo> {
    let iri = if verdict.contains(':') {
        verdict.to_string()
    } else {
        format!("{DEFAULT_BASE_NS}{verdict}")
    };
    let missing = || Error::InvalidValue(format!("no aegis:Verdict <{iri}> in this store"));
    let id = store.lookup(&iri)?.ok_or_else(missing)?;
    let facts = store.entity_facts(id)?;
    if facts.is_empty() {
        return Err(missing());
    }
    let field = |name: &str| -> Result<String> {
        let attr = store.lookup(&format!("{DEFAULT_BASE_NS}{name}"))?;
        facts
            .iter()
            .find(|f| Some(f.attribute) == attr)
            .and_then(|f| match &f.value {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .ok_or_else(|| Error::InvalidValue(format!("verdict <{iri}> has no aegis:{name}")))
    };
    Ok(VerdictInfo {
        policy: field("predicateId")?,
        target: field("targetRef")?,
        outcome: field("outcome")?,
        at: facts[0].valid_from.clone(),
        recorded_tx: facts.iter().map(|f| f.tx).min().unwrap_or(0),
        iri,
    })
}

/// The committed write a verdict judged: the latest transaction before the
/// verdict was recorded that wrote a fact about its target at the verdict's
/// instant. `None` when there is none — a refusal, whose write never landed.
fn committed_write(store: &Store, info: &VerdictInfo) -> Result<Option<i64>> {
    let Some(target) = store.lookup(&info.target)? else {
        return Ok(None);
    };
    let tx: Option<i64> = store.conn.query_row(
        "SELECT MAX(f.tx) FROM facts f JOIN transactions t ON t.id = f.tx \
         WHERE f.e = ?1 AND f.tx < ?2 AND t.timestamp = ?3",
        rusqlite::params![target, info.recorded_tx, info.at],
        |r| r.get(0),
    )?;
    Ok(tx)
}

/// Verify an entry's seal against the keys known for its verifier: every
/// `aegis:VerifierRegistration` key, and the store's own identity if it
/// attests as that verifier.
fn check_seal(store: &Store, entry: &Entry) -> Result<Seal> {
    let mut keys = Vec::new();
    if let Some(identity) = store.signing_identity()
        && identity.verifier == entry.verifier
    {
        keys.push(identity.public_key_hex());
    }
    let verifier = entry.verifier.replace(['"', '\\', '\n'], "");
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?k WHERE {{ \
         ?r a a:VerifierRegistration ; a:verifier \"{verifier}\" ; a:publicKey ?k }}"
    );
    if let QueryResult::Select { rows, .. } =
        sparql::query_temporal(store, &q, &TemporalContext::default())?
    {
        keys.extend(rows.iter().filter_map(|r| match r.get("k") {
            Some(Value::Str(k)) => Some(k.clone()),
            _ => None,
        }));
    }
    if keys.is_empty() {
        return Ok(Seal::Unverifiable);
    }
    let message = quarantine::seal_message(entry);
    Ok(
        if keys
            .iter()
            .any(|k| crate::signing::verify_hex(k, &message, &entry.seal))
        {
            Seal::Valid
        } else {
            Seal::Invalid
        },
    )
}

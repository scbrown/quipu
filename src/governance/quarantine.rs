//! Denial quarantine — the evidence a refusal needs to be re-derived, kept
//! OUTSIDE the governed graph (GS6 for denials).
//!
//! ## The gap this closes
//!
//! GS2 rolls a denied write back and keeps its verdict. That ordering is the
//! whole point of `verdict_facts` — and it is also why a denial could only be
//! replayed as an ATTESTATION: the rules in force at the instant can be checked,
//! but the post-state the gate judged is gone, so the outcome cannot be
//! re-derived. An accepted write leaves its evidence in the facts it wrote; a
//! refused one left nothing (`examples/census/phase6.rs`, RQ5: 50 of 50
//! satisfied verdicts re-derived, 6 denials "rules in force verified" only).
//!
//! ## What is kept, and where
//!
//! Two plain tables (`schema.rs`), never facts. No SPARQL pattern, search,
//! fact read or read model reaches them; only `quipu audit replay` does. So
//! GS2 holds unchanged: the refused content never enters the governed graph
//! and is never visible to an ordinary query.
//!
//! Per verdict of a refused write, by default (**digest-only**):
//!
//! - `attempt` — sha256 over the attempted delta in its canonical form
//!   ([`AttemptedDelta`]): the writer's datums, IRIs not term ids, and the
//!   target graph. OWL inference is excluded — replay re-derives it.
//! - `post_digest` — sha256 over the post-state the gate evaluated
//!   ([`post_state_digest`]). This is what makes a replay a re-derivation
//!   rather than a re-enactment: equal digests say the rebuilt store plus the
//!   delta IS the state the original gate judged, byte for byte.
//! - `rules_digest` — [`PolicyRegistry::digest`] of the rule set in force.
//! - `base_tx`, the write's timestamp, actor, source and chain, and the gate's
//!   clock — everything the write path reads besides the facts.
//!
//! With the graph opted into **full retention** (`retain_full`), the canonical
//! delta itself is also kept, in `quarantine_deltas`, content-addressed by
//! `attempt`. Each entry is sealed: ed25519 by the store's signing identity
//! over the entry's canonical fields ([`seal_message`]), so neither the digests
//! nor the verdict they back can be swapped under a valid seal, and a sealed
//! delta that no longer hashes to its `attempt` is detectably tampered.
//!
//! ## What is deliberately NOT done
//!
//! The verdict's own evidence hash is unchanged. Binding `attempt` into it
//! would change every verdict's seal and break the escalation router, which
//! binds decisions to that hash. The quarantine entry points AT the verdict
//! instead, and carries its own seal.
//!
//! The post-state digest is over this store's term ids. A respace since the
//! denial changes every id, so a replay across one reports the post-state as
//! different — which it is, in the only sense the digest can see. Stated, not
//! hidden.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::guard::PolicyRegistry;
use super::verdict_facts::PendingVerdict;
use crate::error::{Error, Result};
use crate::schema::{ROOT_GRAPH, ROOT_GRAPH_IRI};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// The canonical delta's format tag. Part of the hashed bytes, so a future
/// format can never collide with this one.
pub const DELTA_FORMAT: &str = "quipu-refused-delta/v1";

/// A refused write's datums in canonical, store-independent form.
///
/// IRIs rather than term ids, so the same attempt hashes the same in the store
/// that refused it and in the as-of copy a replay rebuilds — and so a writer
/// who kept its own record of the attempt can present it later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptedDelta {
    /// Always [`DELTA_FORMAT`].
    pub format: String,
    /// The graph IRI the write targeted (`urn:quipu:graph:root` for ROOT).
    pub graph: String,
    /// The writer's datums, in the order written.
    pub datums: Vec<DeltaDatum>,
}

/// One datum of an [`AttemptedDelta`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaDatum {
    /// Entity IRI.
    pub e: String,
    /// Attribute IRI.
    pub a: String,
    /// Value.
    pub v: DeltaValue,
    /// Valid-time start.
    pub valid_from: String,
    /// Valid-time end, if the writer closed the interval.
    #[serde(default)]
    pub valid_to: Option<String>,
    /// `assert`, `retract` or `tombstone`.
    pub op: String,
}

/// A [`Value`] with its term id replaced by the IRI it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeltaValue {
    /// IRI reference.
    Ref(String),
    /// String literal.
    Str(String),
    /// Integer.
    Int(i64),
    /// Float, as its Rust round-trip rendering: JSON numbers would lose NaN
    /// and the infinities, and a canonical form that cannot say a value is
    /// not canonical.
    Float(String),
    /// Boolean.
    Bool(bool),
    /// Raw bytes, hex.
    Bytes(String),
    /// Language-tagged literal.
    Lang {
        /// Lexical form, without the tag.
        lexical: String,
        /// BCP47 tag.
        lang: String,
    },
    /// Literal with an explicit datatype.
    Typed {
        /// Lexical form, verbatim.
        lexical: String,
        /// Datatype IRI.
        datatype: String,
    },
}

impl AttemptedDelta {
    /// The canonical form of `datums` as written to `graph`.
    pub fn from_datums(store: &Store, datums: &[Datum], graph: i64) -> Result<Self> {
        let mut out = Vec::with_capacity(datums.len());
        for d in datums {
            let v = match &d.value {
                Value::Ref(id) => DeltaValue::Ref(store.resolve(*id)?),
                Value::Str(s) => DeltaValue::Str(s.clone()),
                Value::Int(i) => DeltaValue::Int(*i),
                Value::Float(f) => DeltaValue::Float(f.to_string()),
                Value::Bool(b) => DeltaValue::Bool(*b),
                Value::Bytes(b) => DeltaValue::Bytes(hex::encode(b)),
                Value::Lang { lexical, lang } => DeltaValue::Lang {
                    lexical: lexical.clone(),
                    lang: lang.clone(),
                },
                Value::Typed { lexical, datatype } => DeltaValue::Typed {
                    lexical: lexical.clone(),
                    datatype: datatype.clone(),
                },
            };
            out.push(DeltaDatum {
                e: store.resolve(d.entity)?,
                a: store.resolve(d.attribute)?,
                v,
                valid_from: d.valid_from.clone(),
                valid_to: d.valid_to.clone(),
                op: match d.op {
                    Op::Assert => "assert",
                    Op::Retract => "retract",
                    Op::Tombstone => "tombstone",
                }
                .to_string(),
            });
        }
        Ok(Self {
            format: DELTA_FORMAT.to_string(),
            graph: graph_iri(store, graph),
            datums: out,
        })
    }

    /// Parse a delta presented as JSON. Whitespace and key order are free; the
    /// hash is taken over the re-serialized canonical form, never the bytes as
    /// presented.
    pub fn parse(json: &str) -> Result<Self> {
        let delta: Self = serde_json::from_str(json)
            .map_err(|e| Error::InvalidValue(format!("not a {DELTA_FORMAT} delta: {e}")))?;
        if delta.format != DELTA_FORMAT {
            return Err(Error::InvalidValue(format!(
                "delta format is '{}', expected '{DELTA_FORMAT}'",
                delta.format
            )));
        }
        Ok(delta)
    }

    /// The canonical JSON: fixed field order, no whitespace.
    #[must_use]
    pub fn canonical(&self) -> String {
        serde_json::to_string(self).expect("a delta always serializes")
    }

    /// `sha256:<hex>` over [`Self::canonical`].
    #[must_use]
    pub fn hash(&self) -> String {
        sha256(self.canonical().as_bytes())
    }

    /// Re-intern the delta as datums in `store`, with the graph's id.
    pub fn to_datums(&self, store: &Store) -> Result<(Vec<Datum>, i64)> {
        let graph = if self.graph == ROOT_GRAPH_IRI {
            ROOT_GRAPH
        } else {
            store.lookup(&self.graph)?.ok_or_else(|| {
                Error::InvalidValue(format!("graph <{}> is not in this store", self.graph))
            })?
        };
        let mut out = Vec::with_capacity(self.datums.len());
        for d in &self.datums {
            let value = match &d.v {
                DeltaValue::Ref(iri) => Value::Ref(store.intern(iri)?),
                DeltaValue::Str(s) => Value::Str(s.clone()),
                DeltaValue::Int(i) => Value::Int(*i),
                DeltaValue::Float(f) => Value::Float(
                    f.parse()
                        .map_err(|_| Error::InvalidValue(format!("bad float '{f}'")))?,
                ),
                DeltaValue::Bool(b) => Value::Bool(*b),
                DeltaValue::Bytes(h) => Value::Bytes(
                    hex::decode(h).map_err(|_| Error::InvalidValue(format!("bad hex '{h}'")))?,
                ),
                DeltaValue::Lang { lexical, lang } => Value::Lang {
                    lexical: lexical.clone(),
                    lang: lang.clone(),
                },
                DeltaValue::Typed { lexical, datatype } => Value::Typed {
                    lexical: lexical.clone(),
                    datatype: datatype.clone(),
                },
            };
            let op = match d.op.as_str() {
                "assert" => Op::Assert,
                "retract" => Op::Retract,
                "tombstone" => Op::Tombstone,
                other => return Err(Error::InvalidValue(format!("unknown op '{other}'"))),
            };
            out.push(Datum {
                entity: store.intern(&d.e)?,
                attribute: store.intern(&d.a)?,
                value,
                valid_from: d.valid_from.clone(),
                valid_to: d.valid_to.clone(),
                op,
            });
        }
        Ok((out, graph))
    }
}

/// What the gate captured about one refused write, inside its savepoint.
#[derive(Debug, Clone)]
pub struct Capture {
    /// The attempt, canonical.
    pub delta: AttemptedDelta,
    /// Its hash.
    pub attempt: String,
    /// The last committed transaction under the gate's pre-state.
    pub base_tx: i64,
    /// The write's timestamp.
    pub at: String,
    /// The write's declared actor.
    pub actor: Option<String>,
    /// The write's declared source.
    pub source: Option<String>,
    /// The principal chain in force.
    pub chain: Vec<String>,
    /// The gate's clock for this evaluation.
    pub gate_now: i64,
    /// [`PolicyRegistry::digest`] at the gate.
    pub rules_digest: String,
    /// [`post_state_digest`] at the gate.
    pub post_digest: String,
}

/// What the gate decided on a replay's throwaway copy, handed back instead of
/// written (`Store::flush_pending_verdicts`).
#[derive(Debug, Default)]
pub struct ReplayCapture {
    /// The verdicts the gate staged.
    pub verdicts: Vec<PendingVerdict>,
    /// The quarantine capture, when the replayed write was refused.
    pub quarantine: Option<Capture>,
}

/// Capture a refused write, called by the gate while the savepoint still holds
/// the post-state. `caller` is the writer's datums (no OWL inference).
pub(crate) fn capture(
    store: &Store,
    registry: &PolicyRegistry,
    caller: &[Datum],
    graph: i64,
    gate_now: i64,
) -> Result<Capture> {
    // The staged transaction row is the newest one: `transaction_auth::begin`
    // inserted it inside this savepoint. Everything below it was committed
    // before the attempt, which is exactly the pre-state.
    let (tx, at, actor, source): (i64, String, Option<String>, Option<String>) =
        store.conn.query_row(
            "SELECT id, timestamp, actor, source FROM transactions ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let delta = AttemptedDelta::from_datums(store, caller, graph)?;
    Ok(Capture {
        attempt: delta.hash(),
        delta,
        base_tx: tx - 1,
        at,
        actor,
        source,
        chain: store.principal_chain().to_vec(),
        gate_now,
        rules_digest: registry.digest(),
        post_digest: post_state_digest(store)?,
    })
}

/// `sha256:<hex>` over every live row of `facts` — the state a claim ASK reads.
///
/// Rows with `valid_to IS NULL`, in `(g, e, a, v, op, valid_from)` order,
/// which `idx_geav` serves without a sort. `tx` is deliberately NOT hashed: it
/// says when a fact arrived, not what the state is, and a replay reproduces
/// state rather than history.
///
/// Cost: one indexed scan of the live facts. It runs once per DENIAL, never per
/// write — an accepted write needs no quarantine.
pub fn post_state_digest(store: &Store) -> Result<String> {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut stmt = store.conn.prepare(
        "SELECT g, e, a, v, op, valid_from FROM facts WHERE valid_to IS NULL \
         ORDER BY g, e, a, v, op, valid_from",
    )?;
    let mut rows = stmt.query([])?;
    let mut n: u64 = 0;
    while let Some(r) = rows.next()? {
        let v: Vec<u8> = r.get(3)?;
        let from: String = r.get(5)?;
        for id in [r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?] {
            ctx.update(&id.to_le_bytes());
        }
        ctx.update(&(v.len() as u64).to_le_bytes());
        ctx.update(&v);
        ctx.update(&r.get::<_, i64>(4)?.to_le_bytes());
        ctx.update(&(from.len() as u64).to_le_bytes());
        ctx.update(from.as_bytes());
        n += 1;
    }
    ctx.update(&n.to_le_bytes());
    Ok(format!("sha256:{}", hex::encode(ctx.finish().as_ref())))
}

/// The canonical message an entry's seal signs. Every field a replay relies
/// on is in it; `purged_at` is not, because purging is the one change an
/// entry is allowed to undergo.
#[must_use]
pub fn seal_message(entry: &Entry) -> Vec<u8> {
    format!(
        "quarantine-v1|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        entry.verdict,
        entry.attempt,
        entry.graph,
        entry.base_tx,
        entry.at,
        entry.actor.as_deref().unwrap_or(""),
        entry.source.as_deref().unwrap_or(""),
        entry.chain.join(","),
        entry.gate_now,
        entry.rules_digest,
        entry.post_digest,
        entry.retention,
        entry.verifier,
    )
    .into_bytes()
}

/// Record `capture` once per verdict it backs. Called by the verdict flush,
/// after the verdicts landed, with the verdicts' subject ids.
pub(crate) fn record(store: &Store, capture: &Capture, verdicts: &[i64]) -> Result<()> {
    let Some(identity) = store.signing_identity() else {
        // Unreachable in practice — the verdicts this keys to needed the same
        // identity — and a quarantine entry is never written unsealed.
        return Ok(());
    };
    let full = store
        .governance_config()
        .quarantine
        .retains_full(&capture.delta.graph);
    for &subject in verdicts {
        let mut entry = Entry {
            id: 0,
            verdict: store.resolve(subject)?,
            attempt: capture.attempt.clone(),
            graph: capture.delta.graph.clone(),
            base_tx: capture.base_tx,
            at: capture.at.clone(),
            actor: capture.actor.clone(),
            source: capture.source.clone(),
            chain: capture.chain.clone(),
            gate_now: capture.gate_now,
            rules_digest: capture.rules_digest.clone(),
            post_digest: capture.post_digest.clone(),
            retention: if full { "full" } else { "digest" }.to_string(),
            verifier: identity.verifier.clone(),
            seal: String::new(),
            purged_at: None,
            sealed_delta: false,
        };
        entry.seal = identity.sign(&seal_message(&entry));
        store.conn.execute(
            "INSERT OR IGNORE INTO denial_quarantine (verdict, attempt, graph, base_tx, at, \
             actor, source, chain, gate_now, rules_digest, post_digest, retention, verifier, seal) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entry.verdict,
                entry.attempt,
                entry.graph,
                entry.base_tx,
                entry.at,
                entry.actor,
                entry.source,
                entry.chain.join(","),
                entry.gate_now,
                entry.rules_digest,
                entry.post_digest,
                entry.retention,
                entry.verifier,
                entry.seal,
            ],
        )?;
    }
    if full {
        store.conn.execute(
            "INSERT OR IGNORE INTO quarantine_deltas (attempt, delta) VALUES (?1, ?2)",
            params![capture.attempt, capture.delta.canonical()],
        )?;
    }
    Ok(())
}

/// One quarantine entry, as the audit reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Row id.
    pub id: i64,
    /// The verdict IRI this entry backs.
    pub verdict: String,
    /// Hash of the attempted delta.
    pub attempt: String,
    /// Graph IRI the write targeted.
    pub graph: String,
    /// Last committed transaction under the gate's pre-state.
    pub base_tx: i64,
    /// The write's timestamp.
    pub at: String,
    /// The write's declared actor.
    pub actor: Option<String>,
    /// The write's declared source.
    pub source: Option<String>,
    /// The principal chain in force.
    pub chain: Vec<String>,
    /// The gate's clock.
    pub gate_now: i64,
    /// Digest of the rule set in force.
    pub rules_digest: String,
    /// Digest of the post-state judged.
    pub post_digest: String,
    /// `digest` or `full`.
    pub retention: String,
    /// The verifier that sealed the entry.
    pub verifier: String,
    /// Hex ed25519 over [`seal_message`].
    pub seal: String,
    /// When the sealed delta was purged, if it was.
    pub purged_at: Option<String>,
    /// Whether a sealed delta is still held for this entry's attempt.
    pub sealed_delta: bool,
}

/// Quarantine entries, oldest first — every one, or those backing `verdict`.
pub fn entries(store: &Store, verdict: Option<&str>) -> Result<Vec<Entry>> {
    let mut stmt = store.conn.prepare(
        "SELECT q.id, q.verdict, q.attempt, q.graph, q.base_tx, q.at, q.actor, q.source, \
                q.chain, q.gate_now, q.rules_digest, q.post_digest, q.retention, q.verifier, \
                q.seal, q.purged_at, \
                EXISTS (SELECT 1 FROM quarantine_deltas d WHERE d.attempt = q.attempt) \
         FROM denial_quarantine q WHERE ?1 IS NULL OR q.verdict = ?1 ORDER BY q.id",
    )?;
    let rows = stmt.query_map(params![verdict], |r| {
        let chain: Option<String> = r.get(8)?;
        Ok(Entry {
            id: r.get(0)?,
            verdict: r.get(1)?,
            attempt: r.get(2)?,
            graph: r.get(3)?,
            base_tx: r.get(4)?,
            at: r.get(5)?,
            actor: r.get(6)?,
            source: r.get(7)?,
            chain: chain
                .filter(|c| !c.is_empty())
                .map(|c| c.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            gate_now: r.get(9)?,
            rules_digest: r.get(10)?,
            post_digest: r.get(11)?,
            retention: r.get(12)?,
            verifier: r.get(13)?,
            seal: r.get(14)?,
            purged_at: r.get(15)?,
            sealed_delta: r.get(16)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The sealed delta held for `attempt`, verbatim, if any.
pub fn sealed_delta(store: &Store, attempt: &str) -> Result<Option<String>> {
    Ok(store
        .conn
        .query_row(
            "SELECT delta FROM quarantine_deltas WHERE attempt = ?1",
            params![attempt],
            |r| r.get(0),
        )
        .optional()?)
}

/// Erase sealed deltas — by graph, by age, or both — and keep everything else.
///
/// The entries stay, digests and seals intact, stamped `purged_at = now`; only
/// the content in `quarantine_deltas` is DELETED. The verdicts are facts and
/// are not touched at all. So after a purge the audit can still say "refused,
/// content purged", and can still re-derive the refusal from a delta someone
/// presents, because the digest it is checked against is still there.
///
/// `before` compares lexically against the write's timestamp, which is
/// correct for the RFC 3339 UTC form the write paths use. Returns the number
/// of entries whose content was purged.
pub fn purge(store: &Store, graph: Option<&str>, before: Option<&str>, now: &str) -> Result<usize> {
    let stamped = store.conn.execute(
        "UPDATE denial_quarantine SET purged_at = ?1 \
         WHERE retention = 'full' AND purged_at IS NULL \
           AND (?2 IS NULL OR graph = ?2) AND (?3 IS NULL OR at < ?3)",
        params![now, graph, before],
    )?;
    // A delta shared by an unpurged entry (the same attempt refused into two
    // graphs is two attempts, but the same attempt backing two verdicts is one)
    // stays until every entry holding it is purged.
    store.conn.execute(
        "DELETE FROM quarantine_deltas WHERE attempt NOT IN \
         (SELECT attempt FROM denial_quarantine WHERE retention = 'full' AND purged_at IS NULL)",
        [],
    )?;
    Ok(stamped)
}

fn graph_iri(store: &Store, graph: i64) -> String {
    if graph == ROOT_GRAPH {
        ROOT_GRAPH_IRI.to_string()
    } else {
        store
            .resolve(graph)
            .unwrap_or_else(|_| format!("g:{graph}"))
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    format!("sha256:{}", hex::encode(digest.as_ref()))
}

#[cfg(test)]
#[path = "quarantine_tests.rs"]
mod tests;

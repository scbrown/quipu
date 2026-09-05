//! `align apply`: write a decided mapping set's knots into the alignment graph.
//!
//! Four properties, each of which is a requirement rather than a nicety, and
//! each tested (see `docs/design/cross-graph-alignment.md` and the slice-2
//! checklist on `aegis-sosiaa`):
//!
//! * **R1 — provenance on the TRIPLE, not just the row.** A row's provenance is
//!   only reachable by someone holding the mapping set; the graph is what a
//!   reader actually meets. Each derived assertion is accompanied by facts
//!   naming the author, the justification and the date, so an assertion can be
//!   attributed starting from the assertion.
//! * **R3 — idempotent, verified by COUNT.** Re-applying an unchanged set
//!   writes nothing, which the store gives us
//!   (`store::tests::duplicate_assert_into_a_named_graph_is_idempotent`). What
//!   this module has to get right is the `aegis-x1175` case: the same pair with
//!   CHANGED derived content, where the triples differ and nothing dedupes
//!   them.
//! * **R4 — one transaction per invocation.** Not because a partial apply is
//!   unrecoverable — idempotent re-apply converges — but because between a
//!   failure and the retry somebody can edit the set, and the graph would then
//!   hold knots from two generations with nothing recording which.
//! * **R5 — the set version is checked at commit.** R4 closes the sequential
//!   window; two concurrent invocations with an edit between them are two
//!   individually valid transactions, so atomicity cannot see it.

use crate::error::{Error, Result};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::sssom::{MappingSet, OWL_SAME_AS};
use super::verify::QUIPU_DISTINCT_FROM;

/// Provenance predicates written alongside each derived assertion (R1).
const ALIGN_NS: &str = "https://quipu.dev/ontology/align/";

/// What one `apply` did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApplyReport {
    /// The alignment graph written to.
    pub graph: String,
    /// The set version this run committed, as `sha256:…`.
    pub set_version: String,
    /// `owl:sameAs` assertions derived.
    pub same_as: usize,
    /// `quipu:distinctFrom` assertions derived.
    pub distinct_from: usize,
    /// Triples the store actually wrote. **Zero on an unchanged re-apply.**
    ///
    /// Measured as the change in live facts, NOT taken from
    /// `ingest_rdf_to_graph`'s second return value — that is the count of
    /// datums PARSED (`rdf.rs:213`, `let count = datums.len()`), which is the
    /// same on every re-apply whether or not anything was stored. Reading it as
    /// "written" reported 4 writes for a re-apply that stored nothing, in the
    /// function whose entire purpose is that a report cannot be trusted over a
    /// count. Caught by this module's own R3 test.
    pub written: usize,
}

impl ApplyReport {
    /// Did this run change the graph?
    ///
    /// Named for what it measures. A `false` here is the ordinary, expected
    /// outcome of re-applying an unchanged set — it is not a failure, and it is
    /// not a no-op that needs explaining.
    #[must_use]
    pub fn changed_the_graph(&self) -> bool {
        self.written > 0
    }
}

/// The stable identity of a mapping set's content.
///
/// Free, because the set already serialises deterministically to TSV — that is
/// acceptance criterion 2 — so no new machinery is needed to version it.
///
/// # Errors
/// The set cannot be serialised (a tab or newline in a field).
pub fn set_version(set: &MappingSet) -> Result<String> {
    Ok(crate::share::sha256(set.to_tsv()?.as_bytes()))
}

/// The alignment graph IRI for two source graphs.
///
/// Derived rather than operator-named so that criteria 5 and 6 depend on the
/// inputs rather than on somebody typing the same string twice.
#[must_use]
pub fn derived_graph_iri(graph_a: &str, graph_b: &str) -> String {
    let (first, second) = if graph_a <= graph_b {
        (graph_a, graph_b)
    } else {
        (graph_b, graph_a)
    };
    let a = crate::share::sha256(first.as_bytes());
    let b = crate::share::sha256(second.as_bytes());
    format!("urn:quipu:align:{}:{}", &a[7..19], &b[7..19])
}

/// Render the N-Triples a decided set derives, provenance included.
///
/// Pure, so the exact bytes written are testable without a store — and because
/// `apply` must be able to compare what it is about to write against what is
/// already there without a transaction open.
///
/// # Errors
/// A row carries an IRI that cannot be written as N-Triples.
pub fn derive_ntriples(set: &MappingSet, timestamp: &str) -> Result<String> {
    let mut out = String::new();
    let mut rows: Vec<_> = set
        .mappings
        .iter()
        .filter(|m| m.derives_knot() || m.derives_distinct_from())
        .collect();
    // Deterministic output for the same set, for the same reason `propose`
    // sorts: a re-render that reshuffles cannot be diffed or compared.
    rows.sort_by_key(|m| m.pair_key());

    for m in rows {
        let predicate = if m.derives_knot() {
            OWL_SAME_AS
        } else {
            QUIPU_DISTINCT_FROM
        };
        for iri in [&m.subject_id, &m.object_id] {
            if iri.contains('>') || iri.contains(' ') {
                return Err(Error::InvalidValue(format!(
                    "alignment cannot write an IRI containing '>' or a space: {iri:?}"
                )));
            }
        }
        out.push_str(&format!(
            "<{}> <{}> <{}> .\n",
            m.subject_id, predicate, m.object_id
        ));

        // R1: provenance ON the assertion. Attached to the SUBJECT of the pair
        // so it is reachable from either end of the assertion via the pair, and
        // written as ordinary facts so `share`/`import` carry them like any
        // other — a knot that arrives anonymous on a third store would fail the
        // attribution this exists for.
        let author = m.author_id.as_deref().unwrap_or("");
        out.push_str(&format!(
            "<{}> <{ALIGN_NS}assertedBy> \"{}\" .\n",
            m.subject_id,
            escape(author)
        ));
        out.push_str(&format!(
            "<{}> <{ALIGN_NS}assertedOn> \"{}\" .\n",
            m.subject_id,
            escape(timestamp)
        ));
        out.push_str(&format!(
            "<{}> <{ALIGN_NS}justification> \"{}\" .\n",
            m.subject_id,
            escape(m.mapping_justification.curie())
        ));
    }
    Ok(out)
}

/// Build the exact write for a decided set: retractions first, then assertions.
///
/// ## Why this is not `ingest_rdf_to_graph`
///
/// The KNOTS are append-safe — an identical one dedupes, and a different pair is
/// a genuinely different assertion. **The provenance is not.** `assertedBy`,
/// `assertedOn` and `justification` are single-valued per subject, so an edited
/// row re-applied through a plain ingest leaves the OLD value alongside the new
/// one: the `aegis-x1175` shape, where the same subject accumulates a second
/// value every time the content changes and nothing in the response says so.
///
/// So a changed provenance value is RETRACTED and re-asserted in the same
/// transaction, which is `/set` semantics rather than `/knot` semantics — and
/// it has to be built here rather than borrowed from `set_triple`, because that
/// primitive writes to ROOT and the whole point of an alignment graph is that
/// nothing lands outside it.
fn plan(store: &mut Store, set: &MappingSet, graph: i64, timestamp: &str) -> Result<Vec<Datum>> {
    let mut rows: Vec<_> = set
        .mappings
        .iter()
        .filter(|m| m.derives_knot() || m.derives_distinct_from())
        .collect();
    rows.sort_by_key(|m| m.pair_key());

    let mut datums = Vec::new();
    for m in rows {
        let subject = store.intern(&m.subject_id)?;
        let object = store.intern(&m.object_id)?;
        let predicate = store.intern(if m.derives_knot() {
            OWL_SAME_AS
        } else {
            QUIPU_DISTINCT_FROM
        })?;
        datums.push(Datum {
            entity: subject,
            attribute: predicate,
            value: Value::Ref(object),
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });

        // R1, with replace semantics.
        for (suffix, value) in [
            ("assertedBy", m.author_id.clone().unwrap_or_default()),
            ("assertedOn", timestamp.to_string()),
            ("justification", m.mapping_justification.curie().to_string()),
        ] {
            let attribute = store.intern(&format!("{ALIGN_NS}{suffix}"))?;
            for stale in current_values(store, subject, attribute, graph)? {
                if stale != value {
                    datums.push(Datum {
                        entity: subject,
                        attribute,
                        value: Value::Str(stale),
                        valid_from: timestamp.to_string(),
                        valid_to: None,
                        op: Op::Retract,
                    });
                }
            }
            datums.push(Datum {
                entity: subject,
                attribute,
                value: Value::Str(value),
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
    }
    Ok(datums)
}

/// Current string values of one `(entity, attribute)` in one graph.
fn current_values(store: &Store, entity: i64, attribute: i64, graph: i64) -> Result<Vec<String>> {
    let mut stmt = store.conn.prepare(
        "SELECT v FROM facts WHERE e = ?1 AND a = ?2 AND g = ?3 AND op = 1 AND valid_to IS NULL",
    )?;
    let rows = stmt.query_map([entity, attribute, graph], |row| row.get::<_, Vec<u8>>(0))?;
    let mut out = Vec::new();
    for row in rows {
        if let Ok(Value::Str(s)) = Value::from_bytes(&row?) {
            out.push(s);
        }
    }
    Ok(out)
}

/// Live (un-retracted) asserted facts in one graph.
///
/// The measurement `written` is derived from, because the write path reports
/// what it parsed rather than what it stored.
fn live_facts(store: &Store, graph: i64) -> Result<usize> {
    let n: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE g = ?1 AND op = 1 AND valid_to IS NULL",
        [graph],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Write a decided set's knots into `graph`, in ONE transaction.
///
/// `expected_version` is the set version read before any work began (R5): if
/// the set has changed since, the commit is refused rather than writing knots
/// from a generation the operator is no longer looking at.
///
/// # Errors
/// The set changed under the run, the graph is not a registered committed
/// graph, or the write is refused.
pub fn apply(
    store: &mut Store,
    set: &MappingSet,
    graph_iri: &str,
    expected_version: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<ApplyReport> {
    // R5, checked BEFORE the write rather than after: a refusal that happens
    // after the triples are staged still has to unwind them, and the point is
    // that nothing was written.
    let version = set_version(set)?;
    if version != expected_version {
        return Err(Error::InvalidValue(format!(
            "the mapping set changed under this apply: read {expected_version}, now {version}. \
             Nothing was written. Re-read the set and re-run; another apply may be in flight."
        )));
    }

    let graph = store.lookup(graph_iri)?.ok_or_else(|| {
        Error::InvalidValue(format!(
            "alignment graph {graph_iri} is not interned; create it with graph_create first"
        ))
    })?;

    let before = live_facts(store, graph)?;
    let datums = plan(store, set, graph, timestamp)?;
    if !datums.is_empty() {
        store.transact_to_graph(
            &datums,
            timestamp,
            actor,
            Some(&format!("align:apply:{version}")),
            graph,
        )?;
    }
    let written = live_facts(store, graph)?.saturating_sub(before);

    Ok(ApplyReport {
        graph: graph_iri.to_string(),
        set_version: version,
        same_as: set.mappings.iter().filter(|m| m.derives_knot()).count(),
        distinct_from: set
            .mappings
            .iter()
            .filter(|m| m.derives_distinct_from())
            .count(),
        written,
    })
}

//! Streaming RDF ingestion with document-scoped blank nodes.
use super::{intern_subject, term_to_value};
use crate::store::Datum;
use crate::types::Op;
use crate::{Error, Result, Store, Value};
use oxrdf::Triple;
use oxrdfio::{RdfFormat, RdfParser};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, io::Read, rc::Rc};
/// What a chunked ingest actually did.
///
/// `parsed` is triples SEEN BY THE PARSER, and it is named that way on purpose.
/// `ingest_rdf_to_graph` returns `datums.len()` and quipu #127 established, the
/// expensive way, that this is not the number written -- it reported 4 writes for
/// a re-apply that stored nothing. A benchmark publishing ingest throughput from a
/// parse count would report the cheap half of the work and call it the whole, and
/// it would be wrong in the flattering direction (aegis-j0yaxj.2).
///
/// So: to measure THROUGHPUT, take a before/after count of live facts from the
/// store. This struct reports what the ingest did, not what the store now holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    /// Triples the parser produced. NOT a write count -- see above.
    pub parsed: usize,
    /// One per committed chunk, in order.
    pub tx_ids: Vec<i64>,
}

/// Stream RDF into `graph`, committing every `chunk` datums.
///
/// WHY THIS EXISTS. `ingest_rdf_to_graph` parses the WHOLE input into a `Vec<Datum>`
/// and commits it in ONE transaction. That is fine at 1M and impossible at 1B --
/// by construction, not by slowness: it needs the entire dataset resident and a
/// single transaction of the same size. The parser was already streaming
/// (`RdfParser::for_reader` yields one quad at a time); only the accumulation was
/// not.
///
/// MEMORY BOUND: one chunk of `Datum` plus the parser's own state.
///
/// ⚠ `timestamp` IS AN INPUT AND MUST NOT BE `now()` (malcolm, aegis-j0yaxj.2).
/// Two properties depend on it, and the second is the one that blocks acceptance:
///
///   * every chunk carries the SAME timestamp, because one ingest is one logical
///     event -- per-chunk stamps would make a 1B load appear in the store as data
///     that trickled in over hours, and every temporal read would believe it;
///   * two runs over one pinned dataset must produce the SAME store, or a
///     "re-derivable result bundle" is unreachable. A single `now()` resolved once
///     per run still fails that.
///
/// ⚠ A FAILED INGEST LEAVES A PARTIAL GRAPH, AND IT FAILS IN THE FLATTERING
/// DIRECTION. N transactions are not atomic the way one is: if chunk 57 of 100
/// fails, 56 chunks are committed and the graph reads as a smaller, complete
/// dataset. For a benchmark that is worse than an error -- 700M has better latency
/// than 1B, so truncation makes the numbers look BETTER, and a good-looking result
/// is published rather than investigated. The caller MUST declare the expected
/// count up front and refuse an unmet declaration; this function reports what it
/// committed and cannot make that judgement for you.
///
/// # Errors
///
/// Propagates parse and store errors. On error, chunks already committed REMAIN
/// committed -- that is the point of the warning above.
#[allow(clippy::too_many_arguments)] // mirrors ingest_rdf_to_graph, plus the chunk size
pub fn ingest_rdf_chunked(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
    chunk: usize,
) -> Result<IngestReport> {
    ingest_rdf_chunked_with_scope(
        store, reader, format, base_iri, timestamp, actor, source, graph, chunk, None,
    )
}

/// Ingest with an explicit blank-node document scope. Same input and scope share nodes across graphs.
#[allow(clippy::too_many_arguments)]
pub fn ingest_rdf_chunked_with_scope(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
    chunk: usize,
    blank_node_scope: Option<&str>,
) -> Result<IngestReport> {
    let chunk = chunk.max(1);
    let (reader, mut scope) = crate::rdf_scope::prepare(store, reader, graph, blank_node_scope)?;
    let mut parser = RdfParser::from_format(format);
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .map_err(|e| Error::InvalidValue(format!("bad base IRI: {e}")))?;
    }

    let mut report = IngestReport {
        parsed: 0,
        tx_ids: Vec::new(),
    };
    let mut batch: Vec<Datum> = Vec::with_capacity(chunk);

    for result in parser.for_reader(reader) {
        let quad = result.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
        let triple = scope.triple(Triple::from(quad));

        let e = intern_subject(store, &triple.subject)?;
        let a = store.intern(triple.predicate.as_str())?;
        let v = term_to_value(store, &triple.object)?;

        batch.push(Datum {
            entity: e,
            attribute: a,
            value: v,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
        report.parsed += 1;

        if batch.len() >= chunk {
            let tx = store.transact_to_graph(&batch, timestamp, actor, source, graph)?;
            report.tx_ids.push(tx);
            batch.clear();
        }
    }

    // The tail. An empty input commits NOTHING and returns an empty tx list rather
    // than an empty transaction -- a zero-datum tx would appear in the log as an
    // ingest that happened, which is the same class of lie as a parse count
    // reported as a write count.
    if !batch.is_empty() {
        let tx = store.transact_to_graph(&batch, timestamp, actor, source, graph)?;
        report.tx_ids.push(tx);
    }

    Ok(report)
}

/// What the caller commits to BEFORE the load starts, and what the store is left
/// asserting afterwards.
///
/// The declaration is made from the dataset itself (`wc -l`, `sha256sum`) and is an
/// INPUT, so a truncated load cannot satisfy it by lowering the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadDeclaration {
    /// Triples the source is declared to contain.
    pub triples: usize,
    /// Lowercase hex SHA-256 of the source BYTES.
    pub sha256: String,
}

/// IRI namespace for the completion assertions a declared ingest writes.
pub const INGEST_NS: &str = "urn:quipu:ingest:";

/// Reader that hashes every byte handed on, so the digest is of the bytes the
/// parser actually consumed rather than of a file re-read afterwards.
/// The hasher is shared rather than owned: the parser takes the reader by value and
/// `Peekable` has no `into_inner`, so there is no way to get it back afterwards.
struct HashingReader<R: Read> {
    inner: R,
    hasher: Rc<RefCell<Sha256>>,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.borrow_mut().update(&buf[..n]);
        Ok(n)
    }
}

/// Streaming ingest that REFUSES an unmet declaration.
///
/// `ingest_rdf_chunked` cannot make this judgement -- it does not know what the
/// input was supposed to contain. This does: the caller declares count and hash up
/// front, and the load is accepted only if the parse met both.
///
/// TWO GUARANTEES, and the second is the one a benchmark reader depends on:
///
///   1. **Refusal.** A short parse or a different source returns `Err` and the
///      partial graph is LEFT IN PLACE, visibly incomplete. It is not rolled back:
///      a silently-vanished failed load and a load that never ran are the same
///      observation, and the operator needs to be able to tell them apart.
///   2. **A durable marker, in the SAME transaction as the last chunk.** The
///      completion assertions cannot land without the final chunk landing, and the
///      final chunk cannot land without them -- so there is no window in which the
///      store says complete while data is still arriving, and none in which a
///      finished load looks unfinished. A reader who did not run the load can ask
///      the graph whether it is whole.
///
/// The graph asserts, about its own IRI:
///
/// | predicate | value |
/// |---|---|
/// | `urn:quipu:ingest:declaredTriples` | the declared count |
/// | `urn:quipu:ingest:sourceSha256` | the declared (= measured) digest |
/// | `urn:quipu:ingest:complete` | `true` |
///
/// # Errors
///
/// - ROOT is refused: a declaration describes a load window, and ROOT is the store.
/// - Parse and store errors propagate, leaving committed chunks committed.
/// - `Error::InvalidValue` if the parsed count or the source digest does not match
///   the declaration. Chunks already committed REMAIN committed and the completion
///   marker is absent, which is exactly how an incomplete graph should read.
#[allow(clippy::too_many_arguments)] // ingest_rdf_chunked, plus the declaration
pub fn ingest_rdf_declared(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
    chunk: usize,
    declared: &LoadDeclaration,
) -> Result<IngestReport> {
    ingest_rdf_declared_with_scope(
        store, reader, format, base_iri, timestamp, actor, source, graph, chunk, declared, None,
    )
}

/// Ingest with an explicit blank-node document scope. Same input and scope share nodes across graphs.
#[allow(clippy::too_many_arguments)]
pub fn ingest_rdf_declared_with_scope(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
    chunk: usize,
    declared: &LoadDeclaration,
    blank_node_scope: Option<&str>,
) -> Result<IngestReport> {
    if graph == crate::schema::ROOT_GRAPH {
        return Err(Error::InvalidValue(
            "declared ingest refuses ROOT: a declaration describes one load window, \
             and ROOT is the whole store -- its triple count is not the dataset's"
                .into(),
        ));
    }
    let graph_iri = store.resolve(graph)?;
    // ROOT BY NAME IS STILL ROOT. The check above compares the numeric id, and a
    // caller who passes ROOT's IRI never reaches it: `graph_create` interns
    // "urn:quipu:graph:root" as a NEW named graph with a nonzero id, so the refusal
    // that exists twenty lines up is walked straight past. malcolm measured this on
    // 2026-09-05 -- 641,803 facts landed in a named graph while a root query
    // returned 0, caught only by an anti-vacuity assert; without it an empty root
    // reads as a successful load.
    //
    // The root IRI is the one string a caller is likeliest to pass MEANING root, so
    // the guard has to cover the spelling as well as the id. Documenting the hole
    // instead was the alternative, and a documented hole in a guard is how the
    // guard stops meaning anything.
    if graph_iri == crate::schema::ROOT_GRAPH_IRI {
        return Err(Error::InvalidValue(format!(
            "declared ingest refuses ROOT: a declaration describes one load window, \
             and ROOT is the whole store -- its triple count is not the dataset's. \
             (Passed as the IRI '{}', which registers a NAMED graph shadowing ROOT's \
             own name rather than writing to ROOT -- so the load would appear to \
             succeed while a ROOT-scoped query returned nothing.)",
            crate::schema::ROOT_GRAPH_IRI
        )));
    }
    let subject = store.intern(&graph_iri)?;
    let a_count = store.intern(&format!("{INGEST_NS}declaredTriples"))?;
    let a_sha = store.intern(&format!("{INGEST_NS}sourceSha256"))?;
    let a_done = store.intern(&format!("{INGEST_NS}complete"))?;

    let chunk = chunk.max(1);
    let (reader, mut scope) = crate::rdf_scope::prepare(store, reader, graph, blank_node_scope)?;
    let mut parser = RdfParser::from_format(format);
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .map_err(|e| Error::InvalidValue(format!("bad base IRI: {e}")))?;
    }

    let hasher = Rc::new(RefCell::new(Sha256::new()));
    let hashing = HashingReader {
        inner: reader,
        hasher: Rc::clone(&hasher),
    };
    let mut report = IngestReport {
        parsed: 0,
        tx_ids: Vec::new(),
    };
    let mut batch: Vec<Datum> = Vec::with_capacity(chunk + 3);

    // PEEKABLE, not a plain loop. A full batch is committed only when the parser
    // has more to give -- so the last batch is always still in hand when the input
    // ends, and the completion assertions can join it. Committing eagerly would put
    // them in a transaction of their own whenever the triple count happened to be
    // an exact multiple of the chunk size: a rare, input-dependent hole in
    // guarantee 2, which is the worst kind to test for.
    let mut quads = parser.for_reader(hashing).peekable();
    while let Some(result) = quads.next() {
        let quad = result.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
        let triple = scope.triple(Triple::from(quad));

        let e = intern_subject(store, &triple.subject)?;
        let a = store.intern(triple.predicate.as_str())?;
        let v = term_to_value(store, &triple.object)?;

        batch.push(Datum {
            entity: e,
            attribute: a,
            value: v,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
        report.parsed += 1;

        if batch.len() >= chunk && quads.peek().is_some() {
            let tx = store.transact_to_graph(&batch, timestamp, actor, source, graph)?;
            report.tx_ids.push(tx);
            batch.clear();
        }
    }

    // Drop the iterator (and with it the reader) before finalising, so the shared
    // hasher has exactly one owner left and every consumed byte is in it.
    drop(quads);
    let measured = format!("{:x}", hasher.borrow_mut().clone().finalize());

    // CHECK BEFORE THE COMPLETION ASSERTIONS, NEVER AFTER. On a mismatch the tail
    // batch is committed anyway -- the graph must be visibly there and visibly
    // unmarked, not silently absent -- and then the error is returned.
    let mismatch = if report.parsed != declared.triples {
        Some(format!(
            "declared {} triples, parsed {} -- the load is short and a short graph \
             benchmarks BETTER than a whole one, so this is refused rather than reported",
            declared.triples, report.parsed
        ))
    } else if measured != declared.sha256.to_ascii_lowercase() {
        Some(format!(
            "declared source sha256 {} but read {measured} -- the bytes loaded are \
             not the bytes pinned, so the result is not re-derivable",
            declared.sha256
        ))
    } else {
        None
    };

    if let Some(why) = mismatch {
        if !batch.is_empty() {
            let tx = store.transact_to_graph(&batch, timestamp, actor, source, graph)?;
            report.tx_ids.push(tx);
        }
        return Err(Error::InvalidValue(format!(
            "ingest declaration unmet for graph '{graph_iri}': {why}. The partial \
             graph is LEFT IN PLACE and carries no completion marker."
        )));
    }

    for (attribute, value) in [
        (
            a_count,
            Value::Int(i64::try_from(declared.triples).unwrap_or(i64::MAX)),
        ),
        (a_sha, Value::Str(measured)),
        (a_done, Value::Bool(true)),
    ] {
        batch.push(Datum {
            entity: subject,
            attribute,
            value,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    let tx = store.transact_to_graph(&batch, timestamp, actor, source, graph)?;
    report.tx_ids.push(tx);

    Ok(report)
}

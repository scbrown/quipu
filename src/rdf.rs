//! RDF data model layer — bridges oxrdf types with the EAVT fact log.
//!
//! Converts between standard RDF terms (IRIs, blank nodes, literals) and the
//! integer-encoded term dictionary + typed `Value` used by the fact log.
//! Supports parsing Turtle/N-Triples/JSON-LD into the store and serializing
//! facts back to standard RDF formats.

use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term as OxTerm, Triple};
use oxrdfio::{RdfFormat, RdfParser};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

// Prefix for blank node IRIs in the term dictionary.
pub(crate) const BLANK_PREFIX: &str = "_:";

/// Convert an oxrdf subject to a term dictionary id.
pub fn intern_subject(store: &Store, subject: &NamedOrBlankNode) -> Result<i64> {
    match subject {
        NamedOrBlankNode::NamedNode(n) => store.intern(n.as_str()),
        NamedOrBlankNode::BlankNode(b) => store.intern(&format!("{BLANK_PREFIX}{}", b.as_str())),
    }
}

/// Convert an oxrdf `Term` (object position) to a `Value`.
///
/// - Named nodes and blank nodes → `Value::Ref(term_id)`
/// - Literals → `Value::Str`, `Value::Int`, `Value::Float`, `Value::Bool`
///   depending on the XSD datatype.
pub fn term_to_value(store: &Store, term: &OxTerm) -> Result<Value> {
    match term {
        OxTerm::NamedNode(n) => {
            let id = store.intern(n.as_str())?;
            Ok(Value::Ref(id))
        }
        OxTerm::BlankNode(b) => {
            let id = store.intern(&format!("{BLANK_PREFIX}{}", b.as_str()))?;
            Ok(Value::Ref(id))
        }
        OxTerm::Literal(lit) => literal_to_value(lit),
        #[cfg(feature = "shacl")]
        OxTerm::Triple(_) => Err(Error::InvalidValue("unsupported RDF term type".into())),
    }
}

/// Map an RDF literal to a typed `Value` based on its XSD datatype.
fn literal_to_value(lit: &Literal) -> Result<Value> {
    // Language tags are checked first (their datatype is rdf:langString) and
    // are stored SEPARATELY from the lexical form. Concatenating them, as this
    // did, is irreversible corruption — aegis-fmyi.
    if let Some(lang) = lit.language() {
        return Ok(Value::Lang {
            lexical: lit.value().to_string(),
            lang: lang.to_string(),
        });
    }
    let dt = lit.datatype().as_str();
    match dt {
        namespace::XSD_INTEGER => {
            let n: i64 = lit
                .value()
                .parse()
                .map_err(|e| Error::InvalidValue(format!("bad integer literal: {e}")))?;
            Ok(Value::Int(n))
        }
        namespace::XSD_DOUBLE => {
            let value = lit
                .value()
                .parse::<f64>()
                .map_err(|e| Error::InvalidValue(format!("bad float literal: {e}")))?;
            Ok(Value::Typed {
                lexical: canonical_double(value),
                datatype: dt.to_string(),
            })
        }
        namespace::XSD_BOOLEAN => {
            let b = matches!(lit.value(), "true" | "1");
            Ok(Value::Bool(b))
        }
        namespace::XSD_STRING => Ok(Value::Str(lit.value().to_string())),
        _ => {
            // Numeric subtypes still have to parse — a malformed xsd:long is an
            // ingest error, not a string — but they keep their datatype IRI so
            // xsd:long/xsd:decimal/xsd:double stay distinguishable.
            if namespace::is_numeric_datatype(dt) {
                lit.value()
                    .parse::<f64>()
                    .map_err(|e| Error::InvalidValue(format!("bad numeric literal <{dt}>: {e}")))?;
            }
            Ok(Value::Typed {
                lexical: lit.value().to_string(),
                datatype: dt.to_string(),
            })
        }
    }
}

fn canonical_double(value: f64) -> String {
    let rendered = format!("{value:E}");
    let (mantissa, exponent) = rendered.split_once('E').unwrap_or((&rendered, "0"));
    let mantissa = if mantissa.contains('.') {
        mantissa.to_string()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{}", exponent.parse::<i32>().unwrap_or(0))
}

/// Convert a `Value` back to an oxrdf `Term` for serialization.
pub fn value_to_term(store: &Store, value: &Value) -> Result<OxTerm> {
    match value {
        Value::Ref(id) => {
            let iri = store.resolve(*id)?;
            if let Some(bnode_id) = iri.strip_prefix(BLANK_PREFIX) {
                Ok(OxTerm::BlankNode(BlankNode::new(bnode_id).map_err(
                    |e| Error::InvalidValue(format!("bad blank node: {e}")),
                )?))
            } else {
                Ok(OxTerm::NamedNode(NamedNode::new(&iri).map_err(|e| {
                    Error::InvalidValue(format!("bad IRI: {e}"))
                })?))
            }
        }
        // A plain Str is a plain literal. It is NOT sniffed for a trailing
        // "@xx" — the string "hello@en" is a legitimate string, and guessing
        // would manufacture a language tag nobody asserted (aegis-fmyi).
        Value::Str(s) => Ok(OxTerm::Literal(Literal::new_simple_literal(s))),
        Value::Lang { lexical, lang } => Ok(OxTerm::Literal(
            Literal::new_language_tagged_literal(lexical, lang)
                .map_err(|e| Error::InvalidValue(format!("bad language tag: {e}")))?,
        )),
        Value::Typed { lexical, datatype } => Ok(OxTerm::Literal(Literal::new_typed_literal(
            lexical,
            NamedNode::new(datatype)
                .map_err(|e| Error::InvalidValue(format!("bad datatype IRI: {e}")))?,
        ))),
        Value::Int(n) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            n.to_string(),
            NamedNode::new_unchecked(namespace::XSD_INTEGER),
        ))),
        Value::Float(f) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            f.to_string(),
            NamedNode::new_unchecked(namespace::XSD_DOUBLE),
        ))),
        Value::Bool(b) => Ok(OxTerm::Literal(Literal::new_typed_literal(
            b.to_string(),
            NamedNode::new_unchecked(namespace::XSD_BOOLEAN),
        ))),
        Value::Bytes(_) => Err(Error::InvalidValue(
            "cannot convert raw bytes to RDF term".into(),
        )),
    }
}

/// Resolve a fact's entity id back to an oxrdf subject.
fn id_to_subject(store: &Store, id: i64) -> Result<NamedOrBlankNode> {
    let iri = store.resolve(id)?;
    if let Some(bnode_id) = iri.strip_prefix(BLANK_PREFIX) {
        Ok(NamedOrBlankNode::BlankNode(
            BlankNode::new(bnode_id)
                .map_err(|e| Error::InvalidValue(format!("bad blank node: {e}")))?,
        ))
    } else {
        Ok(NamedOrBlankNode::NamedNode(NamedNode::new(&iri).map_err(
            |e| Error::InvalidValue(format!("bad IRI: {e}")),
        )?))
    }
}

/// Resolve a fact's attribute id back to an oxrdf `NamedNode`.
fn id_to_predicate(store: &Store, id: i64) -> Result<NamedNode> {
    let iri = store.resolve(id)?;
    NamedNode::new(&iri).map_err(|e| Error::InvalidValue(format!("bad predicate IRI: {e}")))
}

// ── Public API ──────────────────────────────────────────────────

/// Parse RDF from a reader and ingest all triples into the fact log.
///
/// Supported formats: Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD, `TriG`.
/// Returns the transaction id and the number of triples ingested.
pub fn ingest_rdf(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
) -> Result<(i64, usize)> {
    // Default target is ROOT (g=0). Named-graph writes use the _to_graph form.
    ingest_rdf_to_graph(store, reader, format, base_iri, timestamp, actor, source, 0)
}

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
    let chunk = chunk.max(1);
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
        let triple = Triple::from(quad);

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
    if graph == crate::schema::ROOT_GRAPH {
        return Err(Error::InvalidValue(
            "declared ingest refuses ROOT: a declaration describes one load window, \
             and ROOT is the whole store -- its triple count is not the dataset's"
                .into(),
        ));
    }
    let graph_iri = store.resolve(graph)?;
    let subject = store.intern(&graph_iri)?;
    let a_count = store.intern(&format!("{INGEST_NS}declaredTriples"))?;
    let a_sha = store.intern(&format!("{INGEST_NS}sourceSha256"))?;
    let a_done = store.intern(&format!("{INGEST_NS}complete"))?;

    let chunk = chunk.max(1);
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
        let triple = Triple::from(quad);

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

/// Ingest RDF into a specific named graph `g` (aegis-g1al / #36). g=0 is ROOT.
/// All facts from this parse land in `graph`, via `transact_to_graph`, so an
/// overlay ingest extends ROOT without mutating it.
#[allow(clippy::too_many_arguments)]
pub fn ingest_rdf_to_graph(
    store: &mut Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
    actor: Option<&str>,
    source: Option<&str>,
    graph: i64,
) -> Result<(i64, usize)> {
    let datums = parse_rdf(store, reader, format, base_iri, timestamp)?;
    let count = datums.len();
    let tx_id = store.transact_to_graph(&datums, timestamp, actor, source, graph)?;
    Ok((tx_id, count))
}

/// Parse RDF into fact-log datums without committing them.
///
/// This is used by callers that must combine RDF assertions with other changes
/// in one atomic transaction, such as replacing a producer-owned snapshot.
pub(crate) fn parse_rdf(
    store: &Store,
    reader: impl Read,
    format: RdfFormat,
    base_iri: Option<&str>,
    timestamp: &str,
) -> Result<Vec<Datum>> {
    let mut parser = RdfParser::from_format(format);
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .map_err(|e| Error::InvalidValue(format!("bad base IRI: {e}")))?;
    }

    let mut datums = Vec::new();
    for result in parser.for_reader(reader) {
        let quad = result.map_err(|e| Error::InvalidValue(format!("RDF parse error: {e}")))?;
        let triple = Triple::from(quad);

        let e = intern_subject(store, &triple.subject)?;
        let a = store.intern(triple.predicate.as_str())?;
        let v = term_to_value(store, &triple.object)?;

        datums.push(Datum {
            entity: e,
            attribute: a,
            value: v,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }

    Ok(datums)
}

/// Serialize current facts as RDF in the specified format.
///
/// Supported output formats: Turtle, N-Triples, N-Quads, RDF/XML, `TriG`.
/// Serialize a set of facts to an RDF document (shared by the exporters).
/// `pub(crate)` since quipu-gp5: fork promotion serializes its asserted delta
/// for the SHACL gate through the same path the exporters use.
pub(crate) fn serialize_facts(
    store: &Store,
    facts: &[crate::types::Fact],
    format: RdfFormat,
) -> Result<Vec<u8>> {
    let mut triples = Vec::with_capacity(facts.len());
    for fact in facts {
        let subject = id_to_subject(store, fact.entity)?;
        let predicate = id_to_predicate(store, fact.attribute)?;
        let object = value_to_term(store, &fact.value)?;

        triples.push(Triple {
            subject,
            predicate,
            object,
        });
    }

    crate::rdf_export::serialize_triples_canonical(&triples, format)
}

/// Export the ROOT/default graph's current facts, flattened to triples.
///
/// Drive-by (quipu #81): this said "EVERY current fact across all graphs".
/// `current_facts` is ROOT-only, so the docstring promised a whole-store export
/// that has never happened — name a graph with [`export_rdf_subset`] instead.
pub fn export_rdf(store: &Store, format: RdfFormat) -> Result<Vec<u8>> {
    let facts = store.current_facts()?;
    serialize_facts(store, &facts, format)
}

/// Export a SUBSET — the current facts of one named graph, or the ROOT/default
/// graph (quipu #36). This is the "pull a scoped slice" primitive that
/// subset-export and, above it, federation build on. `graph` is the graph's IRI;
/// `None` exports the ROOT default graph (`g = 0`). An unknown graph IRI is an
/// error (a targeted export of a graph that does not exist is a caller mistake,
/// not an empty success). The slice is a graph's OWN facts — the same scope a
/// `GRAPH <iri> { … }` read sees.
pub fn export_rdf_subset(
    store: &Store,
    format: RdfFormat,
    graph: Option<&str>,
) -> Result<(Vec<u8>, usize)> {
    let g = match graph {
        None => 0,
        Some(iri) => store
            .lookup(iri)?
            .ok_or_else(|| Error::InvalidValue(format!("unknown graph: {iri}")))?,
    };
    let facts = store.current_facts_in_graph(g)?;
    let count = facts.len();
    Ok((serialize_facts(store, &facts, format)?, count))
}

#[cfg(test)]
include!("rdf_tests.rs");

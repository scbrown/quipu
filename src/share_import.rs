//! Quarantined import and composition for git-native knowledge shares.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::{NamedNode, NamedOrBlankNode, Term, Triple};
use oxrdfio::{RdfFormat, RdfParser, RdfSerializer};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::share::{ShareManifest, manifest_bytes, sha256};
use crate::share_scrub::ShareDestination;
use crate::store::Store;

const SCHEMA_V1: &str = "https://github.com/scbrown/quipu/share-manifest/v1";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Wire request accepted by `POST /import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareImportRequest {
    pub manifest: ShareManifest,
    pub export_ntriples: String,
    pub shapes_turtle: String,
    pub source: String,
    #[serde(default)]
    pub actor: Option<String>,
    /// Apply exact canonical-name matches as IRI rewrites at import time, as
    /// releases before aegis-i48b9w did unconditionally.
    ///
    /// Defaults to FALSE, and the default is the point: an exact name match is a
    /// PROPOSAL. `#[serde(default)]` means every existing caller keeps
    /// deserialising and silently gets the safe behaviour, so the destructive
    /// path is one a caller now has to ask for by name.
    #[serde(default)]
    pub accept_exact: bool,
    /// External attestation over the v1 manifest identity (aegis-c9c44).
    ///
    /// NOT ON WASM. `session_attestation` is `cfg(not(wasm32))` — signature
    /// verification needs `ring`, which the browser build does not carry. A wasm
    /// consumer therefore has no attestation concept at all, which is honest:
    /// offering the field and never checking it would be worse than not having it.
    #[cfg(not(target_arch = "wasm32"))]
    ///
    /// OPTIONAL, and `#[serde(default)]` so every existing caller keeps
    /// deserialising unchanged. Absence is not silence: it is recorded as
    /// `attestation.tier = "transport"`, which says plainly that the bytes were
    /// checked and the AUTHOR was not.
    #[serde(default)]
    pub attestation: Option<crate::session_attestation::AttestationEnvelope>,
    /// The destination the OPERATOR declares for this import (aegis-auw0o7) —
    /// not the one the incoming manifest claims.
    ///
    /// `#[serde(skip)]`, so it is unreachable from the wire and every remote
    /// caller is treated as outward-bound. The field exists to let the local
    /// CLI say `--destination internal`; if a request could carry it, the gate
    /// below would be one an untrusted caller could switch off, which is the
    /// same hole the producer side refuses to open.
    #[serde(skip)]
    pub destination: crate::share_scrub::ShareDestination,
}

/// Count split between admitted and quarantined triples.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ImportCounts {
    pub accepted: usize,
    pub quarantined: usize,
}

/// One foreign-to-local resolution decision or review candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportMatch {
    pub foreign: String,
    pub local: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Why the resolver matched, e.g. `canonical_name:exact`. SSSOM's
    /// `mapping_justification` in all but name: it is what lets a caller
    /// bulk-accept the exact hits without re-deriving why they are exact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_on: Option<String>,
}

/// Entity-resolution report for an imported graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportResolution {
    pub exact_merges: Vec<ImportMatch>,
    pub candidates: Vec<ImportMatch>,
    pub unmatched: Vec<String>,
}

/// Local SHACL result and vocabulary findings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportValidation {
    pub conforms: bool,
    pub report: serde_json::Value,
    pub off_vocabulary: Vec<String>,
}

/// Whether the staged graph may be promoted to ROOT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionStatus {
    pub eligible: bool,
    pub blockers: Vec<String>,
}

/// Completed import decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareImportResult {
    pub outcome: String,
    pub import_id: String,
    pub share_id: String,
    pub graph_hash: String,
    pub staging_graph: String,
    pub triples: ImportCounts,
    pub resolution: ImportResolution,
    pub validation: ImportValidation,
    pub promotion: PromotionStatus,
    /// WHO produced this share, or an explicit statement that we do not know.
    /// Absent on wasm, where there is no verifier to answer with.
    #[cfg(not(target_arch = "wasm32"))]
    pub attestation: crate::share_attestation::AttestationStatus,
}

/// Request for the separate ROOT-promotion operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteImportRequest {
    pub share_id: String,
    #[serde(default)]
    pub actor: Option<String>,
}

/// Result of explicitly promoting a staged graph into ROOT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteImportResult {
    pub outcome: String,
    pub share_id: String,
    pub staging_graph: String,
    pub tx_id: i64,
    pub triples: usize,
}

fn hash_suffix(value: &str) -> Result<&str> {
    value
        .strip_prefix("sha256:")
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| Error::InvalidValue(format!("invalid sha256 identifier: {value}")))
}

fn staging_graph(share_id: &str, quarantine: bool) -> Result<String> {
    let hash = hash_suffix(share_id)?;
    let kind = if quarantine { "quarantine" } else { "staging" };
    Ok(format!("urn:quipu:import:{kind}:{hash}"))
}

fn import_id(share_id: &str) -> Result<String> {
    Ok(format!("urn:quipu:import:event:{}", hash_suffix(share_id)?))
}

fn provenance_source(prefix: &str, id: &str, claim: Option<&str>) -> String {
    claim.map_or_else(
        || format!("{prefix}:{id}"),
        |actor| format!("{prefix}:{id}:claimed-actor={actor}"),
    )
}

pub(crate) fn verify_share(request: &ShareImportRequest) -> Result<()> {
    if request.manifest.schema != SCHEMA_V1 {
        return Err(Error::InvalidValue(format!(
            "unsupported share manifest schema: {}",
            request.manifest.schema
        )));
    }
    if request.manifest.files.graph != "export.nt"
        || request.manifest.files.shapes != "shapes.ttl"
        || request
            .manifest
            .files
            .turtle_view
            .as_deref()
            .is_some_and(|p| p != "export.ttl")
    {
        return Err(Error::InvalidValue(
            "share manifest contains unsupported payload paths".into(),
        ));
    }
    let graph_hash = if request.manifest.canonicalization.as_deref() == Some("RDFC-1.0") {
        let canonical = crate::share::canonicalize_ntriples(request.export_ntriples.as_bytes())?;
        if canonical != request.export_ntriples.as_bytes() {
            return Err(Error::InvalidValue(
                "share graph is not RDFC-1.0 canonical N-Triples".into(),
            ));
        }
        sha256(&canonical)
    } else {
        sha256(request.export_ntriples.as_bytes())
    };
    if request.manifest.graph_hash != graph_hash {
        return Err(Error::InvalidValue(format!(
            "share graph hash mismatch: manifest={} actual={graph_hash}",
            request.manifest.graph_hash
        )));
    }
    let shapes_hash = sha256(request.shapes_turtle.as_bytes());
    if request.manifest.shapes_hash != shapes_hash {
        return Err(Error::InvalidValue(format!(
            "share shapes hash mismatch: manifest={} actual={shapes_hash}",
            request.manifest.shapes_hash
        )));
    }
    let expected = sha256(&manifest_bytes(&request.manifest, false)?);
    if request.manifest.share_id != expected {
        return Err(Error::InvalidValue(format!(
            "share id mismatch: manifest={} actual={expected}",
            request.manifest.share_id
        )));
    }
    Ok(())
}

fn parse_triples(input: &str) -> Result<Vec<Triple>> {
    RdfParser::from_format(RdfFormat::NTriples)
        .for_reader(input.as_bytes())
        .map(|quad| {
            quad.map(Triple::from)
                .map_err(|e| Error::InvalidValue(format!("share export.nt parse: {e}")))
        })
        .collect()
}

fn labels(triples: &[Triple]) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    for triple in triples {
        if triple.predicate.as_str() != RDFS_LABEL {
            continue;
        }
        let NamedOrBlankNode::NamedNode(subject) = &triple.subject else {
            continue;
        };
        let Term::Literal(label) = &triple.object else {
            continue;
        };
        labels
            .entry(subject.as_str().to_string())
            .or_insert_with(|| label.value().to_string());
    }
    labels
}

fn resolve_and_rewrite(
    store: &Store,
    triples: &mut [Triple],
    accept_exact: bool,
) -> Result<ImportResolution> {
    let mut report = ImportResolution::default();
    let mut replacements = BTreeMap::new();
    for (foreign, label) in labels(triples) {
        let result = crate::resolve_entity(store, &label, &[], 0.85, 5)?;
        if result.candidates.is_empty() {
            report.unmatched.push(foreign);
            continue;
        }
        let top = &result.candidates[0];
        if top.score == 1.0 && top.matched_on == "canonical_name:exact" {
            // aegis-i48b9w: an exact canonical-name match is a PROPOSAL, not a rewrite.
            //
            // The rewrite happened BEFORE the triples were stored, so the foreign
            // identity was gone by the time anything could record it: there was nothing
            // left to un-knot and no `owl:sameAs` was ever written. And because identity
            // here is the literal name string (aegis-6pd03), two graphs that each hold a
            // node named `config` or `main` were silently unified ACROSS A TRUST
            // BOUNDARY.
            //
            // The match is still reported, now carrying its score and justification so a
            // caller can bulk-accept the exact hits in one action.
            if accept_exact {
                replacements.insert(foreign.clone(), top.iri.clone());
            }
            report.exact_merges.push(ImportMatch {
                foreign,
                local: top.iri.clone(),
                score: Some(top.score),
                matched_on: Some(top.matched_on.clone()),
            });
        } else {
            for candidate in result.candidates {
                report.candidates.push(ImportMatch {
                    foreign: foreign.clone(),
                    local: candidate.iri,
                    score: Some(candidate.score),
                    matched_on: Some(candidate.matched_on),
                });
            }
        }
    }
    for triple in triples {
        if let NamedOrBlankNode::NamedNode(subject) = &triple.subject
            && let Some(local) = replacements.get(subject.as_str())
        {
            triple.subject = NamedOrBlankNode::NamedNode(
                NamedNode::new(local)
                    .map_err(|e| Error::InvalidValue(format!("resolved subject IRI: {e}")))?,
            );
        }
        if let Term::NamedNode(object) = &triple.object
            && let Some(local) = replacements.get(object.as_str())
        {
            triple.object = Term::NamedNode(
                NamedNode::new(local)
                    .map_err(|e| Error::InvalidValue(format!("resolved object IRI: {e}")))?,
            );
        }
    }
    Ok(report)
}

fn serialize(triples: &[Triple]) -> Result<String> {
    let mut lines = BTreeSet::new();
    for triple in triples {
        let mut writer = RdfSerializer::from_format(RdfFormat::NTriples).for_writer(Vec::new());
        writer
            .serialize_triple(triple)
            .map_err(|e| Error::InvalidValue(format!("resolved RDF serialize: {e}")))?;
        let bytes = writer
            .finish()
            .map_err(|e| Error::InvalidValue(format!("resolved RDF finish: {e}")))?;
        lines.insert(
            String::from_utf8(bytes)
                .map_err(|e| Error::InvalidValue(format!("resolved RDF UTF-8: {e}")))?,
        );
    }
    Ok(lines.into_iter().collect())
}

fn validate_local(store: &Store, data: &str) -> Result<ImportValidation> {
    let sanctioned = crate::vocabulary::sanctioned(store)?;
    let off_vocabulary = crate::vocabulary::ungoverned_types_in_turtle(data, &sanctioned);
    #[cfg(feature = "shacl")]
    let (conforms, report) = match store.get_combined_shapes()? {
        Some(shapes) => {
            let feedback = crate::shacl_context::validate_with_store_context(store, &shapes, data)?;
            (
                feedback.conforms,
                serde_json::to_value(feedback)
                    .map_err(|e| Error::Serialization(format!("SHACL report: {e}")))?,
            )
        }
        None => (
            true,
            serde_json::json!({"conforms": true, "reason": "no local shapes loaded"}),
        ),
    };
    #[cfg(not(feature = "shacl"))]
    let (conforms, report) = (
        true,
        serde_json::json!({"conforms": true, "reason": "SHACL feature not compiled"}),
    );
    Ok(ImportValidation {
        conforms,
        report,
        off_vocabulary,
    })
}

/// Verify, resolve, validate, and stage one share without touching ROOT.
pub fn import_share(
    store: &mut Store,
    request: &ShareImportRequest,
    timestamp: &str,
    authenticated_actor: Option<&str>,
) -> Result<ShareImportResult> {
    // MANIFEST IDENTITY FIRST, THEN AUTHORSHIP, and the order is the scope's
    // (aegis-c9c44). `verify_share` recomputes the payload hashes and refuses a
    // mismatch; attesting bytes that do not hash to what the manifest claims would
    // be signing the wrong thing.
    verify_share(request)?;
    // THEN THE DESTINATION MARKER THE SHARE CARRIES, still before anything is
    // staged (aegis-auw0o7). A share stamped `internal` was produced with the
    // outward scrub skipped, so importing it here is the moment that exemption
    // would be forgotten: the triples land in a store from which someone else
    // will later produce a share, and nothing downstream remembers where they
    // came from.
    //
    // The gate is NOT a blanket refusal, and the difference matters. It runs
    // the scrub the producer skipped and refuses only if the payload actually
    // fails it. A share marked internal out of caution, carrying nothing an
    // outward share could not carry, imports normally — the marker is checked
    // against the bytes rather than believed.
    //
    // The manifest is checked AFTER `verify_share`, never before: the marker is
    // inside the hash, so it means nothing until the bytes are known to hash to
    // the id they claim.
    if request
        .manifest
        .destination
        .is_some_and(ShareDestination::is_internal)
        && !request.destination.is_internal()
    {
        let files = BTreeMap::from([
            ("export.nt".to_string(), request.export_ntriples.clone()),
            ("shapes.ttl".to_string(), request.shapes_turtle.clone()),
        ]);
        crate::share_scrub::scrub_outward_payload(
            store,
            &files,
            "import of a share marked destination=internal",
        )?;
    }
    // THEN who produced it, BEFORE anything is staged. A tampered, replayed, or
    // unbound envelope must fail here rather than after the graph exists — the
    // whole point of a pre-staging check is that a refusal leaves nothing behind.
    #[cfg(not(target_arch = "wasm32"))]
    let attestation = crate::share_attestation::verify_attestation(store, request, timestamp)?;
    let mut triples = parse_triples(&request.export_ntriples)?;
    let resolution = resolve_and_rewrite(store, &mut triples, request.accept_exact)?;
    let resolved = serialize(&triples)?;
    let validation = validate_local(store, &resolved)?;
    let blockers = {
        let mut values = Vec::new();
        if !validation.conforms {
            values.push("shacl_nonconforming".to_string());
        }
        if !validation.off_vocabulary.is_empty() {
            values.push("off_vocabulary".to_string());
        }
        values
    };
    let quarantined = !blockers.is_empty();
    let graph_iri = staging_graph(&request.manifest.share_id, quarantined)?;
    let existing = store
        .lookup(&graph_iri)?
        .is_some_and(|g| store.graph_class(g).ok().flatten().as_deref() == Some("committed"));
    let count = triples.len();
    let outcome = if existing {
        "unchanged"
    } else {
        let graph = store.graph_create(&graph_iri)?;
        crate::rdf::ingest_rdf_to_graph(
            store,
            resolved.as_bytes(),
            RdfFormat::NTriples,
            None,
            timestamp,
            authenticated_actor,
            Some(&provenance_source(
                &format!("share-import:{}", request.source),
                &request.manifest.share_id,
                request.actor.as_deref(),
            )),
            graph,
        )?;
        if quarantined { "quarantined" } else { "staged" }
    };
    Ok(ShareImportResult {
        outcome: outcome.into(),
        import_id: import_id(&request.manifest.share_id)?,
        share_id: request.manifest.share_id.clone(),
        graph_hash: request.manifest.graph_hash.clone(),
        staging_graph: graph_iri,
        triples: if quarantined {
            ImportCounts {
                accepted: 0,
                quarantined: count,
            }
        } else {
            ImportCounts {
                accepted: count,
                quarantined: 0,
            }
        },
        resolution,
        validation,
        promotion: PromotionStatus {
            eligible: !quarantined,
            blockers,
        },
        #[cfg(not(target_arch = "wasm32"))]
        attestation,
    })
}

/// Explicitly copy an eligible staging graph into ROOT.
pub fn promote_import(
    store: &mut Store,
    request: &PromoteImportRequest,
    timestamp: &str,
    authenticated_actor: Option<&str>,
) -> Result<PromoteImportResult> {
    let graph_iri = staging_graph(&request.share_id, false)?;
    let graph = store.lookup(&graph_iri)?.ok_or_else(|| {
        Error::InvalidValue(format!(
            "no eligible staged import for {}",
            request.share_id
        ))
    })?;
    if store.graph_class(graph)?.as_deref() != Some("committed") {
        return Err(Error::InvalidValue(format!(
            "staging graph is not committed: {graph_iri}"
        )));
    }
    let (bytes, count) =
        crate::rdf::export_rdf_subset(store, RdfFormat::NTriples, Some(&graph_iri))?;
    let (tx_id, _) = crate::rdf::ingest_rdf(
        store,
        bytes.as_slice(),
        RdfFormat::NTriples,
        None,
        timestamp,
        authenticated_actor,
        Some(&provenance_source(
            "share-promotion",
            &request.share_id,
            request.actor.as_deref(),
        )),
    )?;
    Ok(PromoteImportResult {
        outcome: "promoted".into(),
        share_id: request.share_id.clone(),
        staging_graph: graph_iri,
        tx_id,
        triples: count,
    })
}

#[cfg(test)]
include!("share_import_tests.rs");

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "share_import_attestation_tests.rs"]
mod share_import_attestation_tests;

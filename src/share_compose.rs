//! Compose verified snapshots into an explicit, provenance-preserving dataset.
//!
//! This is an inspection plane, never an implicit promotion into ROOT. Different
//! shape bundles require an explicit authority. Validation sees the entire union,
//! because one pack may supply another pack's required context.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::{BlankNode, NamedOrBlankNode, Term, Triple};
use oxrdfio::{RdfFormat, RdfParser};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::share_import::ShareImportRequest;
use crate::store::Store;
use crate::store::datasets::DatasetMember;

/// One input's durable identity and explicitly scoped source graph.
#[derive(Debug, Serialize)]
pub struct ComposedPack {
    pub share_id: String,
    pub graph: String,
    pub store_id: String,
    pub tx_anchor: i64,
    pub created_at: String,
    pub source: String,
    pub manifest: crate::share::ShareManifest,
    pub attestation: crate::share_attestation::AttestationStatus,
}

/// Composition preserves a snapshot vector, not a fabricated common timestamp.
#[derive(Debug, Serialize)]
pub struct Composition {
    pub outcome: String,
    pub dataset: String,
    pub shapes_authority: String,
    pub packs: Vec<ComposedPack>,
    pub validation: serde_json::Value,
}

fn scoped_graph(request: &ShareImportRequest) -> Result<String> {
    let scope = &request.manifest.share_id[7..]; // verified before this function
    let mut lines = BTreeSet::new();
    for quad in
        RdfParser::from_format(RdfFormat::NTriples).for_reader(request.export_ntriples.as_bytes())
    {
        let mut triple = Triple::from(quad.map_err(|e| Error::InvalidValue(e.to_string()))?);
        let scoped = |node: &BlankNode| {
            BlankNode::new(format!("pack{scope}_{}", node.as_str()))
                .map_err(|e| Error::InvalidValue(e.to_string()))
        };
        if let NamedOrBlankNode::BlankNode(node) = &triple.subject {
            triple.subject = scoped(node)?.into();
        }
        if let Term::BlankNode(node) = &triple.object {
            triple.object = scoped(node)?.into();
        }
        lines.insert(format!("{triple} .\n"));
    }
    Ok(lines.into_iter().collect())
}

fn validate(shapes: &str, data: &str) -> Result<serde_json::Value> {
    #[cfg(feature = "shacl")]
    {
        let feedback = crate::shacl::validate_shapes(shapes, data)?;
        let mut report =
            serde_json::to_value(feedback).map_err(|e| Error::Serialization(e.to_string()))?;
        let authority = Store::open_in_memory()?;
        authority.load_shapes("composition-authority", shapes, "1970-01-01T00:00:00Z")?;
        let vocabulary = crate::vocabulary::sanctioned(&authority)?;
        let off_vocabulary = crate::vocabulary::ungoverned_types_in_turtle(data, &vocabulary);
        if !off_vocabulary.is_empty() {
            report["conforms"] = serde_json::Value::Bool(false);
        }
        report["off_vocabulary"] = serde_json::to_value(off_vocabulary).unwrap();
        let mut counts = BTreeMap::<String, usize>::new();
        if let Some(results) = report["results"].as_array() {
            for result in results {
                let key = result["source_shape"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                *counts.entry(key).or_default() += 1;
            }
        }
        report["violations_by_shape"] = serde_json::to_value(counts).unwrap();
        // Keep the full count but bound CLI diagnostics for large snapshots.
        if let Some(results) = report["results"].as_array_mut() {
            results.truncate(40);
        }
        Ok(report)
    }
    #[cfg(not(feature = "shacl"))]
    {
        let _ = (shapes, data);
        Err(Error::InvalidValue(
            "composition requires the shacl feature".into(),
        ))
    }
}

/// Verify all inputs, validate their union, and atomically stage a named dataset.
///
/// `authority` selects an input's complete shapes bundle. With no selection,
/// every bundle must be byte-identical. This never installs foreign shapes as
/// global write policy, rewrites labelled entities, or writes into ROOT.
/// Nonconforming snapshots remain queryable in this explicit inspection dataset.
/// Reloading the same snapshot set does not reassert locally retracted facts.
pub fn compose(
    store: &mut Store,
    requests: &[ShareImportRequest],
    authority: Option<usize>,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<Composition> {
    store.conn.execute_batch("SAVEPOINT quipu_compose_all")?;
    let result = compose_inner(store, requests, authority, timestamp, actor);
    match result {
        Ok(result) => {
            store.conn.execute_batch("RELEASE quipu_compose_all")?;
            Ok(result)
        }
        Err(error) => {
            store
                .conn
                .execute_batch("ROLLBACK TO quipu_compose_all; RELEASE quipu_compose_all")?;
            store.read_model.borrow_mut().clear();
            store.term_cache.borrow_mut().clear_persistent();
            Err(error)
        }
    }
}

fn compose_inner(
    store: &mut Store,
    requests: &[ShareImportRequest],
    authority: Option<usize>,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<Composition> {
    if requests.is_empty() {
        return Err(Error::InvalidValue(
            "composition needs at least one pack".into(),
        ));
    }
    let selected = authority.unwrap_or(0);
    let chosen = requests
        .get(selected)
        .ok_or_else(|| Error::InvalidValue("shape authority is not an input pack".into()))?;
    let mut attestations = Vec::new();
    for request in requests {
        crate::share_import::verify_share(request)?;
        if authority.is_none() && request.manifest.shapes_hash != chosen.manifest.shapes_hash {
            return Err(Error::InvalidValue(format!(
                "shape conflict: {} has {}, {} has {}; select --shapes-from explicitly",
                chosen.manifest.share_id,
                chosen.manifest.shapes_hash,
                request.manifest.share_id,
                request.manifest.shapes_hash,
            )));
        }
        if request
            .manifest
            .destination
            .is_some_and(crate::share::ShareDestination::is_internal)
            && !request.destination.is_internal()
        {
            crate::share_scrub::scrub_outward_payload(
                store,
                &BTreeMap::from([
                    ("export.nt".into(), request.export_ntriples.clone()),
                    ("shapes.ttl".into(), request.shapes_turtle.clone()),
                ]),
                "composition of an internal share",
            )?;
        }
        // Integrity is not authorship; retain the ordinary import verification.
        attestations.push(crate::share_attestation::verify_attestation(
            store, request, timestamp,
        )?);
    }
    let graphs = requests
        .iter()
        .map(|request| {
            let graph = format!(
                "urn:quipu:composition:pack:{}",
                &request.manifest.share_id[7..]
            );
            if let Some(id) = store.lookup(&graph)?
                && store.graph_class(id)?.as_deref() == Some("committed")
            {
                let (bytes, _) =
                    crate::rdf::export_rdf_subset(store, RdfFormat::NTriples, Some(&graph))?;
                return String::from_utf8(bytes).map_err(|e| Error::Serialization(e.to_string()));
            }
            scoped_graph(request)
        })
        .collect::<Result<Vec<_>>>()?;
    let union: BTreeSet<_> = graphs.iter().flat_map(|g| g.lines()).collect();
    let data = union
        .into_iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    let validation = validate(&chosen.shapes_turtle, &data)?;
    let mut identities: Vec<_> = requests
        .iter()
        .map(|r| r.manifest.share_id.as_str())
        .collect();
    identities.sort_unstable();
    identities.dedup();
    let identity = format!("{}\n{}", identities.join("\n"), chosen.manifest.shapes_hash);
    let digest = crate::share::sha256(identity.as_bytes());
    let dataset = format!("urn:quipu:composition:{}", &digest[7..]);
    let packs: Vec<_> = requests
        .iter()
        .zip(attestations)
        .map(|(r, attestation)| ComposedPack {
            share_id: r.manifest.share_id.clone(),
            graph: format!("urn:quipu:composition:pack:{}", &r.manifest.share_id[7..]),
            store_id: r.manifest.store_id.clone(),
            tx_anchor: r.manifest.tx_anchor,
            created_at: r.manifest.created_at.clone(),
            source: r.source.clone(),
            manifest: r.manifest.clone(),
            attestation,
        })
        .collect();
    let result = Composition {
        outcome: if validation["conforms"] == true {
            "composed"
        } else {
            "quarantined"
        }
        .into(),
        dataset,
        shapes_authority: chosen.manifest.share_id.clone(),
        packs,
        validation,
    };
    store.conn.execute_batch("SAVEPOINT quipu_composition")?;
    let write = (|| -> Result<()> {
        for (pack, data) in result.packs.iter().zip(graphs) {
            // A graph created here is committed with its complete payload in the
            // outer savepoint. Its existence is the durable replay marker; a local
            // retraction never authorizes refilling it from the old snapshot.
            if store.lookup(&pack.graph)?.is_some_and(|id| {
                store.graph_class(id).ok().flatten().as_deref() == Some("committed")
            }) {
                continue;
            }
            let graph = store.graph_create(&pack.graph)?;
            crate::rdf::ingest_rdf_to_graph(
                store,
                data.as_bytes(),
                RdfFormat::NTriples,
                None,
                timestamp,
                actor,
                Some(&format!("share-composition:{}", pack.share_id)),
                graph,
            )?;
        }
        let members: Vec<_> = result
            .packs
            .iter()
            .map(|p| DatasetMember::new(&p.graph))
            .collect();
        store.dataset_create(&result.dataset, &members, timestamp, actor)?;
        let meta = store.graph_create("urn:quipu:composition:metadata")?;
        let record =
            serde_json::to_string(&result).map_err(|e| Error::Serialization(e.to_string()))?;
        let payload = format!(
            "<{}> <urn:quipu:composition:manifest> {} .\n",
            result.dataset,
            oxrdf::Literal::new_simple_literal(record)
        );
        crate::rdf::ingest_rdf_to_graph(
            store,
            payload.as_bytes(),
            RdfFormat::NTriples,
            None,
            timestamp,
            actor,
            Some("share-composition:metadata"),
            meta,
        )?;
        Ok(())
    })();
    match write {
        Ok(()) => store.conn.execute_batch("RELEASE quipu_composition")?,
        Err(error) => {
            store
                .conn
                .execute_batch("ROLLBACK TO quipu_composition; RELEASE quipu_composition")?;
            store.read_model.borrow_mut().clear();
            store.term_cache.borrow_mut().clear_persistent();
            return Err(error);
        }
    }
    Ok(result)
}

#[cfg(all(test, feature = "shacl"))]
#[path = "share_compose_tests.rs"]
mod tests;

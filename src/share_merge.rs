//! Shape-aware three-way reconnect for git-native knowledge shares.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use oxrdf::{Term, Triple};
use oxrdfio::{RdfFormat, RdfParser};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::share::{ShareManifest, manifest_bytes, sha256};
use crate::store::Store;
use crate::types::Op;

const SH_PATH: &str = "http://www.w3.org/ns/shacl#path";
const SH_MAX_COUNT: &str = "http://www.w3.org/ns/shacl#maxCount";
type Graph = HashSet<Triple>;
type Slot = (String, String);

#[derive(Clone)]
struct LoadedShare {
    dir: PathBuf,
    manifest: ShareManifest,
    graph: Graph,
    shapes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionRecord {
    pub subject: String,
    pub predicate: String,
    pub max_count: usize,
    pub base: Vec<String>,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareStatus {
    pub incoming_share: String,
    pub base_share: String,
    pub base_path: String,
    pub local_graph_hash: String,
    pub incoming_graph_hash: String,
    pub ours_added: usize,
    pub ours_removed: usize,
    pub theirs_added: usize,
    pub theirs_removed: usize,
    pub conflicts: Vec<DecisionRecord>,
    pub diverged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeResult {
    pub outcome: String,
    pub tx_id: Option<i64>,
    pub asserted: usize,
    pub retracted: usize,
    pub provenance_parents: [String; 2],
    pub conflicts: Vec<DecisionRecord>,
}

fn parse_graph(input: &str, what: &str) -> Result<Graph> {
    RdfParser::from_format(RdfFormat::NTriples)
        .for_reader(input.as_bytes())
        .map(|q| {
            q.map(Triple::from)
                .map_err(|e| Error::InvalidValue(format!("{what} parse: {e}")))
        })
        .collect()
}

fn read_share(dir: &Path) -> Result<LoadedShare> {
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .map_err(|e| Error::Store(format!("share read {}/{name}: {e}", dir.display())))
    };
    let manifest: ShareManifest = serde_json::from_str(&read("manifest.json")?)
        .map_err(|e| Error::Serialization(format!("share manifest: {e}")))?;
    let export = read("export.nt")?;
    let shapes = read("shapes.ttl")?;
    let expected_share_id = sha256(&manifest_bytes(&manifest, false)?);
    if manifest.schema != "https://github.com/scbrown/quipu/share-manifest/v1"
        || manifest.files.graph != "export.nt"
        || manifest.files.shapes != "shapes.ttl"
        || manifest.share_id != expected_share_id
        || manifest.graph_hash != sha256(export.as_bytes())
        || manifest.shapes_hash != sha256(shapes.as_bytes())
    {
        return Err(Error::InvalidValue(format!(
            "share envelope or payload hash mismatch: {}",
            dir.display()
        )));
    }
    Ok(LoadedShare {
        dir: dir.into(),
        manifest,
        graph: parse_graph(&export, "share export.nt")?,
        shapes,
    })
}

fn scan(root: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth > 5 || !root.is_dir() {
        return Ok(());
    }
    if root.join("manifest.json").is_file() {
        out.push(root.into());
        return Ok(());
    }
    for entry in std::fs::read_dir(root)
        .map_err(|e| Error::Store(format!("lineage scan {}: {e}", root.display())))?
    {
        let path = entry
            .map_err(|e| Error::Store(format!("lineage entry: {e}")))?
            .path();
        if path.is_dir() && path.file_name().is_none_or(|n| n != ".git") {
            scan(&path, depth + 1, out)?;
        }
    }
    Ok(())
}

fn locate_base(incoming: &LoadedShare) -> Result<LoadedShare> {
    let parent = incoming.manifest.parent_share.as_deref().ok_or_else(|| {
        Error::InvalidValue(
            "incoming share has no parent_share; three-way merge has no base".into(),
        )
    })?;
    let root = incoming.dir.parent().unwrap_or(Path::new("."));
    let mut dirs = Vec::new();
    scan(root, 0, &mut dirs)?;
    let mut matches: Vec<_> = dirs
        .into_iter()
        .filter(|d| d != &incoming.dir)
        .filter_map(|d| read_share(&d).ok())
        .filter(|s| s.manifest.share_id == parent)
        .collect();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(Error::InvalidValue(format!(
            "base share {parent} not found under {}",
            root.display()
        ))),
        n => Err(Error::InvalidValue(format!(
            "base share {parent} is ambiguous: found {n} copies under {}",
            root.display()
        ))),
    }
}

fn root_graph(store: &Store) -> Result<(Graph, String)> {
    let (bytes, _) = crate::rdf::export_rdf_subset(store, RdfFormat::NTriples, None)?;
    let hash = sha256(&bytes);
    let text = String::from_utf8(bytes)
        .map_err(|e| Error::Serialization(format!("ROOT export is not UTF-8: {e}")))?;
    Ok((parse_graph(&text, "ROOT export")?, hash))
}

fn by_slot(graph: &Graph) -> BTreeMap<Slot, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for t in graph {
        out.entry((t.subject.to_string(), t.predicate.as_str().to_string()))
            .or_insert_with(BTreeSet::new)
            .insert(t.object.to_string());
    }
    out
}

fn max_counts(shapes: &str) -> Result<BTreeMap<String, usize>> {
    let triples: Vec<Triple> = RdfParser::from_format(RdfFormat::Turtle)
        .for_reader(shapes.as_bytes())
        .map(|q| {
            q.map(Triple::from)
                .map_err(|e| Error::InvalidValue(format!("shapes.ttl parse: {e}")))
        })
        .collect::<Result<_>>()?;
    let mut paths = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    for t in triples {
        let node = t.subject.to_string();
        if t.predicate.as_str() == SH_PATH
            && let Term::NamedNode(path) = &t.object
        {
            paths.insert(node.clone(), path.as_str().to_string());
        }
        if t.predicate.as_str() == SH_MAX_COUNT
            && let Term::Literal(v) = &t.object
            && let Ok(n) = v.value().parse()
        {
            bounds.insert(node, n);
        }
    }
    Ok(bounds
        .into_iter()
        .filter_map(|(node, n)| paths.get(&node).map(|p| (p.clone(), n)))
        .collect())
}

fn merge_graphs(
    base: &Graph,
    ours: &Graph,
    theirs: &Graph,
    shapes: &str,
) -> Result<(Graph, Vec<DecisionRecord>)> {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let bounds = max_counts(shapes)?;
    let slots: BTreeSet<_> = b.keys().chain(o.keys()).chain(t.keys()).cloned().collect();
    let empty = BTreeSet::new();
    let mut merged = Graph::new();
    let mut conflicts = Vec::new();
    for slot in slots {
        let bv = b.get(&slot).unwrap_or(&empty);
        let ov = o.get(&slot).unwrap_or(&empty);
        let tv = t.get(&slot).unwrap_or(&empty);
        let mut values = bv.clone();
        for removed in bv.difference(ov).chain(bv.difference(tv)) {
            values.remove(removed);
        }
        values.extend(ov.difference(bv).cloned());
        values.extend(tv.difference(bv).cloned());
        let bound = bounds.get(&slot.1);
        let delete_replace = bound == Some(&1)
            && !bv.is_empty()
            && ((ov.is_empty() && !tv.is_empty() && tv != bv)
                || (tv.is_empty() && !ov.is_empty() && ov != bv));
        let conflict = bound.is_some_and(|n| values.len() > *n) || delete_replace;
        let chosen = if conflict { bv } else { &values };
        if conflict {
            conflicts.push(DecisionRecord {
                subject: slot.0.clone(),
                predicate: slot.1.clone(),
                max_count: bound.copied().unwrap_or(1),
                base: bv.iter().cloned().collect(),
                ours: ov.iter().cloned().collect(),
                theirs: tv.iter().cloned().collect(),
            });
        }
        for value in chosen {
            let line = format!("{} <{}> {} .", slot.0, slot.1, value);
            merged.extend(parse_graph(&line, "merged triple")?);
        }
    }
    Ok((merged, conflicts))
}

fn parts(store: &Store, incoming_dir: &Path) -> Result<(ShareStatus, Graph, Graph, String)> {
    let incoming = read_share(incoming_dir)?;
    let base = locate_base(&incoming)?;
    let (ours, ours_hash) = root_graph(store)?;
    let (_, conflicts) = merge_graphs(&base.graph, &ours, &incoming.graph, &incoming.shapes)?;
    let status = ShareStatus {
        incoming_share: incoming.manifest.share_id.clone(),
        base_share: base.manifest.share_id,
        base_path: base.dir.display().to_string(),
        local_graph_hash: ours_hash,
        incoming_graph_hash: incoming.manifest.graph_hash,
        ours_added: ours.difference(&base.graph).count(),
        ours_removed: base.graph.difference(&ours).count(),
        theirs_added: incoming.graph.difference(&base.graph).count(),
        theirs_removed: base.graph.difference(&incoming.graph).count(),
        diverged: ours != base.graph && incoming.graph != base.graph,
        conflicts,
    };
    Ok((status, base.graph, incoming.graph, incoming.shapes))
}

pub fn status(store: &Store, incoming_dir: &Path) -> Result<ShareStatus> {
    parts(store, incoming_dir).map(|p| p.0)
}

pub fn merge(
    store: &mut Store,
    incoming_dir: &Path,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<MergeResult> {
    let (status, base, theirs, shapes) = parts(store, incoming_dir)?;
    let (ours, _) = root_graph(store)?;
    let (merged, conflicts) = merge_graphs(&base, &ours, &theirs, &shapes)?;
    let parents = [status.local_graph_hash, status.incoming_share];
    if !conflicts.is_empty() {
        return Ok(MergeResult {
            outcome: "conflicts".into(),
            tx_id: None,
            asserted: 0,
            retracted: 0,
            provenance_parents: parents,
            conflicts,
        });
    }
    let additions: Graph = merged.difference(&ours).cloned().collect();
    let removals: Graph = ours.difference(&merged).cloned().collect();
    let render = |g: &Graph| {
        let mut lines: Vec<_> = g.iter().map(|t| format!("{t} .\n")).collect();
        lines.sort();
        lines.concat()
    };
    let mut datums = crate::rdf::parse_rdf(
        store,
        render(&additions).as_bytes(),
        RdfFormat::NTriples,
        None,
        timestamp,
    )?;
    let mut retracts = crate::rdf::parse_rdf(
        store,
        render(&removals).as_bytes(),
        RdfFormat::NTriples,
        None,
        timestamp,
    )?;
    for d in &mut retracts {
        d.op = Op::Retract;
    }
    datums.extend(retracts);
    let source = format!("share-merge:parents={},{}", parents[0], parents[1]);
    let tx_id = store.transact(&datums, timestamp, actor, Some(&source))?;
    Ok(MergeResult {
        outcome: "merged".into(),
        tx_id: Some(tx_id),
        asserted: additions.len(),
        retracted: removals.len(),
        provenance_parents: parents,
        conflicts: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const P_ONE: &str = "https://example.org/one";
    const P_MANY: &str = "https://example.org/many";
    const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
[] sh:path <https://example.org/one> ; sh:maxCount 1 .
[] sh:path <https://example.org/many> .
"#;

    fn graph(lines: &[(&str, &str)]) -> Graph {
        let text = lines
            .iter()
            .map(|(p, o)| format!("<https://example.org/s> <{p}> {o} .\n"))
            .collect::<String>();
        parse_graph(&text, "test graph").unwrap()
    }

    fn values(graph: &Graph, predicate: &str) -> BTreeSet<String> {
        by_slot(graph)
            .get(&("<https://example.org/s>".into(), predicate.into()))
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn independent_multivalued_additions_union() {
        let base = graph(&[]);
        let ours = graph(&[(P_MANY, "\"ours\"")]);
        let theirs = graph(&[(P_MANY, "\"theirs\"")]);
        let (merged, conflicts) = merge_graphs(&base, &ours, &theirs, SHAPES).unwrap();
        assert!(conflicts.is_empty());
        assert_eq!(
            values(&merged, P_MANY),
            BTreeSet::from(["\"ours\"".into(), "\"theirs\"".into()])
        );
    }

    #[test]
    fn divergent_functional_values_emit_decision_and_hold_base() {
        let base = graph(&[(P_ONE, "\"base\"")]);
        let ours = graph(&[(P_ONE, "\"ours\"")]);
        let theirs = graph(&[(P_ONE, "\"theirs\"")]);
        let (merged, conflicts) = merge_graphs(&base, &ours, &theirs, SHAPES).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].max_count, 1);
        assert_eq!(values(&merged, P_ONE), BTreeSet::from(["\"base\"".into()]));
    }

    #[test]
    fn functional_delete_replace_is_a_conflict() {
        let base = graph(&[(P_ONE, "\"base\"")]);
        let ours = graph(&[]);
        let theirs = graph(&[(P_ONE, "\"replacement\"")]);
        let (_, conflicts) = merge_graphs(&base, &ours, &theirs, SHAPES).unwrap();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn missing_parent_refuses_before_merge() {
        let store = Store::open_in_memory().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("incoming");
        crate::share::share(
            &store,
            dir.to_str().unwrap(),
            &crate::share::ShareOptions {
                no_shapes: true,
                ..Default::default()
            },
        )
        .unwrap();
        let error = status(&store, &dir).unwrap_err().to_string();
        assert!(error.contains("no parent_share"), "{error}");
    }

    #[test]
    fn clean_reconnect_writes_one_tx_with_two_provenance_parents() {
        let root = tempfile::tempdir().unwrap();
        let mut source = Store::open_in_memory().unwrap();
        source.load_shapes("merge", SHAPES, "2026-08-29").unwrap();
        crate::rdf::ingest_rdf(
            &mut source,
            graph(&[(P_ONE, "\"base\"")])
                .iter()
                .map(|t| format!("{t} .\n"))
                .collect::<String>()
                .as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-08-29T00:00:00Z",
            None,
            Some("base"),
        )
        .unwrap();
        let base_dir = root.path().join("base");
        let base_manifest = crate::share::share(
            &source,
            base_dir.to_str().unwrap(),
            &crate::share::ShareOptions::default(),
        )
        .unwrap();
        crate::rdf::ingest_rdf(
            &mut source,
            format!("<https://example.org/s> <{P_MANY}> \"theirs\" .\n").as_bytes(),
            RdfFormat::NTriples,
            None,
            "2026-08-29T00:01:00Z",
            None,
            Some("theirs"),
        )
        .unwrap();
        let incoming_dir = root.path().join("incoming");
        let incoming_manifest = crate::share::share(
            &source,
            incoming_dir.to_str().unwrap(),
            &crate::share::ShareOptions {
                parent_share: Some(base_manifest.share_id),
                ..Default::default()
            },
        )
        .unwrap();

        let mut local = Store::open_in_memory().unwrap();
        local.load_shapes("merge", SHAPES, "2026-08-29").unwrap();
        crate::rdf::ingest_rdf(&mut local, format!("<https://example.org/s> <{P_ONE}> \"base\" .\n<https://example.org/s> <{P_MANY}> \"ours\" .\n").as_bytes(), RdfFormat::NTriples, None, "2026-08-29T00:02:00Z", None, Some("ours")).unwrap();
        let result = merge(
            &mut local,
            &incoming_dir,
            "2026-08-29T00:03:00Z",
            Some("reviewer"),
        )
        .unwrap();
        assert_eq!(result.outcome, "merged");
        assert_eq!(result.asserted, 1);
        assert_eq!(result.provenance_parents[1], incoming_manifest.share_id);
        let tx = local
            .get_transaction(result.tx_id.unwrap())
            .unwrap()
            .unwrap();
        let source = tx.source.unwrap();
        assert!(source.contains(&result.provenance_parents[0]));
        assert!(source.contains(&result.provenance_parents[1]));
        let (bytes, _) = crate::rdf::export_rdf_subset(&local, RdfFormat::NTriples, None).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"ours\""));
        assert!(text.contains("\"theirs\""));
    }
}

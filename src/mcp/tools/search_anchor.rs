//! Graph-anchored search (aegis-rcz5ib.8): root a search on an entity and rank
//! results by how many hops away from it they are.
//!
//! The anchor's neighbourhood is walked with BOUND single-pattern lookups only
//! (`e = ?` for outgoing edges, `v = ?` for incoming), never a property path or
//! an unbound pattern. Three things keep hop distance meaningful, each measured
//! on the served store before this was written ([rcz5ib8-bfs-findings]):
//!
//! - **Excluded predicates.** `rdf:type`, `rdfs:subClassOf`, PROV links,
//!   `distinctFrom` (it asserts the two are NOT the same thing), `mentions` and
//!   `inDocument` (document structure). Through them every node is two hops from
//!   everything, and one document's sections filled a whole result page.
//! - **Hubs are reached, not expanded.** A node with more than `hub_degree`
//!   edges (a person who owns hundreds of things, a repository that manages
//!   every service) gets a hop count but its edges are not followed. The anchor
//!   itself is always expanded.
//! - **`owl:sameAs` costs nothing**: aliases are one node (the aegis-6pd03 class).
//!
//! The walk is deterministic (neighbours in id order) and bounded by a node
//! budget. When the budget is hit, the response says so and names the hop it
//! stopped in: a truncated ring is never reported as complete.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{Value as JsonValue, json};

use crate::error::{Error, Result};
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

const AEGIS: &str = "http://aegis.gastown.local/ontology/";
const PROV: &str = "http://www.w3.org/ns/prov#";

/// Predicates never traversed unless `via` names them.
fn default_excluded() -> Vec<String> {
    let mut out = vec![
        namespace::RDF_TYPE.to_owned(),
        "http://www.w3.org/2000/01/rdf-schema#subClassOf".to_owned(),
        "http://quipu.dev/ns#distinctFrom".to_owned(),
    ];
    for p in [
        "wasGeneratedBy",
        "wasDerivedFrom",
        "used",
        "wasAssociatedWith",
        "wasAttributedTo",
    ] {
        out.push(format!("{PROV}{p}"));
    }
    for p in ["distinctFrom", "mentions", "inDocument"] {
        out.push(format!("{AEGIS}{p}"));
    }
    out
}

/// How hop distance shapes the ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Hops first; the text score orders results within a hop ring.
    Sort,
    /// `score × decay^hops`: a strong match two hops out can beat a weak one at one.
    Decay,
    /// Only results within `max_hops`, by text score.
    Filter,
}

/// The parsed anchor parameters of one request.
pub(super) struct AnchorRequest {
    pub anchor: String,
    pub max_hops: u32,
    pub mode: Mode,
    pub decay: f64,
    pub via: Option<Vec<String>>,
    pub direction: Direction,
    pub explain: bool,
    /// Nodes the walk may reach before it stops ([`NODE_BUDGET`]; tests lower it).
    pub budget: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Out,
    In,
    Both,
}

/// Server ceiling on `max_hops`.
const MAX_HOPS_CAP: u32 = 4;
/// Nodes the walk may visit before it stops and reports truncation.
const NODE_BUDGET: usize = 5000;
/// A node with more edges than this is reached but not expanded.
const HUB_DEGREE: usize = 150;

fn text_option<'a>(input: &'a JsonValue, key: &str) -> Result<Option<&'a str>> {
    input
        .get(key)
        .map(|v| {
            v.as_str()
                .ok_or_else(|| Error::InvalidValue(format!("{key} must be a string")))
        })
        .transpose()
}

impl AnchorRequest {
    /// `None` when the request names no anchor: the plain path is untouched.
    pub fn parse(input: &JsonValue) -> Result<Option<Self>> {
        let Some(value) = input.get("anchor") else {
            return Ok(None);
        };
        let anchor = value
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::InvalidValue("anchor must be a nonempty string".into()))?;
        let max_hops = match input.get("max_hops") {
            None => 3,
            Some(v) => u32::try_from(v.as_u64().ok_or_else(|| {
                Error::InvalidValue("max_hops must be a nonnegative integer".into())
            })?)
            .unwrap_or(u32::MAX),
        }
        .min(MAX_HOPS_CAP);
        let mode = match text_option(input, "anchor_mode")? {
            None | Some("decay") => Mode::Decay,
            Some("sort") => Mode::Sort,
            Some("filter") => Mode::Filter,
            Some(other) => {
                return Err(Error::InvalidValue(format!(
                    "anchor_mode must be sort, decay or filter, not {other:?}"
                )));
            }
        };
        let decay = match input.get("decay") {
            None => 0.5,
            Some(v) => v
                .as_f64()
                .ok_or_else(|| Error::InvalidValue("decay must be a number".into()))?,
        };
        if !(decay > 0.0 && decay <= 1.0) {
            return Err(Error::InvalidValue("decay must be in (0, 1]".into()));
        }
        let via = match input.get("via") {
            None => None,
            Some(v) => Some(
                v.as_array()
                    .ok_or_else(|| Error::InvalidValue("via must be a string array".into()))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                Error::InvalidValue("via entries must be nonempty strings".into())
                            })
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        };
        let direction = match text_option(input, "direction")? {
            None | Some("both") => Direction::Both,
            Some("out") => Direction::Out,
            Some("in") => Direction::In,
            Some(other) => {
                return Err(Error::InvalidValue(format!(
                    "direction must be out, in or both, not {other:?}"
                )));
            }
        };
        Ok(Some(Self {
            anchor: anchor.to_owned(),
            max_hops,
            mode,
            decay,
            via,
            direction,
            explain: input
                .get("explain")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            budget: NODE_BUDGET,
        }))
    }
}

/// Resolve the anchor to EXACTLY one entity: an IRI, a CURIE, or an exact
/// `rdfs:label`. Ambiguity and absence refuse and name what was found.
fn resolve_anchor(store: &Store, anchor: &str) -> Result<i64> {
    let prefixes = crate::compact::PrefixMap::from_store(store)?;
    let expanded = prefixes.expand(anchor);
    for candidate in [anchor, expanded.as_str()] {
        if let Some(id) = store.lookup(candidate)? {
            return Ok(id);
        }
    }
    let label = store.lookup(namespace::RDFS_LABEL)?;
    let mut matches = Vec::new();
    if let Some(label) = label {
        let bytes = Value::Str(anchor.to_owned()).to_bytes();
        let mut stmt = store.prepare(
            "SELECT DISTINCT e FROM facts WHERE a = ?1 AND v = ?2 AND op = 1 \
             AND valid_to IS NULL ORDER BY e LIMIT 6",
        )?;
        let rows = stmt.query_map(rusqlite::params![label, bytes], |r| r.get::<_, i64>(0))?;
        for row in rows {
            matches.push(row?);
        }
    }
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(Error::InvalidValue(format!(
            "anchor {anchor:?} is not an IRI, CURIE or exact label of any entity"
        ))),
        many => {
            let names: Vec<String> = many
                .iter()
                .map(|id| store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}")))
                .collect();
            Err(Error::InvalidValue(format!(
                "anchor {anchor:?} is ambiguous; it labels {}: {}. Pass one IRI",
                if many.len() > 5 {
                    "6 or more entities".to_owned()
                } else {
                    format!("{} entities", many.len())
                },
                names.join(", ")
            )))
        }
    }
}

/// The result of walking the anchor's neighbourhood.
pub(super) struct Neighbourhood {
    pub anchor_iri: String,
    /// Hop count per reached entity id (the anchor is 0).
    pub hops: HashMap<i64, u32>,
    /// `(parent, predicate id, outgoing?)` per reached node, for `explain`.
    parent: HashMap<i64, (i64, i64, bool)>,
    pub truncated_at: Option<u32>,
    pub hubs_not_expanded: usize,
}

impl Neighbourhood {
    /// One shortest path from the anchor to `id`, as `a -p-> b <-q- c`.
    pub fn path(&self, store: &Store, id: i64) -> Option<String> {
        if !self.hops.contains_key(&id) {
            return None;
        }
        let name = |n: i64| store.resolve(n).unwrap_or_else(|_| format!("ref:{n}"));
        let mut steps = Vec::new();
        let mut cur = id;
        while let Some((par, pred, out)) = self.parent.get(&cur) {
            let p = name(*pred);
            steps.push(if *out {
                format!("-[{p}]-> {}", name(cur))
            } else {
                format!("<-[{p}]- {}", name(cur))
            });
            cur = *par;
        }
        steps.reverse();
        Some(format!("{} {}", self.anchor_iri, steps.join(" ")))
    }
}

/// Edges of `node` as `(neighbour, predicate, outgoing?)`, current facts only
/// (or valid at `valid_at`), in id order. ROOT graph, like plain search.
fn edges(
    store: &Store,
    node: i64,
    direction: Direction,
    valid_at: Option<&str>,
    cap: usize,
    aliases_only: Option<&HashSet<i64>>,
) -> Result<Vec<(i64, i64, bool)>> {
    if aliases_only.is_some_and(HashSet::is_empty) {
        return Ok(Vec::new());
    }
    let alias_filter = aliases_only.map_or(String::new(), |ids| {
        let mut ids: Vec<_> = ids.iter().copied().collect();
        ids.sort_unstable();
        format!(
            " AND a IN ({})",
            ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
        )
    });
    let live = if valid_at.is_some() {
        "valid_from <= ?2 AND (valid_to IS NULL OR valid_to > ?2)"
    } else {
        "valid_to IS NULL AND ?2 IS NULL"
    };
    let mut out = Vec::new();
    if direction != Direction::In {
        let mut stmt = store.prepare(&format!(
            "SELECT a, v FROM facts WHERE e = ?1 AND g = 0 AND op = 1 AND {live} AND substr(v,1,1) = x'00'{alias_filter} ORDER BY a, v LIMIT {cap}"
        ))?;
        let rows = stmt.query_map(rusqlite::params![node, valid_at], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (a, v) = row?;
            if let Ok(Value::Ref(id)) = Value::from_bytes(&v) {
                out.push((id, a, true));
            }
        }
    }
    if direction != Direction::Out {
        let mut stmt = store.prepare(&format!(
            "SELECT e, a FROM facts WHERE v = ?1 AND g = 0 AND op = 1 AND {}{alias_filter} ORDER BY e, a LIMIT {cap}",
            live.replace("?2", "?3")
        ))?;
        let rows = stmt.query_map(
            rusqlite::params![Value::Ref(node).to_bytes(), Option::<i64>::None, valid_at],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        for row in rows {
            let (e, a) = row?;
            out.push((e, a, false));
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn ids(store: &Store, iris: &[String]) -> Result<HashSet<i64>> {
    let mut out = HashSet::new();
    let prefixes = crate::compact::PrefixMap::from_store(store)?;
    for iri in iris {
        out.extend(store.lookup_all(&prefixes.expand(iri))?);
    }
    Ok(out)
}

/// Walk the anchor's neighbourhood.
///
/// # Errors
/// An unresolvable or ambiguous anchor, or a store error.
pub(super) fn walk(
    store: &Store,
    req: &AnchorRequest,
    valid_at: Option<&str>,
) -> Result<Neighbourhood> {
    let anchor = resolve_anchor(store, &req.anchor)?;
    let anchor_iri = store.resolve(anchor)?;
    let same_as = ids(store, &[namespace::OWL_SAME_AS.to_owned()])?;
    let (allowed, excluded) = match &req.via {
        Some(via) => (Some(ids(store, via)?), HashSet::new()),
        None => (None, ids(store, &default_excluded())?),
    };
    let mut hops = HashMap::from([(anchor, 0u32)]);
    let mut parent = HashMap::new();
    // 0-1 BFS: a sameAs edge costs 0 (front), any other edge 1 (back).
    let mut queue = VecDeque::from([anchor]);
    let mut truncated_at = None;
    let mut hubs_not_expanded = 0;
    while let Some(node) = queue.pop_front() {
        let here = hops[&node];
        // At the limit, query ONLY zero-cost aliases instead of materializing
        // the whole outer ring merely to discard every ordinary edge.
        let aliases_only = (here >= req.max_hops).then_some(&same_as);
        let cap = if node == anchor {
            req.budget.saturating_add(1)
        } else {
            HUB_DEGREE + 1
        };
        let mut all = edges(store, node, req.direction, valid_at, cap, aliases_only)?;
        if node == anchor && all.len() >= cap {
            truncated_at = Some(here.saturating_add(1).min(req.max_hops));
        }
        if node != anchor && all.len() > HUB_DEGREE {
            hubs_not_expanded += 1;
            // A hub suppresses ordinary expansion, never alias collapse.
            all = edges(
                store,
                node,
                req.direction,
                valid_at,
                HUB_DEGREE + 1,
                Some(&same_as),
            )?;
            if all.len() > HUB_DEGREE {
                truncated_at = Some(truncated_at.map_or(here, |t: u32| t.min(here)));
            }
        }
        for (next, pred, out) in all {
            let zero = same_as.contains(&pred);
            if !zero
                && (excluded.contains(&pred)
                    || allowed.as_ref().is_some_and(|a| !a.contains(&pred)))
            {
                continue;
            }
            let cost = here + u32::from(!zero);
            if cost > req.max_hops || hops.get(&next).is_some_and(|h| *h <= cost) {
                continue;
            }
            if !hops.contains_key(&next) && hops.len() >= req.budget {
                truncated_at = Some(truncated_at.map_or(cost, |t: u32| t.min(cost)));
                continue;
            }
            hops.insert(next, cost);
            parent.insert(next, (node, pred, out));
            if zero {
                queue.push_front(next);
            } else {
                queue.push_back(next);
            }
        }
    }
    Ok(Neighbourhood {
        anchor_iri,
        hops,
        parent,
        truncated_at,
        hubs_not_expanded,
    })
}

/// Reorder `(entity id, score)` candidates by the anchor, returning
/// `(id, final score, hops)`.
pub(super) fn rerank(
    candidates: Vec<(i64, f64)>,
    nb: &Neighbourhood,
    req: &AnchorRequest,
) -> Vec<(i64, f64, Option<u32>)> {
    // Unreachable candidates rank as one hop beyond the limit.
    let floor = req
        .decay
        .powi(i32::try_from(req.max_hops + 1).unwrap_or(i32::MAX));
    let mut out: Vec<(i64, f64, Option<u32>)> = candidates
        .into_iter()
        .map(|(id, score)| {
            let hops = nb.hops.get(&id).copied();
            let final_score = match (req.mode, hops) {
                (Mode::Decay, Some(h)) => {
                    score * req.decay.powi(i32::try_from(h).unwrap_or(i32::MAX))
                }
                (Mode::Decay, None) => score * floor,
                _ => score,
            };
            (id, final_score, hops)
        })
        .filter(|(_, _, hops)| req.mode != Mode::Filter || hops.is_some())
        .collect();
    let key = |h: Option<u32>| h.unwrap_or(u32::MAX);
    match req.mode {
        Mode::Sort => out.sort_by(|a, b| {
            key(a.2)
                .cmp(&key(b.2))
                .then(b.1.total_cmp(&a.1))
                .then(a.0.cmp(&b.0))
        }),
        Mode::Decay | Mode::Filter => {
            out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        }
    }
    out
}

/// The `anchor` block of a response.
pub(super) fn summary(nb: &Neighbourhood, req: &AnchorRequest) -> JsonValue {
    json!({
        "iri": nb.anchor_iri,
        "mode": format!("{:?}", req.mode).to_lowercase(),
        "max_hops": req.max_hops,
        "decay": req.decay,
        "reached": nb.hops.len(),
        "truncated": nb.truncated_at.is_some(),
        "truncated_at_hop": nb.truncated_at,
        "hubs_not_expanded": nb.hubs_not_expanded,
    })
}

#[cfg(test)]
#[path = "search_anchor_tests.rs"]
mod tests;

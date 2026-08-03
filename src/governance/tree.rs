//! The attribution tree — reassembling who dispatched whom.
//!
//! SARC §9.5's **attribution dilution**: an orchestrator dispatches, a worker
//! acts, and a flat record cannot say which link was answerable. The paper's
//! remedy is that the trace be a *tree*, with worker subtrees attached to their
//! dispatch node and never summarised away.
//!
//! The trace this stack emits is a **sequence**. What it now carries per record
//! is the principal chain (`crate::governance::authority`, `hank/src/
//! attribution.rs`), and a chain is a path from the root — so the tree is
//! *reconstructible*. This module does that reconstruction, and is explicit
//! about the two places where reconstruction is weaker than structure.
//!
//! ## Where reconstruction loses information, and why it is said out loud
//!
//! **Sibling dispatches collapse.** Two separate dispatches of the same worker
//! by the same orchestrator produce the same chain, so they land on one node.
//! If those two runs did different things, the tree shows one node that did
//! both — which is attribution dilution reappearing at the reader rather than at
//! the record. [`Forest::collapsed`] reports every node this could apply to
//! instead of presenting the tree as if it were unambiguous.
//!
//! **Dispatch nodes can be implied rather than observed.** A record with chain
//! `[orchestrator, worker]` proves an orchestrator exists; it does not prove the
//! orchestrator's *own* actions were recorded. Interior nodes with no records of
//! their own are marked [`Node::implied`], because "this agent did nothing" and
//! "this agent's actions are not in this window" are different facts and only
//! one of them is good news.
//!
//! **An unattributed record is not placed at the root.** A record with no chain
//! goes to [`Forest::unattributed`]. Attaching it to whichever root happened to
//! be first would invent an answer to the exact question the tree exists to
//! answer.

use std::collections::BTreeMap;

use super::audit::TraceRecord;

/// One principal in the dispatch tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// This principal's id.
    pub principal: String,
    /// The full chain from the root to here, inclusive.
    pub path: Vec<String>,
    /// Indices of trace records attributed to exactly this chain.
    pub records: Vec<usize>,
    /// Children, in id order.
    pub children: Vec<Node>,
    /// True when nothing in the window is attributed to this node itself — it
    /// exists only because a descendant's chain named it.
    pub implied: bool,
}

impl Node {
    /// Records here and below.
    #[must_use]
    pub fn subtree_records(&self) -> usize {
        self.records.len()
            + self
                .children
                .iter()
                .map(Node::subtree_records)
                .sum::<usize>()
    }

    /// This node and every descendant, depth-first.
    fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for child in &self.children {
            child.walk(out);
        }
    }
}

/// The reassembled trace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forest {
    /// Roots, in id order. More than one means the window holds several
    /// independent dispatch chains, which is normal.
    pub roots: Vec<Node>,
    /// Records carrying no principal chain. Deliberately not attached anywhere.
    pub unattributed: Vec<usize>,
    /// Records read.
    pub records: usize,
}

impl Forest {
    /// Every node, depth-first.
    #[must_use]
    pub fn nodes(&self) -> Vec<&Node> {
        let mut out = Vec::new();
        for root in &self.roots {
            root.walk(&mut out);
        }
        out
    }

    /// Nodes holding more than one record, where separate dispatches of the
    /// same principal by the same caller would be indistinguishable.
    ///
    /// This is the honest statement of what a reconstructed tree cannot do. It
    /// is not an error — one agent legitimately performs many actions — it is
    /// the set of places where "one node" might be "several runs".
    #[must_use]
    pub fn collapsed(&self) -> Vec<&Node> {
        self.nodes()
            .into_iter()
            .filter(|n| n.records.len() > 1)
            .collect()
    }

    /// Interior nodes whose own actions are not in this window.
    #[must_use]
    pub fn implied(&self) -> Vec<&Node> {
        self.nodes().into_iter().filter(|n| n.implied).collect()
    }

    /// A summary carrying the reconstruction's own limits.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{roots} root(s), {nodes} node(s) over {records} record(s); \
             {unattributed} record(s) carry no chain and are NOT placed; \
             {implied} dispatch node(s) implied rather than observed; \
             {collapsed} node(s) where separate dispatches of the same principal \
             would be indistinguishable. Reconstructed from principal chains — \
             the trace is a sequence, not a tree.",
            roots = self.roots.len(),
            nodes = self.nodes().len(),
            records = self.records,
            unattributed = self.unattributed.len(),
            implied = self.implied().len(),
            collapsed = self.collapsed().len(),
        )
    }

    /// The tree as indented lines, for an operator.
    #[must_use]
    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        for root in &self.roots {
            render_node(root, 0, &mut out);
        }
        out
    }
}

fn render_node(node: &Node, depth: usize, out: &mut Vec<String>) {
    let note = if node.implied {
        " (implied — its own actions are not in this window)"
    } else {
        ""
    };
    out.push(format!(
        "{:indent$}{principal}: {own} record(s), {sub} in subtree{note}",
        "",
        indent = depth * 2,
        principal = node.principal,
        own = node.records.len(),
        sub = node.subtree_records(),
    ));
    for child in &node.children {
        render_node(child, depth + 1, out);
    }
}

/// A node under construction, keyed by child id so insertion is order-free.
#[derive(Default)]
struct Building {
    records: Vec<usize>,
    children: BTreeMap<String, Building>,
}

impl Building {
    fn finish(self, principal: String, path: Vec<String>) -> Node {
        let children = self
            .children
            .into_iter()
            .map(|(id, child)| {
                let mut child_path = path.clone();
                child_path.push(id.clone());
                child.finish(id, child_path)
            })
            .collect();
        Node {
            principal,
            implied: self.records.is_empty(),
            records: self.records,
            children,
            path,
        }
    }
}

/// Reassemble `trace` into a dispatch forest.
#[must_use]
pub fn build(trace: &[TraceRecord]) -> Forest {
    let mut roots: BTreeMap<String, Building> = BTreeMap::new();
    let mut unattributed = Vec::new();

    for (index, record) in trace.iter().enumerate() {
        let Some((first, rest)) = record.principal_chain.split_first() else {
            unattributed.push(index);
            continue;
        };
        let mut node = roots.entry(first.clone()).or_default();
        for link in rest {
            node = node.children.entry(link.clone()).or_default();
        }
        node.records.push(index);
    }

    Forest {
        roots: roots
            .into_iter()
            .map(|(id, building)| building.finish(id.clone(), vec![id]))
            .collect(),
        unattributed,
        records: trace.len(),
    }
}

/// Reassemble a raw JSONL trace, returning the forest and the unreadable count.
#[must_use]
pub fn build_jsonl(jsonl: &str) -> (Forest, usize) {
    let (records, unreadable) = super::audit::parse_trace(jsonl);
    (build(&records), unreadable)
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;

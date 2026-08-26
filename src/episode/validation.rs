use std::collections::HashSet;

use crate::error::{Error, Result};

use super::Node;

pub(super) fn reject_duplicate_node_names(nodes: &[Node]) -> Result<()> {
    let mut names = HashSet::with_capacity(nodes.len());
    for node in nodes {
        if !names.insert(node.name.as_str()) {
            return Err(Error::InvalidValue(format!(
                "duplicate node name '{}' in one episode — each name may appear only once. \
                 Repeated entries can append multiple rdfs:comment values; consolidate the \
                 node into one entry before POSTing (aegis-ibft0).",
                node.name
            )));
        }
    }
    Ok(())
}

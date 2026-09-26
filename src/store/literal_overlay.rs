//! Compose by exported RDF term identity, without changing audit encodings.
use super::Store;
use crate::{Fact, Result};
use rusqlite::params;
use std::collections::{BTreeMap, HashSet};

impl Store {
    pub(super) fn compose_literal_view(&self, overlay: i64, governed: bool) -> Result<Vec<Fact>> {
        let root = self.overlay_parent(overlay)?;
        let mut stmt = self.prepare("SELECT e,a,v,tx,valid_from,valid_to,op FROM facts WHERE g=?1 AND op=2 AND valid_to IS NULL")?;
        let tombstones = Self::collect_facts(&mut stmt, params![overlay])?;
        let key = |f: &Fact| (f.entity, f.attribute, f.value.term_key());
        let hidden: HashSet<_> = tombstones.iter().map(&key).collect();
        let parents = self.current_facts_in_graph(root)?;
        let owned: HashSet<_> = parents.iter().map(|f| (f.entity, f.attribute)).collect();
        let mut view = BTreeMap::new();
        for fact in parents {
            let k = key(&fact);
            if governed || !hidden.contains(&k) {
                view.entry(k).or_insert(fact);
            }
        }
        for fact in self.current_facts_in_graph(overlay)? {
            let k = key(&fact);
            if !governed
                || (!owned.contains(&(fact.entity, fact.attribute)) && !hidden.contains(&k))
            {
                view.insert(k, fact);
            }
        }
        Ok(view.into_values().collect())
    }
}

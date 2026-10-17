//! Bounded exact-entity reads for opt-in task briefing.
use super::Store;
use crate::{Result, types::Fact};
use rusqlite::params;

impl Store {
    /// A bounded current-fact window for one exact entity and graph.
    /// Oversized values are omitted explicitly, never silently truncated into facts.
    pub(crate) fn briefing_facts(&self, entity: i64, graph: i64) -> Result<(Vec<Fact>, bool)> {
        let oversized: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM facts WHERE e=?1 AND g=?2 AND op=1 AND valid_to IS NULL AND length(v)>65536)",
            params![entity, graph], |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT e,a,v,tx,valid_from,valid_to,op FROM facts WHERE e=?1 AND g=?2 AND op=1 AND valid_to IS NULL AND length(v)<=65536 ORDER BY tx DESC,a LIMIT 65",
        )?;
        let mut facts = Self::collect_visible_facts(&mut stmt, params![entity, graph])?;
        let capped = oversized || facts.len() >= 64;
        facts.truncate(64);
        Ok((facts, capped))
    }
}

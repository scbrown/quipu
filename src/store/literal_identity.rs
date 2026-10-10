//! Indexed compatibility lookup without rewriting historical literal blobs.
use super::Store;
use crate::{Result, Value, namespace};
use rusqlite::params;

impl Store {
    /// Include attached graphs for query lookup, local facts only for writes.
    pub(crate) fn literal_aliases(&self, value: &Value, composed: bool) -> Result<Vec<Vec<u8>>> {
        let mut aliases = value.physical_aliases();
        let key = value.term_key();
        let nan = Value::Typed {
            lexical: "NaN".into(),
            datatype: namespace::XSD_DOUBLE.into(),
        };
        if key == nan.term_key() {
            let facts = if composed {
                self.facts_source()
            } else {
                "facts"
            };
            // The range is a BLOB range over the existing value-leading index.
            // Do not reconstruct NaN payloads or reinterpret their stored bits.
            let mut stmt = self.prepare(&format!(
                "SELECT DISTINCT v FROM {facts} WHERE v >= ?1 AND v < ?2"
            ))?;
            let mut rows = stmt.query(params![vec![3_u8], vec![4_u8]])?;
            while let Some(row) = rows.next()? {
                let bytes: Vec<u8> = row.get(0)?;
                if Value::from_bytes(&bytes)?.term_key() == key {
                    aliases.push(bytes);
                }
            }
        }
        aliases.sort_unstable();
        aliases.dedup();
        Ok(aliases)
    }
}

#[cfg(test)]
mod tests;

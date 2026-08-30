//! Identity-orphan planning for snapshot retractions.

use rusqlite::params;

use crate::error::Result;
use crate::types::{Fact, Value};

use super::Store;
use super::retraction::IdentityOrphan;

impl Store {
    /// Term ids for `rdfs:label` and `rdf:type`, or `None` if never interned.
    pub(super) fn identity_predicate_ids(&self) -> Result<(Option<i64>, Option<i64>)> {
        Ok((
            self.lookup(crate::namespace::RDFS_LABEL)?,
            self.lookup(crate::namespace::RDF_TYPE)?,
        ))
    }

    /// Find entities that would lose identity while other writes reference them.
    pub(super) fn identity_orphans(
        &self,
        in_scope: &[Fact],
        source_tag: &str,
    ) -> Result<Vec<IdentityOrphan>> {
        let started = crate::time::Stopwatch::start();
        let limit_ms = self.search_config().query_timeout_ms;
        let deadline = (limit_ms > 0).then(|| crate::time::Deadline::after_millis(limit_ms));
        self.identity_orphans_until(in_scope, source_tag, started, deadline)
    }

    pub(super) fn identity_orphans_until(
        &self,
        in_scope: &[Fact],
        source_tag: &str,
        started: crate::time::Stopwatch,
        deadline: Option<crate::time::Deadline>,
    ) -> Result<Vec<IdentityOrphan>> {
        let check_deadline = || {
            if deadline.is_some_and(|dl| dl.passed()) {
                return Err(crate::error::Error::QueryTimeout {
                    elapsed_ms: started.elapsed_ms(),
                    limit_ms: deadline
                        .map(|dl| dl.millis_from(&started))
                        .unwrap_or_default(),
                });
            }
            Ok(())
        };
        check_deadline()?;
        let (label_id, type_id) = self.identity_predicate_ids()?;
        if label_id.is_none() && type_id.is_none() {
            return Ok(Vec::new());
        }

        let mut entities: Vec<i64> = in_scope
            .iter()
            .filter(|f| Some(f.attribute) == label_id || Some(f.attribute) == type_id)
            .map(|f| f.entity)
            .collect();
        entities.sort_unstable();
        entities.dedup();

        let mut orphans = Vec::new();
        for entity in entities {
            // This planning runs while the caller owns the sole writer mutex.
            check_deadline()?;
            if !self.has_surviving_reference(entity, source_tag)? {
                continue;
            }
            let declared_label = in_scope
                .iter()
                .any(|f| f.entity == entity && Some(f.attribute) == label_id);
            let declared_type = in_scope
                .iter()
                .any(|f| f.entity == entity && Some(f.attribute) == type_id);
            let lost_label =
                declared_label && !self.has_surviving_predicate(entity, label_id, source_tag)?;
            let lost_type =
                declared_type && !self.has_surviving_predicate(entity, type_id, source_tag)?;
            if lost_label || lost_type {
                orphans.push(IdentityOrphan {
                    entity,
                    lost_label,
                    lost_type,
                });
            }
        }
        Ok(orphans)
    }

    /// Is `entity` the subject or object of an active fact from another source?
    fn has_surviving_reference(&self, entity: i64, source_tag: &str) -> Result<bool> {
        let as_object = Value::Ref(entity).to_bytes();
        // Separate probes preserve idx_geav and idx_active_vge; combining them
        // with OR made SQLite full-scan facts once per candidate (aegis-ffeud).
        let mut subject = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
               AND f.e = ?1 AND (t.source IS NULL OR t.source <> ?2) LIMIT 1",
        )?;
        if subject.exists(params![entity, source_tag])? {
            return Ok(true);
        }
        let mut object = self.conn.prepare(
            "SELECT 1 FROM facts f JOIN transactions t ON f.tx = t.id \
             WHERE f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
               AND f.v = ?1 AND (t.source IS NULL OR t.source <> ?2) LIMIT 1",
        )?;
        Ok(object.exists(params![as_object, source_tag])?)
    }
}

//! Named-graph overlays — the multi-tenant / multi-branch extension substrate
//! (aegis-g1al / quipu #36/#37), implementing the model settled with hank/weaver
//! (see aegis `docs/designs/quipu-named-graph-overlays-20260718.md`).
//!
//! An overlay extends a committed **parent branch** (ROOT by default) WITHOUT
//! mutating it. Two write primitives, one uniform read:
//!
//! - `overlay_create(iri, parent)` — register an overlay-class graph, BOUND ONCE
//!   to its committed parent branch. Idempotent; rebinding to a different parent
//!   is an error (the binding is unforgeable, per the compose contract).
//! - `overlay_write(g, Assert|Retract|Tombstone, e,a,v)` — Assert/Retract are
//!   graph-scoped writes; **Tombstone** marks a specific `(e,a,v)` from the
//!   parent root ABSENT in this overlay's composed view, without touching root.
//! - `compose_view(g)` — resolve the stack `[overlay > parent-branch-root]` with
//!   the single uniform rule: a triple is present iff **asserted and not
//!   tombstoned**, nearest-overlay-wins. Overlay asserts shadow the root; overlay
//!   tombstones hide root triples; everything else falls through to root.
//!
//! Cardinality lives in the caller (hank emits `tombstone(old)+assert(new)` for a
//! single-valued replace); quipu stays semantics-agnostic and carries no
//! per-predicate cardinality — one rule for every predicate.

use rusqlite::{OptionalExtension, params};

use crate::error::{Error, Result};
use crate::types::{Fact, Op, Value};

use super::{Datum, Store};

impl Store {
    /// Register an overlay-class graph bound to a committed `parent_branch`
    /// (pass `0` for ROOT). Returns the overlay's graph id (the interned term id
    /// of `overlay_iri`). Idempotent: re-creating with the SAME parent is a
    /// no-op; re-creating with a DIFFERENT parent, or over a non-overlay graph,
    /// is an error — the overlay↔branch binding is fixed at create (bind-once).
    pub fn overlay_create(&self, overlay_iri: &str, parent_branch: i64) -> Result<i64> {
        // The parent must be a registered COMMITTED graph (ROOT g=0 is seeded).
        let parent_class: Option<String> = self
            .conn
            .query_row(
                "SELECT class FROM graphs WHERE g = ?1",
                params![parent_branch],
                |r| r.get(0),
            )
            .optional()?;
        match parent_class.as_deref() {
            Some("committed") => {}
            Some(other) => {
                return Err(Error::InvalidValue(format!(
                    "parent branch g={parent_branch} is class '{other}'; an overlay must extend a committed graph"
                )));
            }
            None => {
                return Err(Error::InvalidValue(format!(
                    "parent branch g={parent_branch} is not a registered graph"
                )));
            }
        }

        let overlay_g = self.intern(overlay_iri)?;
        if overlay_g == parent_branch {
            return Err(Error::InvalidValue(
                "an overlay cannot be its own parent branch".into(),
            ));
        }

        // Bind-once: idempotent on identical (class, parent); reject a rebind.
        let existing: Option<(String, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT class, parent_branch FROM graphs WHERE g = ?1",
                params![overlay_g],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((class, pb)) = existing {
            if class != "overlay" || pb != Some(parent_branch) {
                return Err(Error::InvalidValue(format!(
                    "graph g={overlay_g} already exists as class '{class}' parent={pb:?}; cannot rebind to overlay/parent={parent_branch}"
                )));
            }
            return Ok(overlay_g);
        }

        self.conn.execute(
            "INSERT INTO graphs (g, class, parent_branch, created_at) VALUES (?1, 'overlay', ?2, ?3)",
            params![overlay_g, parent_branch, crate::time::now_iso()],
        )?;
        Ok(overlay_g)
    }

    /// The committed parent-branch graph id an overlay is bound to. Errors if `g`
    /// is not a registered overlay.
    pub fn overlay_parent(&self, overlay_g: i64) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT parent_branch FROM graphs WHERE g = ?1 AND class = 'overlay'",
                params![overlay_g],
                |r| r.get(0),
            )
            .optional()?
            .flatten()
            .ok_or_else(|| {
                Error::InvalidValue(format!("g={overlay_g} is not a registered overlay"))
            })
    }

    /// The class of a graph (`committed` | `overlay`), or `None` if unregistered.
    pub fn graph_class(&self, g: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT class FROM graphs WHERE g = ?1", params![g], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Write one primitive into an overlay. `Assert` / `Retract` are graph-scoped
    /// writes (base un-mutated); `Tombstone` records that `(e,a,v)` is absent in
    /// this overlay's composed view. `g` must be a registered overlay.
    pub fn overlay_write(
        &mut self,
        overlay_g: i64,
        op: Op,
        entity: i64,
        attribute: i64,
        value: Value,
        timestamp: &str,
    ) -> Result<i64> {
        // Reject writes to a non-overlay graph (committed graphs are written via
        // transact/transact_to_graph, not this overlay path).
        let _parent = self.overlay_parent(overlay_g)?;

        match op {
            Op::Assert | Op::Retract => {
                let d = Datum {
                    entity,
                    attribute,
                    value,
                    valid_from: timestamp.to_string(),
                    valid_to: None,
                    op,
                };
                self.transact_to_graph(&[d], timestamp, None, Some("overlay"), overlay_g)
            }
            Op::Tombstone => {
                let v_bytes = value.to_bytes();
                let sp = self.conn.savepoint()?;
                sp.execute(
                    "INSERT INTO transactions (timestamp, actor, source) VALUES (?1, ?2, ?3)",
                    params![timestamp, Option::<&str>::None, Some("overlay-tombstone")],
                )?;
                let tx_id = sp.last_insert_rowid();
                // Idempotent: one active tombstone per (e,a,v) in this overlay.
                let exists: bool = sp
                    .query_row(
                        "SELECT 1 FROM facts \
                         WHERE e = ?1 AND a = ?2 AND v = ?3 AND g = ?4 AND op = 2 AND valid_to IS NULL \
                         LIMIT 1",
                        params![entity, attribute, v_bytes, overlay_g],
                        |_| Ok(true),
                    )
                    .optional()?
                    .unwrap_or(false);
                if !exists {
                    sp.execute(
                        "INSERT INTO facts (e, a, v, g, tx, valid_from, valid_to, op) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 2)",
                        params![entity, attribute, v_bytes, overlay_g, tx_id, timestamp],
                    )?;
                }
                sp.commit()?;
                Ok(tx_id)
            }
        }
    }

    /// Resolve an overlay's composed view over the stack
    /// `[overlay > parent-branch-root]`. Returns the effective present facts: a
    /// triple is present iff **asserted and not tombstoned, nearest-overlay-wins**.
    ///
    /// Concretely — overlay asserts are always present; a root assert is present
    /// unless the overlay tombstones it (or re-asserts it, in which case the
    /// overlay row is the one returned). Root is never mutated; this is a pure
    /// read. `overlay_g` must be a registered overlay.
    pub fn compose_view(&self, overlay_g: i64) -> Result<Vec<Fact>> {
        let root_g = self.overlay_parent(overlay_g)?;
        // (1) every live overlay assertion (shadows root); UNION (2) live root
        // assertions the overlay neither tombstones nor re-asserts. The outer
        // GROUP BY e,a,v collapses a triple that was re-asserted across several
        // transactions (multiple current op=1 rows) to ONE composed triple —
        // the same dedup the SPARQL BGP path applies (GH#13). Without it the
        // composed view repeats every re-asserted base fact once per assertion.
        let mut stmt = self.conn.prepare(
            "SELECT e, a, v, tx, valid_from, valid_to, op FROM ( \
               SELECT e, a, v, tx, valid_from, valid_to, op FROM facts \
               WHERE g = ?1 AND op = 1 AND valid_to IS NULL \
               UNION ALL \
               SELECT r.e, r.a, r.v, r.tx, r.valid_from, r.valid_to, r.op FROM facts r \
               WHERE r.g = ?2 AND r.op = 1 AND r.valid_to IS NULL \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM facts t WHERE t.g = ?1 AND t.op = 2 AND t.valid_to IS NULL \
                     AND t.e = r.e AND t.a = r.a AND t.v = r.v) \
                 AND NOT EXISTS ( \
                   SELECT 1 FROM facts o WHERE o.g = ?1 AND o.op = 1 AND o.valid_to IS NULL \
                     AND o.e = r.e AND o.a = r.a AND o.v = r.v) \
             ) GROUP BY e, a, v ORDER BY e, a",
        )?;
        Self::collect_facts(&mut stmt, params![overlay_g, root_g])
    }
}

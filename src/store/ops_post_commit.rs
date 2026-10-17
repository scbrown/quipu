//! Effects that run after the transaction commit boundary.

use super::{Datum, Staged, Store};
use crate::{embedding, error::Result};

impl Store {
    /// Post-commit side effects: auto-embedding and reactive observers. Runs
    /// after the savepoint is released (the write is durable relative to the
    /// parent), preserving the previous ordering.
    pub(in crate::store) fn after_commit_hooks(
        &mut self,
        datums: &[Datum],
        staged: Staged,
        timestamp: &str,
        source: Option<&str>,
        _graph: i64,
    ) -> Result<()> {
        // Post-transact hook: auto-embed touched entities.
        // Skipped when a vector search delegate is set — embeddings belong in
        // the external index layer (e.g. Bobbin's LanceDB).
        if self.embedding_config.auto_embed
            && self.vector_delegate.is_none()
            && let Some(provider) = &self.embedding_provider
        {
            let entity_ids = embedding::touched_entity_ids(datums);
            if self.defer_auto_embed {
                // Deferred path: do only the cheap parts here —
                // close retired embeddings and build texts — and queue the
                // multi-second ONNX embed for the caller to run OUTSIDE the
                // store lock. An episode write's lock hold-time drops from
                // ~seconds (embed under lock) to the transact itself.
                let work = embedding::collect_embed_work(self, &entity_ids, timestamp, datums)?;
                if !work.is_empty() {
                    match &mut self.pending_embed {
                        Some(pending) => pending.merge(work),
                        None => self.pending_embed = Some(work),
                    }
                }
            } else {
                embedding::auto_embed_entities(
                    self,
                    provider,
                    &entity_ids,
                    timestamp,
                    self.embedding_config.embed_batch_size,
                    datums,
                )?;
            }
        }

        // Notify registered observers. Observers may call store.transact()
        // directly for per-rule provenance. Cloning the Arc vec first
        // avoids a borrow conflict with &mut self.
        #[cfg(feature = "reactive-reasoner")]
        {
            use crate::store::Delta;

            if !self.observers.is_empty() {
                let delta = Delta {
                    graph: _graph,
                    tx: staged.tx_id,
                    asserts: staged.asserts,
                    retracts: staged.retracts,
                    source: source.map(String::from),
                };

                let observers: Vec<_> = self.observers.clone();
                for obs in &observers {
                    obs.after_commit(self, &delta)?;
                }
            }
        }

        // `staged` (its tx id / written indices) and `source` are only consumed
        // by the reactive-reasoner observer dispatch above.
        #[cfg(not(feature = "reactive-reasoner"))]
        let _ = (staged, source);

        Ok(())
    }
}

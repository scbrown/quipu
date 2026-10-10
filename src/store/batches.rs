//! Multi-graph batch writes: one SPARQL Update touching several graphs commits
//! all-or-nothing, one transaction per graph (aegis-xajsgn).

use crate::error::Result;

use super::{Datum, Store};

impl Store {
    /// Atomically apply already-planned changes to multiple RDF graphs.
    ///
    /// Each graph still passes through the normal authority, SHACL, OWL, and
    /// governed-policy gates. The outer savepoint makes a multi-operation
    /// SPARQL Update all-or-nothing across those graph-scoped transactions.
    pub fn transact_graph_batches(
        &mut self,
        batches: &[(i64, Vec<Datum>)],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<()> {
        self.transact_graph_batches_tx(batches, timestamp, actor, source)
            .map(|_| ())
    }

    /// [`Self::transact_graph_batches`], returning each graph's transaction
    /// id as `(graph, tx)` in batch order, so a caller can report what it
    /// committed (aegis-xajsgn).
    pub fn transact_graph_batches_tx(
        &mut self,
        batches: &[(i64, Vec<Datum>)],
        timestamp: &str,
        actor: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<(i64, i64)>> {
        self.conn.execute_batch("SAVEPOINT quipu_multi_graph")?;
        let mut txs = Vec::with_capacity(batches.len());
        for (graph, datums) in batches {
            match self.transact_to_graph(datums, timestamp, actor, source, *graph) {
                Ok(tx) => txs.push((*graph, tx)),
                Err(error) => {
                    self.conn.execute_batch(
                        "ROLLBACK TO quipu_multi_graph; RELEASE quipu_multi_graph",
                    )?;
                    self.read_model.borrow_mut().clear();
                    return Err(error);
                }
            }
        }
        self.conn.execute_batch("RELEASE quipu_multi_graph")?;
        Ok(txs)
    }
}

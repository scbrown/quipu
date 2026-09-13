//! Graph label writes keyed by graph id, including the ROOT sentinel.
use super::*;

impl Store {
    /// Declare a graph label that expires at `valid_to`.
    /// After expiry it reads as undeclared; expiry does not preserve a value.
    pub fn set_graph_label_until(
        &mut self,
        graph_iri: &str,
        label: &GraphLabel,
        timestamp: &str,
        valid_to: Option<&str>,
        actor: Option<&str>,
    ) -> Result<i64> {
        let graph_id = self.intern(graph_iri)?;
        self.write_graph_label(graph_id, graph_iri, label, timestamp, valid_to, actor)
    }

    /// Declare a label on a registered graph by id. Zero selects ROOT.
    /// ROOT metadata uses its reserved IRI, while its cache stays on graph zero.
    /// A named graph using the reserved IRI is ambiguous and is refused.
    ///
    /// # Errors
    /// As `set_graph_label_until`, or when ROOT's metadata IRI is a named graph.
    pub fn set_graph_label_by_id(
        &mut self,
        graph_id: i64,
        label: &GraphLabel,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<i64> {
        let iri = if graph_id == crate::schema::ROOT_GRAPH {
            if let Some(id) = self.lookup(crate::schema::ROOT_GRAPH_IRI)? {
                let exists: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM graphs WHERE g=?1)",
                    [id],
                    |r| r.get(0),
                )?;
                if exists {
                    return Err(Error::InvalidValue(
                        "ROOT label IRI is already a named graph".into(),
                    ));
                }
            }
            crate::schema::ROOT_GRAPH_IRI.to_string()
        } else {
            self.resolve(graph_id)?
        };
        self.write_graph_label(graph_id, &iri, label, timestamp, None, actor)
    }

    fn write_graph_label(
        &mut self,
        graph_id: i64,
        graph_iri: &str,
        label: &GraphLabel,
        timestamp: &str,
        valid_to: Option<&str>,
        actor: Option<&str>,
    ) -> Result<i64> {
        if let Some(end) = valid_to
            && (end.len() != 20 || !end.ends_with('Z') || end <= timestamp)
        {
            return Err(Error::InvalidValue(format!(
                "label valid_to '{end}' must be canonical UTC and later than valid_from '{timestamp}'"
            )));
        }
        if label.is_empty() {
            return Err(Error::InvalidValue(format!(
                "set_graph_label on '{graph_iri}' declares no axis; to clear a \
                 label, retract the meta-graph facts explicitly"
            )));
        }

        let meta_g = self.meta_graph_id()?;
        let subject = self.intern(graph_iri)?;

        let mut datums: Vec<Datum> = Vec::new();
        if let Some(f) = label.freshness {
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_FRESHNESS)?,
                value: Value::Str(f.as_str().to_string()),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
        }
        if let Some(d) = label.durability {
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_DURABILITY)?,
                value: Value::Str(d.as_str().to_string()),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
        }
        if let Some(t) = &label.trust {
            let trust_term = self.intern(&t.iri)?;
            let chain_term = self.intern(&t.chain)?;
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_TRUST)?,
                value: Value::Ref(trust_term),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
            // The chain and rank are facts about the TRUST VALUE, not about the
            // graph — that is what makes the ordering data rather than a
            // hardcoded enum, and what lets two consumers ship different chains.
            datums.push(Datum {
                entity: trust_term,
                attribute: self.intern(QUIPU_IN_CHAIN)?,
                value: Value::Ref(chain_term),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
            datums.push(Datum {
                entity: trust_term,
                attribute: self.intern(QUIPU_TRUST_RANK)?,
                value: Value::Int(t.rank),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
        }
        if let Some(p) = &label.policy {
            let attr = self.intern(QUIPU_POLICY_CLASS)?;
            for tok in p.tokens() {
                datums.push(Datum {
                    entity: subject,
                    attribute: attr,
                    value: Value::Str(tok.to_string()),
                    valid_from: timestamp.to_string(),
                    valid_to: valid_to.map(str::to_string),
                    op: Op::Assert,
                });
            }
        }
        if let Some(k) = &label.kind {
            datums.push(Datum {
                entity: subject,
                attribute: self.intern(QUIPU_DATA_KIND)?,
                value: Value::Str(k.as_str().to_string()),
                valid_from: timestamp.to_string(),
                valid_to: valid_to.map(str::to_string),
                op: Op::Assert,
            });
        }

        let policy_encoded = match &label.policy {
            Some(p) => Some(encode_policy(p)?),
            None => None,
        };

        // One savepoint over BOTH writes. `transact_to_graph` opens its own
        // `quipu_transact` inside this one — the same nesting `speculate` uses.
        self.conn.execute_batch("SAVEPOINT quipu_set_label")?;
        let result = (|| -> Result<i64> {
            let tx =
                self.transact_to_graph(&datums, timestamp, actor, Some("graph-label"), meta_g)?;
            let updated = self.conn.execute(
                "UPDATE graphs SET fresh_rank = ?2, durability_rank = ?3, trust_rank = ?4, trust_chain = ?5, \
                 policy = ?6, labels_tx = ?7, labels_valid_to = ?8, data_kind = ?9 WHERE g = ?1",
                params![
                    graph_id,
                    label.freshness.map(|f| f as i64),
                    label.durability.map(|d| d as i64),
                    label.trust.as_ref().map(|t| t.rank),
                    label.trust.as_ref().map(|t| t.chain.clone()),
                    policy_encoded,
                    tx,
                    valid_to,
                    label.kind.as_ref().map(|k| k.as_str().to_string()),
                ],
            )?;
            // An UPDATE matching no row is not an error in SQL, and that is
            // exactly how this would go wrong quietly: labelling a graph that
            // was never registered in `graphs` (a typo'd IRI, most likely)
            // would write the meta-graph facts and cache nothing, leaving
            // permanent drift that only `doctor labels` would ever surface.
            // Refuse instead, and let the savepoint take the facts back out.
            if updated != 1 {
                return Err(Error::InvalidValue(format!(
                    "cannot label '{graph_iri}': it is not a registered graph \
                     (no row in `graphs`). Create it first — labelling an \
                     unregistered graph would write facts the cache could \
                     never mirror."
                )));
            }
            Ok(tx)
        })();

        match result {
            Ok(tx) => {
                self.conn.execute_batch("RELEASE quipu_set_label")?;
                Ok(tx)
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_set_label; RELEASE quipu_set_label");
                Err(e)
            }
        }
    }
}

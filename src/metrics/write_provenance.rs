//! How completely committed writes declared their provenance (aegis-7zp4rc).
//!
//! One count per COMMITTED write transaction, by normalized client, route
//! template and completeness class (`crate::write_provenance`). Both labels are
//! already bounded (32 clients, the router's write routes), and the header
//! VALUES are never labels. `..._missing_total` names which required field a
//! partial or absent declaration lacked, for diagnosis.

use super::*;
use crate::write_provenance::{Completeness, RequestProvenance};

#[derive(Default)]
pub(super) struct WriteProvenanceMetrics {
    /// (client, endpoint, completeness) -> committed write transactions.
    writes: Mutex<BTreeMap<(String, String, Completeness), u64>>,
    /// (client, field) -> committed write transactions missing that field.
    missing: Mutex<BTreeMap<(String, &'static str), u64>>,
}

impl WriteProvenanceMetrics {
    pub(super) fn observe(&self, p: &RequestProvenance) {
        *self
            .writes
            .lock()
            .unwrap()
            .entry((p.client.clone(), p.endpoint.clone(), p.completeness))
            .or_insert(0) += 1;
        let mut missing = self.missing.lock().unwrap();
        for field in &p.missing {
            *missing.entry((p.client.clone(), field)).or_insert(0) += 1;
        }
    }

    pub(super) fn render(&self, out: &mut String) {
        out.push_str(
            "# HELP quipu_write_provenance_total Committed write transactions by client, route \
             template and how completely the request declared its provenance: complete (agent, \
             harness and host, plus session and model for an agent harness), partial, or absent \
             (no X-Quipu-Agent/-Harness/-Model/-Session/-Host header). A refused request commits \
             nothing and counts nothing.\n\
             # TYPE quipu_write_provenance_total counter\n",
        );
        for ((client, endpoint, completeness), n) in self.writes.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_write_provenance_total{{client=\"{}\",endpoint=\"{}\",provenance=\"{}\"}} {n}",
                esc(client),
                esc(endpoint),
                completeness.as_str()
            );
        }
        out.push_str(
            "# HELP quipu_write_provenance_missing_total Committed write transactions missing a \
             required provenance field, by client and field.\n\
             # TYPE quipu_write_provenance_missing_total counter\n",
        );
        for ((client, field), n) in self.missing.lock().unwrap().iter() {
            let _ = writeln!(
                out,
                "quipu_write_provenance_missing_total{{client=\"{}\",field=\"{field}\"}} {n}",
                esc(client)
            );
        }
    }
}

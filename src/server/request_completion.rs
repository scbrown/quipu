//! Terminal accounting for a polled HTTP request future, including abandonment.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Cancellations(Mutex<BTreeMap<(String, String), u64>>);

impl Cancellations {
    fn observe(&self, client: &str, endpoint: &str) {
        let mut map = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Match the existing attribution budget: 31 named clients plus other.
        // Endpoint labels are route templates supplied by the middleware.
        let known: BTreeSet<&str> = map
            .keys()
            .map(|(client, _)| client.as_str())
            .filter(|client| *client != "other")
            .collect();
        let client = if known.contains(client) || known.len() < 31 {
            client
        } else {
            "other"
        };
        *map.entry((client.into(), endpoint.into())).or_default() += 1;
    }

    pub(super) fn render(&self, out: &mut String) {
        out.push_str(
            "# HELP quipu_http_requests_cancelled_total Request futures dropped before a response, by caller and route template.\n\
             # TYPE quipu_http_requests_cancelled_total counter\n",
        );
        for ((client, endpoint), count) in self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            // JSON string escaping also escapes quotes, slashes and newlines in
            // Prometheus labels. Route templates and clients are already bounded.
            let _ = writeln!(
                out,
                "quipu_http_requests_cancelled_total{{client={},endpoint={}}} {count}",
                serde_json::to_string(client).unwrap(),
                serde_json::to_string(endpoint).unwrap()
            );
        }
    }
}

pub(super) fn cancellations() -> &'static Cancellations {
    static COUNTERS: OnceLock<Cancellations> = OnceLock::new();
    COUNTERS.get_or_init(Cancellations::default)
}

pub(super) struct Completion<'a> {
    pub(super) id: u64,
    pub(super) client: String,
    pub(super) task: String,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) endpoint: String,
    pub(super) declared_host: Option<String>,
    pub(super) declared_agent: Option<String>,
    pub(super) started: Instant,
    pub(super) metrics: &'a quipu::metrics::Metrics,
    pub(super) cancellations: &'a Cancellations,
    pub(super) emit: Box<dyn FnMut(String) + Send + 'a>,
    pub(super) finished: bool,
}

impl Completion<'_> {
    pub(super) fn finish(
        &mut self,
        status: u16,
        auth: quipu::request_usage::AuthOutcome,
        usage: Option<quipu::request_usage::RequestUsage>,
        cancelled: bool,
    ) {
        if self.finished {
            return;
        }
        // Disarm first so a later panic cannot manufacture a second terminal event.
        self.finished = true;
        let elapsed = self.started.elapsed();
        self.metrics
            .observe_request(&self.endpoint, status, elapsed.as_secs_f64());
        self.metrics.observe_client(
            &self.client,
            &self.task,
            &self.endpoint,
            elapsed.as_secs_f64(),
        );
        if cancelled {
            self.cancellations.observe(&self.client, &self.endpoint);
        } else {
            self.metrics
                .observe_auth_result(&self.client, &self.endpoint, &self.method, status);
        }
        let log = quipu::request_usage::with_declared_attribution(
            quipu::request_usage::structured_request_log(
                "request_complete",
                self.id,
                &self.client,
                &self.task,
                &self.method,
                &self.path,
                &self.endpoint,
                Some(status),
                Some(elapsed.as_millis()),
                auth,
                usage,
            ),
            self.declared_host.as_deref(),
            self.declared_agent.as_deref(),
        );
        let mut log: serde_json::Value = serde_json::from_str(&log).unwrap();
        log["completion_outcome"] = if cancelled { "cancelled" } else { "response" }.into();
        (self.emit)(log.to_string());
    }
}

impl Drop for Completion<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // 499 is a synthetic log/metric status; no response was delivered.
            // A dropped HTTP future does not prove its blocking worker stopped.
            self.finish(499, quipu::request_usage::AuthOutcome::Pending, None, true);
        }
    }
}

#[cfg(test)]
#[path = "request_completion_tests.rs"]
mod tests;

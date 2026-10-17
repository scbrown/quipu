//! Default-off, one-attempt diagnostics outside every store guard.
use quipu::config::QueryCaptureConfig;
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[path = "query_capture/private_file.rs"]
mod private_file;
#[cfg(test)]
#[path = "query_capture/tests.rs"]
mod tests;

const MAX_RECORD: usize = 16 * 1024;
const OPTIONS: &[&str] = &[
    "graph",
    "fork",
    "include_kinds",
    "entailment",
    "valid_at",
    "tx",
    "row_labels",
    "federated",
    "verbose",
    "_sparql_protocol",
];

pub(super) struct Capture {
    config: QueryCaptureConfig,
    starts: Instant,
    expires: Instant,
    consumed: AtomicBool,
    sender: mpsc::SyncSender<(Vec<u8>, &'static str)>,
}

fn event(window: &str, outcome: &str) {
    eprintln!(
        "{}",
        serde_json::json!({"event":"query_capture_state", "window_id":window, "outcome":outcome})
    );
}

fn validate(config: &QueryCaptureConfig, now: u64) -> Result<(), &'static str> {
    if config.query_sha256.len() != 64
        || !config
            .query_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid_selector");
    }
    if !(1..=4096).contains(&config.query_bytes)
        || config.window_id.is_empty()
        || config.window_id.len() > 64
        || !config
            .window_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("invalid_selector");
    }
    if config.end_epoch_secs <= config.start_epoch_secs
        || config.end_epoch_secs - config.start_epoch_secs > 300
        || now >= config.end_epoch_secs
        || config.start_epoch_secs > now.saturating_add(900)
    {
        return Err("invalid_window");
    }
    Ok(())
}

/// Disabled and invalid configurations never change normal request outcomes.
pub(super) fn start(config: Option<&QueryCaptureConfig>, db_path: &str) -> Option<Arc<Capture>> {
    let config = config.filter(|c| c.enabled)?;
    let now = quipu::time::epoch_secs();
    if let Err(reason) = validate(config, now) {
        // Invalid input is deliberately not echoed into a log.
        event("REDACTED", reason);
        return None;
    }
    let Ok(file) = private_file::claim(db_path, &config.window_id) else {
        event(&config.window_id, "claim_refused");
        return None;
    };
    let initialized = Instant::now();
    let starts = initialized + Duration::from_secs(config.start_epoch_secs.saturating_sub(now));
    let expires = initialized + Duration::from_secs(config.end_epoch_secs - now);
    let (sender, receiver) = mpsc::sync_channel::<(Vec<u8>, &'static str)>(1);
    let window = config.window_id.clone();
    let worker = std::thread::Builder::new()
        .name("query-capture".into())
        .spawn(move || {
            let mut file = file;
            let (bytes, outcome) =
                match receiver.recv_timeout(expires.saturating_duration_since(Instant::now())) {
                    Ok(record) => record,
                    Err(_) => (
                        b"{\"outcome\":\"expired_or_stopped\"}\n".to_vec(),
                        "expired_or_stopped",
                    ),
                };
            let outcome = if file
                .write_all(&bytes)
                .and_then(|()| file.sync_all())
                .is_ok()
            {
                outcome
            } else {
                "write_unknown"
            };
            event(&window, outcome);
        });
    if worker.is_err() {
        event(&config.window_id, "worker_refused");
        return None;
    }
    event(&config.window_id, "armed");
    Some(Arc::new(Capture {
        config: config.clone(),
        starts,
        expires,
        consumed: AtomicBool::new(false),
        sender,
    }))
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_RECORD.saturating_sub(self.0.len()) {
            return Err(io::Error::other("record budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Serialize)]
struct Record<'a> {
    contract: &'static str,
    utc: String,
    source_git_sha: &'static str,
    source_git_dirty: bool,
    window_id: &'a str,
    query: &'a str,
    identity: &'a Value,
    options: Vec<(&'a str, &'a Value)>,
    omitted_input_fields: bool,
}

impl Capture {
    pub(super) fn observe(&self, input: &Value, identity: &Value, client: &str, is_post: bool) {
        let now = quipu::time::epoch_secs();
        let monotonic = Instant::now();
        if !is_post
            || client != "seeds"
            || now < self.config.start_epoch_secs
            || now >= self.config.end_epoch_secs
            || monotonic < self.starts
            || monotonic >= self.expires
            || identity.get("query_sha256").and_then(Value::as_str)
                != Some(self.config.query_sha256.as_str())
        {
            return;
        }
        let Some(query) = input
            .get("query")
            .and_then(Value::as_str)
            .filter(|q| q.len() == self.config.query_bytes)
        else {
            return;
        };
        if self.consumed.swap(true, Ordering::AcqRel) {
            return;
        }
        let options = OPTIONS
            .iter()
            .filter_map(|key| input.get(*key).map(|value| (*key, value)))
            .collect();
        let record = Record {
            contract: "quipu-private-query-capture/v1",
            utc: quipu::time::now_iso(),
            source_git_sha: env!("QUIPU_GIT_SHA"),
            source_git_dirty: env!("QUIPU_GIT_DIRTY") == "true",
            window_id: &self.config.window_id,
            query,
            identity,
            options,
            omitted_input_fields: input.as_object().is_some_and(|map| {
                map.keys()
                    .any(|key| key != "query" && !OPTIONS.contains(&key.as_str()))
            }),
        };
        let mut bytes = Bounded(Vec::new());
        if serde_json::to_writer(&mut bytes, &record).is_err() || bytes.write_all(b"\n").is_err() {
            event(&self.config.window_id, "record_refused");
            // Consume the claim even on overflow. Do not retry a later request.
            let _ = self.sender.try_send((
                b"{\"outcome\":\"record_refused\"}\n".to_vec(),
                "record_refused",
            ));
            return;
        }
        if self.sender.try_send((bytes.0, "captured")).is_err() {
            event(&self.config.window_id, "handoff_unknown");
        }
    }
}

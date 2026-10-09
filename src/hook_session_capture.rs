//! `quipu hook session-capture`: the Stop hook that solicits an agent ONCE PER
//! SESSION to (a) write a Quipu episode of the durable knowledge it produced
//! and (b) propose shape updates for any new entity kinds.
//!
//! Non-gating: the agent can act or reply "skip"; the solicitation fires at
//! most once per session and never traps the agent (block-once, guarded by
//! `stop_hook_active`).
//!
//! Contract: read the Stop-hook JSON on stdin, write at most one response JSON
//! on stdout, ALWAYS exit 0. Every failure path is SILENCE — a malformed Stop
//! response is worse than none, so nothing here prints anything it did not
//! build with `serde_json`.
//!
//! Environment:
//! - `GT_CREW`: the crew name; else derived from a `.../crew/<name>/...` cwd.
//! - `QUIPU_HOOK_CREWS`: space-separated crew allowlist (default `*`).
//! - `QUIPU_HOOK_STATE_DIR`: markers and the solicitation log
//!   (default `~/.quipu-hook`).
//! - `QUIPU_SERVER`: the server URL named in the message
//!   (default: the server's default bind address).
//! - `QUIPU_HOOK_GROUP`: the episode group named in the message
//!   (default [`DEFAULT_GROUP`]).

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

/// The episode group named when `QUIPU_HOOK_GROUP` is unset.
pub const DEFAULT_GROUP: &str = "default";

/// Markers older than this are reaped.
const MARKER_TTL: Duration = Duration::from_secs(2 * 86_400);

/// The human-facing one-liner (terminal only).
const NOTE: &str = "Session knowledge capture (optional, one-time).";

/// Everything the hook reads from its environment, resolved up front so tests
/// can drive every arm without mutating the process environment.
#[derive(Debug, Clone)]
pub struct Settings {
    /// `GT_CREW`, if set and non-empty.
    pub crew: Option<String>,
    /// `QUIPU_HOOK_CREWS` (default `*`).
    pub scope: String,
    /// Where markers and `solicit-log.jsonl` live.
    pub state_dir: PathBuf,
    /// Server base URL named in the message.
    pub server: String,
    /// Episode group named in the message.
    pub group: String,
}

impl Settings {
    /// Resolve from the process environment. `None` when no state dir can be
    /// named (no `QUIPU_HOOK_STATE_DIR` and no `HOME`): the hook stays silent.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let state_dir = match var("QUIPU_HOOK_STATE_DIR") {
            Some(d) => PathBuf::from(d),
            // The SAME default as the shell hook this replaces, so existing
            // markers and the solicitation log carry over.
            None => PathBuf::from(var("HOME")?).join(".quipu-hook"),
        };
        let server = var("QUIPU_SERVER")
            .unwrap_or_else(|| format!("http://{}", quipu::ServerConfig::default().bind));
        Some(Self {
            crew: var("GT_CREW"),
            scope: var("QUIPU_HOOK_CREWS").unwrap_or_else(|| "*".into()),
            state_dir,
            server: server.trim_end_matches('/').to_string(),
            group: var("QUIPU_HOOK_GROUP").unwrap_or_else(|| DEFAULT_GROUP.into()),
        })
    }
}

/// Entry point for `quipu hook session-capture`.
pub fn run_stdio() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Some(settings) = Settings::from_env() else {
        return;
    };
    if let Some(resp) = respond(&input, &settings, SystemTime::now()) {
        // Serialised whole, then written once: a partial line is never emitted.
        if let Ok(text) = serde_json::to_string(&resp) {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{text}");
        }
    }
}

/// The crew name from a cwd of the form `.../crew/<name>[/...]`.
fn crew_from_cwd(cwd: &str) -> Option<String> {
    let rest = &cwd[cwd.rfind("/crew/")? + "/crew/".len()..];
    let name = rest.split('/').next().unwrap_or_default();
    (!name.is_empty()).then(|| name.to_string())
}

/// A session id safe to embed in a file name.
fn file_safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Decide the hook's response. `None` means stay silent. `now` is the clock
/// the marker reaper ages against.
#[must_use]
pub fn respond(input: &str, settings: &Settings, now: SystemTime) -> Option<Value> {
    let event: Value = serde_json::from_str(input).ok()?;
    let stop_active = event["stop_hook_active"].as_bool().unwrap_or(false);
    let session_id = event["session_id"].as_str().unwrap_or("unknown");
    let cwd = event["cwd"].as_str().unwrap_or("");

    // 0) Scope gate. Registration lives in a settings file shared by every
    //    crew workspace (harness hooks override rather than merge across
    //    scopes), so scoping happens HERE. Fail CLOSED: a crew we cannot
    //    positively identify is never interrupted. "*" widens WHO, never
    //    WHETHER-identified: it means any positively identified crew, because
    //    a name list rots as the roster changes.
    let crew = settings.crew.clone().or_else(|| crew_from_cwd(cwd))?;
    if settings.scope != "*" && !settings.scope.split_whitespace().any(|c| c == crew) {
        return None;
    }

    // 1) Loop guard: if we already forced a continuation this turn, let it stop.
    if stop_active {
        return None;
    }

    // 2) Once-per-session guard: a marker keyed by session id, claimed
    //    atomically (create_new) so two racing stops cannot both solicit.
    let dir = &settings.state_dir;
    std::fs::create_dir_all(dir).ok()?;
    let marker = dir.join(format!("solicited-{}", file_safe(session_id)));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .ok()?;

    // 2b) Reap stale markers. Runs once per session (we just claimed ours), so
    //     the dir self-limits to recent sessions without an external cron.
    reap_markers(dir, now);

    // 2c) DURABLE solicitation record. The marker is the once-only GUARD and
    //     is correctly ephemeral (reaped above), so it CANNOT be the
    //     denominator of an act-rate read: episodes are PERMANENT, and a
    //     decaying denominator over a permanent numerator makes the metric
    //     INFLATE the longer you wait — it fails in the direction that
    //     flatters, the one direction a metric must not fail. A metric must be
    //     able to FALL. This append-only log is that denominator:
    //       act rate = episodes carrying session_id / lines in this log
    //     Skip is DERIVED (solicited - acted), never self-reported.
    //     NB: this file name must NEVER match the reaper's `solicited-*` glob.
    //     FAIL CLOSED: if we cannot record the solicitation we do NOT solicit.
    //     An unlogged solicitation that is acted on lands in the numerator
    //     while the denominator misses it, inflating the very number this log
    //     keeps honest. Silence costs one datapoint; an uncounted solicitation
    //     corrupts the metric in the flattering direction. The marker is
    //     already claimed, so this session goes unsolicited AND uncounted —
    //     consistent either way.
    let line = json!({
        "ts": quipu::time::now_iso(),
        "session_id": session_id,
        "crew": crew,
    });
    append_line(&dir.join("solicit-log.jsonl"), &line).ok()?;

    // 3) Solicit once. decision:block forces ONE continuation so the agent
    //    SEES this (a non-blocking hook's output is discarded). The agent may
    //    act or reply "skip"; either way the next stop is allowed.
    //
    //    FIELD ROUTING IS LOAD-BEARING:
    //      reason        -> reaches the MODEL. The payload MUST live here.
    //      systemMessage -> reaches the user's TERMINAL ONLY, never the model.
    //    A payload in systemMessage is an interruption the agent cannot act on.
    Some(json!({
        "decision": "block",
        "reason": message(settings, session_id),
        "systemMessage": NOTE,
    }))
}

fn append_line(path: &Path, line: &Value) -> std::io::Result<()> {
    let mut text = serde_json::to_string(line)?;
    text.push('\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

fn reap_markers(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("solicited-")
        {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .filter(std::fs::Metadata::is_file)
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > MARKER_TTL);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The agent-facing instructions. Generic on purpose: the server and group
/// come from the environment, so a deployment names its own.
#[must_use]
pub fn message(settings: &Settings, session_id: &str) -> String {
    let server = &settings.server;
    let group = &settings.group;
    format!(
        "Before you finish: did this session produce durable knowledge (an operational fact, a fix, \
a decision, a new entity or relationship)? If nothing durable, reply 'skip' and stop. Skipping is a \
legitimate outcome, not a failure.

If yes, capture it as a Quipu episode in group {group}: POST {server}/episode. Writes require a \
bearer token: send Authorization: Bearer $QUIPU_AUTH_TOKEN from your environment, and never print \
the token. Reads such as /search stay open. Send the header X-Quipu-Client: session-capture on every \
call so this traffic is attributable. A client label and free-text source are not structured
write provenance. Prefer the installed graph-extract writer, which adds provenance automatically.
For direct HTTP, use an environment-based provenance formatter and merge X-Quipu-Agent,
X-Quipu-Harness, X-Quipu-Model, X-Quipu-Session and X-Quipu-Host into the request headers before
POSTing. Never type or guess these values. If the writer cannot derive them, keep capture pending
and record the missing fields; do not send an unattributable write. Three rules:

1. SEARCH BEFORE YOU MINT. POST {server}/search with {{\"query\":\"<your concept>\"}} and REUSE the \
existing node's exact name if one matches. Do not create a second node for a concept that already \
has one; fragmenting the graph is the most common first-use mistake, so assume you will make it.
2. EVERY node needs a \"type\" from the server's loaded vocabulary (POST {server}/shapes with \
{{\"action\":\"vocabulary\"}}, bearer required). A node without a governed type is refused, and with \
it the ENTIRE episode, including every well-formed node and edge beside it. Do not invent a type.
3. Put your SESSION ID ({session_id}) in the episode \"source\". It is the only way to tell a real \
capture from an interruption nobody acted on.

Need an entity kind that genuinely does not exist yet? Do not invent it inline: shapes are enforced \
and violating writes are refused. POST {server}/propose with the proposed shape (pending review, \
provenance-tracked, never auto-applied), and raise it with whoever owns the shapes so the accepted \
shape also lands in the shapes files; an accepted proposal that never reaches those files is lost on \
redeploy."
    )
}

#[cfg(test)]
#[path = "hook_session_capture_tests.rs"]
mod tests;

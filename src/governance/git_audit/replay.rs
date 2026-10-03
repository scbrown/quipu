//! Bounded invocation of an explicitly selected, trusted offline Yupana binary.
use super::invalid;
use crate::error::Result;
use std::path::Path;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct Response {
    schema_version: u32,
    rule: String,
    path: String,
    pub verdict: String,
    pub violations: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn run(
    executable: &Path,
    rule: &serde_json::Value,
    path: &str,
    source: &str,
) -> Result<Response> {
    use std::{
        io::{Read, Seek, SeekFrom, Write},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut input = tempfile::tempfile().map_err(|e| invalid(e.to_string()))?;
    serde_json::to_writer(
        &mut input,
        &serde_json::json!({"rule":rule,"path":path,"source":source}),
    )
    .map_err(|e| invalid(e.to_string()))?;
    input.flush().map_err(|e| invalid(e.to_string()))?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|e| invalid(e.to_string()))?;
    let mut output = tempfile::tempfile().map_err(|e| invalid(e.to_string()))?;
    let errors = tempfile::tempfile().map_err(|e| invalid(e.to_string()))?;
    let child = Command::new(executable)
        .arg("audit-rule")
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(
            output.try_clone().map_err(|e| invalid(e.to_string()))?,
        ))
        .stderr(Stdio::from(
            errors.try_clone().map_err(|e| invalid(e.to_string()))?,
        ))
        .spawn()
        .map_err(|e| invalid(format!("cannot run Yupana audit-rule: {e}")))?;
    let mut child = Reap(child);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().map_err(|e| invalid(e.to_string()))? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(10)
            || output.metadata().map_err(|e| invalid(e.to_string()))?.len() > 1024 * 1024
            || errors.metadata().map_err(|e| invalid(e.to_string()))?.len() > 1024 * 1024
        {
            let _ = child.0.kill();
            let _ = child.0.wait();
            return Err(invalid(
                "Yupana replay exceeded its 10-second/1-MiB output bound",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if errors.metadata().map_err(|e| invalid(e.to_string()))?.len() > 1024 * 1024 {
        return Err(invalid("Yupana stderr exceeds 1 MiB"));
    }
    output
        .seek(SeekFrom::Start(0))
        .map_err(|e| invalid(e.to_string()))?;
    let mut bytes = Vec::new();
    output
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| invalid(e.to_string()))?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("Yupana response exceeds 1 MiB"));
    }
    let response: Response = serde_json::from_slice(&bytes)
        .map_err(|e| invalid(format!("invalid Yupana replay response: {e}")))?;
    let expected_code = match response.verdict.as_str() {
        "satisfied" | "not_applicable"
            if response.violations.is_empty() && response.errors.is_empty() =>
        {
            0
        }
        "unsatisfied" if !response.violations.is_empty() && response.errors.is_empty() => 1,
        "unknown" if !response.errors.is_empty() && response.violations.is_empty() => 2,
        _ => return Err(invalid("inconsistent Yupana verdict")),
    };
    if response.schema_version != 1
        || rule["name"].as_str() != Some(response.rule.as_str())
        || response.path != path
        || status.code() != Some(expected_code)
    {
        return Err(invalid(
            "Yupana response does not match request, protocol, or exit status",
        ));
    }
    Ok(response)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn run(
    _executable: &Path,
    _rule: &serde_json::Value,
    _path: &str,
    _source: &str,
) -> Result<Response> {
    Err(invalid("Yupana replay requires a native process host"))
}

#[cfg(all(test, unix))]
#[path = "replay_tests.rs"]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
struct Reap(std::process::Child);
#[cfg(not(target_arch = "wasm32"))]
impl Drop for Reap {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

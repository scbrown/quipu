//! Endpoint declaration resolution. Caboodle owns all endpoint trust policy.
use super::{FederationConfig, RemoteEndpoint};
use crate::time::Deadline;
use crate::{Error, Result};

impl FederationConfig {
    /// Resolve only the exact requested endpoint. Verification and current
    /// Caboodle consent precede every use; no endpoint or pack is fetched here.
    pub(crate) fn pointer_endpoint(
        &self,
        endpoint: &str,
        deadline: Option<Deadline>,
    ) -> Result<Option<RemoteEndpoint>> {
        self.pointer_endpoint_with(endpoint, deadline, |remote, endpoint| {
            check_endpoint(remote, endpoint, deadline)
        })
    }

    fn pointer_endpoint_with(
        &self,
        endpoint: &str,
        deadline: Option<Deadline>,
        mut trust: impl FnMut(&str, &str) -> Result<()>,
    ) -> Result<Option<RemoteEndpoint>> {
        check_deadline(deadline)?;
        for pendant in &self.pendants {
            check_deadline(deadline)?;
            let context = |e| {
                Error::InvalidValue(format!(
                    "SERVICE pointer {endpoint} declared by {} in {}: {e}",
                    pendant.repo_remote,
                    pendant.path.display()
                ))
            };
            let path = pendant
                .path
                .to_str()
                .ok_or_else(|| context("non-UTF-8 local path".into()))?;
            let request =
                crate::share_transport::read_local(path).map_err(|e| context(e.to_string()))?;
            check_deadline(deadline)?;
            crate::share_import::verify_share(&request).map_err(|e| context(e.to_string()))?;
            check_deadline(deadline)?;
            for pointer in &request.manifest.pointers {
                match &pointer.target {
                    crate::share::PointerTarget::Endpoint { endpoint: declared }
                        if declared == endpoint =>
                    {
                        trust(&pendant.repo_remote, declared)
                            .map_err(|e| context(e.to_string()))?;
                        check_deadline(deadline)?;
                        // No auth token or self-declared trust label crosses this boundary.
                        return Ok(Some(RemoteEndpoint::new(&pointer.graph, declared)));
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }
}

fn check_deadline(deadline: Option<Deadline>) -> Result<()> {
    if let Some(deadline) = deadline.filter(Deadline::passed) {
        return Err(deadline.request_timeout());
    }
    Ok(())
}

fn check_endpoint(remote: &str, endpoint: &str, deadline: Option<Deadline>) -> Result<()> {
    use std::process::{Command, Stdio};
    let error = |reason: &str| {
        Error::InvalidValue(format!(
            "caboodle endpoint trust {reason}: {endpoint} declared by {remote}"
        ))
    };
    check_deadline(deadline)?;
    let child = Command::new("caboodle")
        .args(["packs", "trust", "check-endpoint", remote, endpoint])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("unavailable"))?;
    let status = wait_for_policy(child, deadline).map_err(|err| match err {
        err @ Error::RequestTimeout { .. } => err,
        _ => error("did not complete"),
    })?;
    match status.code() {
        Some(0) => Ok(()),
        Some(3) => Err(error("refused")),
        _ => Err(error("unavailable (check failed)")),
    }
}

fn wait_for_policy(
    mut child: std::process::Child,
    deadline: Option<Deadline>,
) -> Result<std::process::ExitStatus> {
    use std::time::{Duration, Instant};
    let policy_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(err) = check_deadline(deadline) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                check_deadline(deadline)?;
                return Ok(status);
            }
            Ok(None) if Instant::now() < policy_deadline => {
                let pause = Duration::from_millis(10)
                    .min(policy_deadline.saturating_duration_since(Instant::now()));
                let pause =
                    deadline.map_or(pause, |deadline| pause.min(deadline.remaining_duration()));
                std::thread::sleep(pause);
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::InvalidValue(
                    "caboodle endpoint trust did not complete".into(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests;

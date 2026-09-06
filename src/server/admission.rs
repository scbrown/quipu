//! Cancellable admission for synchronous store handlers.
//!
//! Writes have been admitted here since this module was written; reads were
//! added after aegis-raq1ok, where the missing read half wedged the server.

use super::base::{AppError, blocking};

/// Admit one write before moving it onto Tokio's uncancellable blocking pool.
///
/// A request future can be cancelled while it waits for this permit, so an
/// abandoned client never becomes another `spawn_blocking` task queued on the
/// writer mutex. The permit moves into the blocking closure and is held across
/// deferred embedding as well as the transaction: dropping the HTTP future
/// cannot release admission while its blocking work is still running.
static WRITE_ADMISSION: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub(crate) async fn write_blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    write_blocking_with(&WRITE_ADMISSION, f).await
}

pub(crate) async fn write_blocking_with<T, F>(
    admission: &'static tokio::sync::Semaphore,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    admit_blocking_with(admission, "write", f).await
}

/// The shared acquire-then-block step behind both admission paths.
///
/// Both halves must acquire in ASYNC context and hold the permit across the
/// blocking closure; writing that twice invites the two from drifting, and the
/// read half exists precisely because one half of this codebase had the
/// property and the other did not (aegis-raq1ok).
async fn admit_blocking_with<T, F>(
    admission: &'static tokio::sync::Semaphore,
    kind: &'static str,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    let permit = admission.acquire().await.map_err(|_| {
        AppError::from(quipu::Error::InvalidValue(format!(
            "{kind} admission is closed"
        )))
    })?;
    blocking(move || {
        let _permit = permit;
        f()
    })
    .await
}

/// Cancellable admission for synchronous READ handlers (aegis-raq1ok).
///
/// The write path above has been cancellable since it was written; **the read
/// path was not, and that asymmetry is what took the deployed server down for 9 minutes
/// on 2026-09-06.** Measured from the request log: 312 requests started and
/// never completed, 246 of them `/query`; the last read to complete did so at
/// `00:04:54Z` and the next at `00:13:42Z`, after a restart. Throughout,
/// `/health`, `/version` and `/set` answered 200 in ~0 ms — writes survived
/// precisely because they queue on `WRITE_ADMISSION`, where an abandoned
/// client's future is dropped BEFORE it reaches the uncancellable blocking
/// pool. Reads went straight to `blocking()` and kept their thread.
///
/// The consequence is a ratchet, not a slowdown. `spawn_blocking` work cannot
/// be cancelled, so every reader that gives up (yupana-hook abandons at ~30 s,
/// aegis-tjyhh4) leaves a thread parked on a read connection while its client
/// retries. In-flight reads rose 17 -> 292 without ever plateauing, the cgroup
/// hit `TasksMax` — `cgroup: fork rejected by pids controller`, the only such
/// line in the journal — and no read completed again until the process was
/// restarted.
///
/// Permits are sized to the read pool because a read cannot make progress
/// without a pool connection: beyond `read_pool_size`, extra admitted readers
/// are threads waiting on a `FairMutex`, which is exactly the wasted resource
/// that exhausted the pid budget. Bounding admission converts that unbounded
/// thread growth into bounded queueing on a cancellable semaphore.
static READ_ADMISSION: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();

/// Used when `init_read_admission` was never called — unit tests construct
/// handlers directly without going through `serve`. Deliberately not 1: an
/// uninitialised default that serialised every read would turn a missing
/// startup call into a silent throughput collapse rather than a loud failure.
const DEFAULT_READ_PERMITS: usize = 8;

/// Size read admission from the actual read pool. Called once, from `serve`.
pub(crate) fn init_read_admission(permits: usize) {
    let _ = READ_ADMISSION.set(tokio::sync::Semaphore::new(permits.max(1)));
}

fn read_admission() -> &'static tokio::sync::Semaphore {
    READ_ADMISSION.get_or_init(|| tokio::sync::Semaphore::new(DEFAULT_READ_PERMITS))
}

/// Admit one read before moving it onto Tokio's uncancellable blocking pool.
///
/// Mirrors [`write_blocking`]. The permit is acquired in async context, so a
/// client that disconnects while queued is cancelled here and never becomes a
/// blocking task; and it is held across the blocking closure, so dropping the
/// HTTP future cannot release admission while the read is still running.
pub(crate) async fn read_blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    admit_blocking_with(read_admission(), "read", f).await
}

/// `read_blocking` against a caller-supplied semaphore, so a test can prove the
/// cancellation property without touching the process-wide permit count.
#[cfg(test)]
pub(crate) async fn read_blocking_with<T, F>(
    admission: &'static tokio::sync::Semaphore,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    admit_blocking_with(admission, "read", f).await
}

//! Cancellable admission for synchronous write handlers.

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
    let permit = admission.acquire().await.map_err(|_| {
        AppError::from(quipu::Error::InvalidValue(
            "write admission is closed".to_string(),
        ))
    })?;
    blocking(move || {
        let _permit = permit;
        f()
    })
    .await
}

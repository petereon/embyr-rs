//! Async wrappers that run Argon2id on the blocking pool.
//!
//! Each Argon2id computation is ~64 MiB and CPU-bound, and
//! `embyr_core::auth::argon2` gates concurrency with a blocking `Condvar`.
//! Calling it directly from an async handler stalls a tokio worker — a handful
//! of concurrent bad-key requests could freeze every listener, `/healthz`
//! included. Async code must go through these wrappers instead.

use embyr_core::{auth::argon2, error::CoreError};

async fn run<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> Result<T, CoreError> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(CoreError::InvalidArgument(format!("argon2 task failed: {e}"))))
}

pub async fn verify_api_key(api_key: &[u8], phc_hash: &str) -> Result<bool, CoreError> {
    let (key, hash) = (api_key.to_vec(), phc_hash.to_owned());
    run(move || argon2::verify_api_key(&key, &hash)).await
}

/// ADR-085: current key, else the still-live previous key (see `argon2::verify_project_key`).
pub async fn verify_project_key(
    api_key: &[u8],
    current: &str,
    previous_live: Option<&str>,
) -> Result<bool, CoreError> {
    let (key, cur, prev) = (api_key.to_vec(), current.to_owned(), previous_live.map(str::to_owned));
    run(move || Ok(argon2::verify_project_key(&key, &cur, prev.as_deref()))).await
}

pub async fn verify_password(password: &[u8], phc_hash: &str) -> Result<bool, CoreError> {
    let (pw, hash) = (password.to_vec(), phc_hash.to_owned());
    run(move || argon2::verify_password(&pw, &hash)).await
}

pub async fn hash_password(password: &[u8]) -> Result<String, CoreError> {
    let pw = password.to_vec();
    run(move || argon2::hash_password(&pw)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a current-thread runtime, an inline Argon2 call would block the only
    /// worker; the wrapper must leave it free to run other tasks meanwhile.
    #[tokio::test]
    async fn verify_runs_off_the_async_worker() {
        let hash = hash_password(b"pw").await.unwrap();
        let ticked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let t = ticked.clone();
        let ticker = tokio::spawn(async move { t.store(true, std::sync::atomic::Ordering::SeqCst) });
        assert!(verify_password(b"pw", &hash).await.unwrap());
        assert!(!verify_api_key(b"nope", &hash).await.unwrap());
        ticker.await.unwrap();
        assert!(ticked.load(std::sync::atomic::Ordering::SeqCst));
    }
}

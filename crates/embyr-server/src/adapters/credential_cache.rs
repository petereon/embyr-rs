use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use tokio::sync::Mutex;

use embyr_core::domain::project::CredentialCacheKey;
use embyr_core::storage::backend_adapter::BackendAdapter;

pub type SharedBackendAdapter = Arc<dyn BackendAdapter + Send + Sync>;

pub struct CachedEntry {
    pub adapter: SharedBackendAdapter,
    pub project_status: String,
}

/// LRU credential cache — maps (project_id, BLAKE3(api_key)) → backend adapter.
pub struct CredentialCache {
    inner: Mutex<LruCache<CredentialCacheKey, CachedEntry>>,
}

impl CredentialCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity must be > 0"),
            )),
        }
    }

    /// Returns `(SharedBackendAdapter, project_status)` if present, promoting it in LRU order.
    pub async fn get(
        &self,
        key: &CredentialCacheKey,
    ) -> Option<(SharedBackendAdapter, String)> {
        let mut guard = self.inner.lock().await;
        guard
            .get(key)
            .map(|e| (Arc::clone(&e.adapter), e.project_status.clone()))
    }

    /// Insert or replace the entry for `key`.
    pub async fn insert(&self, key: CredentialCacheKey, entry: CachedEntry) {
        self.inner.lock().await.put(key, entry);
    }
}

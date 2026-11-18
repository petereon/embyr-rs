use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use tokio::sync::Mutex;

use embyr_core::domain::project::CredentialCacheKey;
use embyr_core::storage::backend_adapter::BackendAdapter;

use crate::adapters::postgres_backend::PostgresBackendAdapter;
use crate::middleware::rate_limit::TokenBucket;

pub type SharedBackendAdapter = Arc<dyn BackendAdapter + Send + Sync>;

/// How long a cached credential is served without re-reading the project row.
///
/// Bounds how long ANY replica keeps serving a project that another replica
/// (or a sweeper) suspended, deleted, re-keyed or re-pointed at a new DSN —
/// `evict_project` only reaches the process that made the change.
// ponytail: fixed 60s TTL — every active key pays one project-row read +
// Argon2id verify + tenant pool rebuild per minute per replica, and a status
// change takes up to 60s to reach other replicas. Upgrade path: Postgres
// LISTEN/NOTIFY invalidation fan-out, then raise or drop the TTL.
pub const CREDENTIAL_TTL: Duration = Duration::from_secs(60);

/// Failed-authentication budget per project, per process: burst size and
/// refill (attempts per second). Only keys this process has never verified
/// draw on it; a successful verification hands its token back.
// ponytail: in-process and per-project, not per-client — N replicas allow N×
// this, and while a bad-key flood holds the budget at zero a key this replica
// has never seen is refused too (already-verified keys are not). Upgrade
// path: key the budget by (project, client address).
const UNVERIFIED_KEY_BURST: f64 = 10.0;
const UNVERIFIED_KEY_REFILL_PER_SEC: f64 = 1.0;

pub struct CachedEntry {
    pub adapter: SharedBackendAdapter,
    pub project_status: String,
    /// Decrypted customer DSN — stored so the Listen handler can start
    /// `PostgresNotifyListener` without re-decrypting on each stream open.
    pub dsn: String,
    /// The same adapter as `adapter`, as its concrete type, when the backend
    /// is Postgres (`None` for agent mode). REST hosted-identity handlers run
    /// raw SQL through its `pool()`; sharing this one `Arc` means they reuse
    /// the gRPC path's tenant pool (same size / acquire-timeout, same TTL,
    /// same eviction) instead of dialing the customer DB on every request.
    pub pg: Option<Arc<PostgresBackendAdapter>>,
}

struct Inner<V> {
    /// Value plus the instant it was verified.
    entries: LruCache<CredentialCacheKey, (Instant, V)>,
    /// One bucket per REAL project (callers only ask after the project row
    /// was found), so this map is bounded by the number of projects.
    unverified_key_budget: HashMap<String, TokenBucket>,
}

/// LRU credential cache — maps (project_id, BLAKE3(api_key)) → backend adapter.
///
/// Generic over the cached value only so the TTL / budget rules are unit
/// testable without a real backend adapter; production uses the default.
pub struct CredentialCache<V = CachedEntry> {
    inner: Mutex<Inner<V>>,
}

impl<V> CredentialCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: LruCache::new(NonZeroUsize::new(capacity).expect("capacity must be > 0")),
                unverified_key_budget: HashMap::new(),
            }),
        }
    }

    /// The entry for `key` if it was verified less than `CREDENTIAL_TTL`
    /// before `now`. An expired entry is left in place (see
    /// `admit_verification`) but never served.
    async fn fresh_at<T>(
        &self,
        key: &CredentialCacheKey,
        now: Instant,
        read: impl FnOnce(&V) -> T,
    ) -> Option<T> {
        let mut guard = self.inner.lock().await;
        let (verified_at, _) = guard.entries.peek(key)?;
        if now.saturating_duration_since(*verified_at) >= CREDENTIAL_TTL {
            return None;
        }
        guard.entries.get(key).map(|(_, value)| read(value))
    }

    /// Read from the fresh entry for `key`, or `build` one on a miss/expiry,
    /// insert it and read from that. `build` runs only when nothing fresh is
    /// cached, so N requests inside one TTL build the entry (and the tenant
    /// pool inside it) once. Concurrent misses may each build; the last
    /// insert wins and the losers' pools drop with their last `Arc`.
    pub async fn get_or_try_build<T, E, Fut>(
        &self,
        key: &CredentialCacheKey,
        read: impl Fn(&V) -> T,
        build: impl FnOnce() -> Fut,
    ) -> Result<T, E>
    where
        Fut: std::future::Future<Output = Result<V, E>>,
    {
        self.get_or_try_build_at(key, Instant::now(), read, build).await
    }

    async fn get_or_try_build_at<T, E, Fut>(
        &self,
        key: &CredentialCacheKey,
        now: Instant,
        read: impl Fn(&V) -> T,
        build: impl FnOnce() -> Fut,
    ) -> Result<T, E>
    where
        Fut: std::future::Future<Output = Result<V, E>>,
    {
        if let Some(hit) = self.fresh_at(key, now, &read).await {
            return Ok(hit);
        }
        let entry = build().await?;
        let out = read(&entry);
        self.insert(key.clone(), entry).await;
        Ok(out)
    }

    /// Insert or replace the entry for `key`, restarting its TTL.
    ///
    /// Callers insert only after a successful key verification, so this also
    /// returns the token `admit_verification` may have taken for it.
    pub async fn insert(&self, key: CredentialCacheKey, entry: V) {
        let mut guard = self.inner.lock().await;
        if let Some(bucket) = guard.unverified_key_budget.get_mut(key.project_id.as_str()) {
            bucket.tokens = (bucket.tokens + 1.0).min(bucket.capacity);
        }
        guard.entries.put(key, (Instant::now(), entry));
    }

    /// May the caller spend an Argon2id verification on `key`?
    ///
    /// A key that still has an (expired) entry was verified by this process
    /// before, so it is always admitted — once: the entry is consumed, and a
    /// failed re-verification leaves the key unknown. Any other key draws on
    /// the project's failed-authentication budget, which is what bounds a
    /// bad-key flood now that the project's own rate-limit bucket is only
    /// charged after authentication succeeds.
    pub async fn admit_verification(&self, key: &CredentialCacheKey) -> bool {
        let mut guard = self.inner.lock().await;
        if guard.entries.pop(key).is_some() {
            return true;
        }
        guard
            .unverified_key_budget
            .entry(key.project_id.as_str().to_owned())
            .or_insert_with(|| TokenBucket::new(UNVERIFIED_KEY_BURST, UNVERIFIED_KEY_REFILL_PER_SEC))
            .try_consume()
    }

    /// Remove all cache entries for a given project_id.
    ///
    /// Called when project status changes (suspend/activate/delete) so the next
    /// SDK call performs a DB lookup and observes the new status immediately.
    pub async fn evict_project(&self, project_id: &str) {
        let mut guard = self.inner.lock().await;
        let keys_to_remove: Vec<CredentialCacheKey> = guard
            .entries
            .iter()
            .filter(|(k, _)| k.project_id.as_str() == project_id)
            .map(|(k, _)| k.clone())
            .collect();
        for k in keys_to_remove {
            guard.entries.pop(&k);
        }
    }
}

impl CredentialCache {
    /// Returns `(SharedBackendAdapter, project_status, dsn)` if present and
    /// not older than `CREDENTIAL_TTL`, promoting it in LRU order.
    pub async fn get(
        &self,
        key: &CredentialCacheKey,
    ) -> Option<(SharedBackendAdapter, String, String)> {
        self.fresh_at(key, Instant::now(), |e| {
            (Arc::clone(&e.adapter), e.project_status.clone(), e.dsn.clone())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embyr_core::domain::project::ProjectId;

    type TestCache = CredentialCache<&'static str>;

    fn key(project: &str, api_key: &str) -> CredentialCacheKey {
        CredentialCacheKey {
            project_id: ProjectId::new(project).expect("valid project id"),
            api_key_blake3: embyr_core::auth::blake3::derive_cache_key(api_key.as_bytes()),
        }
    }

    async fn served(cache: &TestCache, k: &CredentialCacheKey, age: Duration) -> Option<&'static str> {
        cache.fresh_at(k, Instant::now() + age, |v| *v).await
    }

    #[tokio::test]
    async fn entry_is_built_once_per_ttl_window_and_rebuilt_after_eviction_or_expiry() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = TestCache::new(8);
        let k = key("acme", "good-key");
        let builds = AtomicUsize::new(0);
        let get = |now: Instant| {
            cache.get_or_try_build_at::<_, (), _>(&k, now, |v| *v, || async {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok("active")
            })
        };

        for _ in 0..5 {
            assert_eq!(get(Instant::now()).await, Ok("active"));
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1, "5 requests, one build");

        cache.evict_project("acme").await;
        assert_eq!(get(Instant::now()).await, Ok("active"));
        assert_eq!(builds.load(Ordering::SeqCst), 2, "rebuilt after eviction");

        assert_eq!(get(Instant::now() + CREDENTIAL_TTL).await, Ok("active"));
        assert_eq!(builds.load(Ordering::SeqCst), 3, "rebuilt after TTL");
    }

    #[tokio::test]
    async fn failed_build_caches_nothing() {
        let cache = TestCache::new(8);
        let k = key("acme", "good-key");
        let r = cache.get_or_try_build(&k, |v| *v, || async { Err::<&str, _>("boom") }).await;
        assert_eq!(r, Err("boom"));
        assert_eq!(served(&cache, &k, Duration::ZERO).await, None);
    }

    /// Bad keys admitted for `project` before the budget refuses one.
    async fn admitted_bad_keys(cache: &TestCache, project: &str) -> usize {
        let mut admitted = 0;
        while admitted < 1000
            && cache.admit_verification(&key(project, &format!("bad-{admitted}"))).await
        {
            admitted += 1;
        }
        admitted
    }

    #[tokio::test]
    async fn entry_is_served_until_the_ttl_elapses_and_never_after() {
        let cache = TestCache::new(8);
        let k = key("acme", "good-key");
        cache.insert(k.clone(), "active").await;

        assert_eq!(served(&cache, &k, Duration::ZERO).await, Some("active"));
        assert_eq!(served(&cache, &k, CREDENTIAL_TTL - Duration::from_secs(1)).await, Some("active"));
        assert_eq!(served(&cache, &k, CREDENTIAL_TTL).await, None);
        assert_eq!(served(&cache, &k, CREDENTIAL_TTL * 10).await, None);
    }

    #[tokio::test]
    async fn bad_key_flood_is_bounded_per_project_without_touching_other_projects() {
        let cache = TestCache::new(8);

        assert_eq!(admitted_bad_keys(&cache, "acme").await, UNVERIFIED_KEY_BURST as usize);
        assert!(
            cache.admit_verification(&key("globex", "first-attempt")).await,
            "one project's exhausted budget must not refuse another project"
        );
    }

    #[tokio::test]
    async fn previously_verified_key_is_readmitted_once_even_when_the_budget_is_exhausted() {
        let cache = TestCache::new(8);
        let good = key("acme", "good-key");
        cache.insert(good.clone(), "active").await;
        admitted_bad_keys(&cache, "acme").await;

        assert!(cache.admit_verification(&good).await, "expired-but-known key must re-verify");
        assert!(
            !cache.admit_verification(&good).await,
            "a key whose re-verification did not succeed is unknown again"
        );
    }

    #[tokio::test]
    async fn successful_verification_returns_its_token_to_the_budget() {
        let cache = TestCache::new(8);
        admitted_bad_keys(&cache, "acme").await;
        assert!(!cache.admit_verification(&key("acme", "new-good-key")).await);

        cache.insert(key("acme", "other-good-key"), "active").await;

        assert!(cache.admit_verification(&key("acme", "new-good-key")).await);
    }
}

//! `CapStatusCache` — in-process, per-instance cache of the latest computed
//! `CapStatus` per account (ADR-020). Mirrors `CredentialCache`'s
//! `Arc<RwLock<HashMap<K, V>>>` shape (pattern reuse, not literal reuse — a
//! different key/value type).
//!
//! Not RED-scaffolded: pure in-process cache mechanics, no business logic to
//! TDD (mirrors `CredentialCache`, which is likewise not a scaffold target in
//! its own feature history). Written by `CapUsageRefresher`; read by
//! `billing_subscription::get_subscription`. A cache miss (never-refreshed
//! account) is represented by `None` and MUST be treated as "omit
//! `cap_status`" by callers (AC-206-04, fail-open) — never fabricated as
//! exceeded.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use embyr_core::admin::CapStatus;

pub struct CapStatusCache {
    inner: RwLock<HashMap<Uuid, CapStatus>>,
}

impl CapStatusCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the last-computed `CapStatus` for `account_id`, or `None` if
    /// this account has never been refreshed (fail-open — AC-206-04).
    pub async fn get(&self, account_id: Uuid) -> Option<CapStatus> {
        self.inner.read().await.get(&account_id).cloned()
    }

    /// Insert or replace the cached `CapStatus` for `account_id`. Called by
    /// `CapUsageRefresher` once per refresh cycle per Free-plan account.
    pub async fn set(&self, account_id: Uuid, status: CapStatus) {
        self.inner.write().await.insert(account_id, status);
    }

    /// Remove the cached entry for `account_id` (e.g. on upgrade to Pro,
    /// which makes `cap_status` no longer applicable, AC-206-03).
    pub async fn evict(&self, account_id: Uuid) {
        self.inner.write().await.remove(&account_id);
    }
}

impl Default for CapStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared handle type, mirrors `SharedBackendAdapter`'s naming convention in
/// `credential_cache.rs`.
pub type SharedCapStatusCache = Arc<CapStatusCache>;

//! Background interval tasks (Tokio-interval, advisory-lock-guarded where the
//! task must not run redundantly across instances).
//!
//! `CapUsageRefresher` (ADR-020) was the first module here. `TransactionSweeper`
//! (customer-db-transaction-sweeper, ADR-054) is the second, and
//! `SoftDeletePurgeSweeper` (soft-delete-purge-sweeper, ADR-073) is the
//! third — all three share the identical interval-loop +
//! `pg_try_advisory_lock`/`pg_advisory_unlock` shape, so
//! [`advisory_lock_key`] is extracted here rather than duplicated per module.

pub mod cap_usage_refresher;
pub mod signin_rate_limit_sweeper;
pub mod soft_delete_purge_sweeper;
pub mod transaction_sweeper;

/// FNV-1a hash of `s`, used as the `pg_try_advisory_lock` key by every
/// sweeper in this module (ADR-020, ADR-054 § D5). Computed once, at compile
/// time would be ideal but FNV-1a-64 has no const-fn stdlib implementation
/// available here — computed inline via the same algorithm every cycle
/// (cheap, no allocation, negligible cost relative to the DB round-trip it
/// guards).
pub fn advisory_lock_key(s: &str) -> i64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_lock_key_is_deterministic() {
        assert_eq!(
            advisory_lock_key("embyr_cap_check"),
            advisory_lock_key("embyr_cap_check")
        );
    }

    #[test]
    fn advisory_lock_key_differs_for_different_input() {
        assert_ne!(
            advisory_lock_key("embyr_cap_check"),
            advisory_lock_key("embyr_transaction_sweep")
        );
    }
}

//! Account-deletion retention math (pure, no IO).

use chrono::{DateTime, Duration, Utc};

/// Moment after which a soft-deleted account becomes eligible for purge.
pub fn purge_after(deleted_at: DateTime<Utc>, grace_days: i64) -> DateTime<Utc> {
    deleted_at + Duration::days(grace_days)
}

/// `deleted_at` cutoff: accounts deleted strictly before it are past grace at `now`.
pub fn purge_cutoff(now: DateTime<Utc>, grace_days: i64) -> DateTime<Utc> {
    now - Duration::days(grace_days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn grace_window_boundaries() {
        let deleted_at = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let purge_at = purge_after(deleted_at, 7);
        assert_eq!(purge_at, Utc.with_ymd_and_hms(2026, 1, 8, 12, 0, 0).unwrap());
        // exactly at purge_after the account is not yet strictly before the cutoff
        assert!(!(deleted_at < purge_cutoff(purge_at, 7)));
        assert!(deleted_at < purge_cutoff(purge_at + Duration::seconds(1), 7));
    }
}

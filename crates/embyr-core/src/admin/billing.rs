//! Billing domain types — subscriptions, usage-vs-cap status (BC-1 Tenant Management).
//!
//! Zero IO imports (enforced by workspace `deny.toml`). `compute_cap_status`
//! and `cap_exceeded` are the business rules driving AC-206-01/02/05 and
//! AC-207-01/02 respectively — both implemented via Outside-In TDD (card-
//! payments-backend, DELIVER).

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Subscription plan / status (D-4, D-6, D-12)
// ---------------------------------------------------------------------------

/// D-4: exactly two tiers. No third value is ever valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionPlan {
    Free,
    Pro,
}

impl SubscriptionPlan {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubscriptionPlan::Free => "free",
            SubscriptionPlan::Pro => "pro",
        }
    }

    /// Parses the `plan` column / request-body value. Returns `None` for any
    /// value other than `"free"`/`"pro"` (AC-202-05: unrecognized plan → 422).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "free" => Some(SubscriptionPlan::Free),
            "pro" => Some(SubscriptionPlan::Pro),
            _ => None,
        }
    }
}

/// D-6/D-12: four subscription statuses. `FreeCapExceeded` and `PastDue` are
/// the two suspension-driving statuses (US-207 and US-204 respectively) —
/// both resolve through the identical `set_project_status` code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Active,
    PastDue,
    FreeCapExceeded,
    Canceled,
}

impl SubscriptionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubscriptionStatus::Active => "active",
            SubscriptionStatus::PastDue => "past_due",
            SubscriptionStatus::FreeCapExceeded => "free_cap_exceeded",
            SubscriptionStatus::Canceled => "canceled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(SubscriptionStatus::Active),
            "past_due" => Some(SubscriptionStatus::PastDue),
            "free_cap_exceeded" => Some(SubscriptionStatus::FreeCapExceeded),
            "canceled" => Some(SubscriptionStatus::Canceled),
            _ => None,
        }
    }
}

/// A local read-cache of an account's Stripe subscription state (source of
/// truth is Stripe; this row is kept in sync by the webhook handler, US-203).
#[derive(Debug, Clone)]
pub struct Subscription {
    pub account_id: uuid::Uuid,
    pub plan: SubscriptionPlan,
    pub status: SubscriptionStatus,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    /// Pro-plan-display-only (ADR-020 § Billing Cycle Boundary). Never
    /// consulted for cap computation — Free-plan cap cycles are UTC calendar
    /// months, sourced independently of these fields.
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
}

// ---------------------------------------------------------------------------
// Usage dimensions + cumulative cap status (D-7, D-9, ADR-020)
// ---------------------------------------------------------------------------

/// D-7: the four billable usage dimensions. `Storage` has no real data source
/// in this feature's V1 (OQ-CP-1, ADR-020 § Storage Dimension Gap) — callers
/// must not assume `Storage` entries in a `CapStatus` reflect real usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageDimension {
    Reads,
    Writes,
    Deletes,
    Storage,
}

impl UsageDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            UsageDimension::Reads => "reads",
            UsageDimension::Writes => "writes",
            UsageDimension::Deletes => "deletes",
            UsageDimension::Storage => "storage",
        }
    }
}

/// Illustrative Free-plan cap allowances (DISCUSS § Out of Scope: "exact price
/// points and included allowances per dimension" are placeholders pending a
/// business/market decision — matches the frontend `card-payments` feature's
/// own precedent). `Storage` has no real data source (OQ-CP-1) and is
/// excluded from this map — `compute_cap_status` must never fabricate a
/// storage cap entry from it.
pub fn free_plan_caps() -> HashMap<UsageDimension, u64> {
    let mut caps = HashMap::new();
    caps.insert(UsageDimension::Reads, 2_000_000);
    caps.insert(UsageDimension::Writes, 500_000);
    caps.insert(UsageDimension::Deletes, 100_000);
    caps
}

/// One dimension's cumulative-usage-vs-cap entry.
///
/// `pct` is boundary-inclusive: `used == cap` reports exactly `100`, never
/// `99` or `101` (AC-206-02).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionCapEntry {
    pub dimension: UsageDimension,
    pub used: u64,
    pub cap: u64,
    /// Percentage, 0-100+ (usage may exceed 100% before enforcement catches
    /// up — see ADR-020 § Consequences, bounded staleness window).
    pub pct: u64,
}

/// Per-account, per-dimension cumulative usage against the Free-plan cap,
/// summed across ALL of the account's projects (AC-206-01 — keyed by
/// `account_id`, explicitly NOT `project_id`, unlike `rate_buckets`).
///
/// `None` (rather than an empty `Vec`) distinguishes "not computed / stale /
/// unavailable" (AC-206-04: fail-open, never fabricate an over-cap reading)
/// from "computed and empty" (which cannot occur in practice — Free accounts
/// always have all three real dimensions present after `free_plan_caps()`,
/// but the type keeps the state representable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapStatus {
    pub account_id: uuid::Uuid,
    pub entries: Vec<DimensionCapEntry>,
}

// ---------------------------------------------------------------------------
// Business logic — cumulative cap-status computation (US-206/US-207)
// ---------------------------------------------------------------------------

/// Computes the per-dimension cumulative-usage-vs-cap entries for a Free-plan
/// account, given this cycle's summed usage across all of the account's
/// projects.
///
/// Contract (AC-206-01/02/03/05):
///   - Only meaningful for Free-plan accounts (callers must not invoke this
///     for Pro-plan accounts — Pro never receives `cap_status`, D-6/D-7).
///   - `pct` is boundary-inclusive: `used == cap` → `pct == 100`.
///   - Never includes a `Storage` entry (OQ-CP-1 — no real data source).
///   - Pure function: the cycle-boundary reset (AC-206-05) is the caller's
///     responsibility (usage is pre-filtered to the current UTC calendar
///     month before this function is invoked, per ADR-020 § Billing Cycle
///     Boundary) — this function has no notion of time.
///
pub fn compute_cap_status(
    account_id: uuid::Uuid,
    usage_this_cycle: &HashMap<UsageDimension, u64>,
) -> CapStatus {
    let entries = free_plan_caps()
        .into_iter()
        .map(|(dimension, cap)| {
            let used = usage_this_cycle.get(&dimension).copied().unwrap_or(0);
            // Integer arithmetic (AC-206-02): used == cap must yield exactly
            // 100, never 99 (float rounding down) or 101 (rounding up).
            let pct = (used * 100) / cap;
            DimensionCapEntry {
                dimension,
                used,
                cap,
                pct,
            }
        })
        .collect();
    CapStatus {
        account_id,
        entries,
    }
}

/// Returns `true` when ANY dimension in `status` has crossed `>= 100%` —
/// the trigger condition US-207's enforcement action reads (AC-207-01/02).
///
/// Boundary-inclusive (reuses `compute_cap_status`'s `pct` — `used == cap`
/// yields `pct == 100`, which counts as exceeded here, AC-207-01/02).
pub fn cap_exceeded(status: &CapStatus) -> bool {
    status.entries.iter().any(|entry| entry.pct >= 100)
}

// ---------------------------------------------------------------------------
// Webhook event classification (US-203)
// ---------------------------------------------------------------------------

/// How a webhook handler applied (or didn't apply) a single event —
/// classifies the outcome for logging/observability (KPI #2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookEventOutcome {
    /// Event was new and its state transition was applied.
    Applied,
    /// Event's `event.id` was already present in `processed_webhook_events` —
    /// no-op (AC-203-03).
    Duplicate,
    /// Event type is not one this feature handles — logged, 200, no state
    /// change (AC-203-06, forward-compatible).
    Ignored,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_status_round_trips_through_as_str_and_parse() {
        for status in [
            SubscriptionStatus::Active,
            SubscriptionStatus::PastDue,
            SubscriptionStatus::FreeCapExceeded,
            SubscriptionStatus::Canceled,
        ] {
            assert_eq!(SubscriptionStatus::parse(status.as_str()), Some(status));
        }
    }

    #[test]
    fn subscription_status_parse_rejects_unknown_value() {
        assert_eq!(SubscriptionStatus::parse("bogus"), None);
    }

    fn cap_status_with_pct(pct: u64) -> CapStatus {
        CapStatus {
            account_id: uuid::Uuid::nil(),
            entries: vec![DimensionCapEntry {
                dimension: UsageDimension::Reads,
                used: pct,
                cap: 100,
                pct,
            }],
        }
    }

    #[test]
    fn cap_exceeded_is_false_below_100_percent() {
        assert!(!cap_exceeded(&cap_status_with_pct(99)));
    }

    #[test]
    fn cap_exceeded_is_true_at_exactly_100_percent() {
        assert!(cap_exceeded(&cap_status_with_pct(100)));
    }

    #[test]
    fn cap_exceeded_is_true_above_100_percent() {
        assert!(cap_exceeded(&cap_status_with_pct(150)));
    }

    #[test]
    fn compute_cap_status_reports_100_pct_when_used_equals_cap() {
        let account_id = uuid::Uuid::nil();
        let mut usage = HashMap::new();
        usage.insert(UsageDimension::Reads, 2_000_000);

        let status = compute_cap_status(account_id, &usage);

        let reads_entry = status
            .entries
            .iter()
            .find(|e| e.dimension == UsageDimension::Reads)
            .expect("Reads entry present");
        assert_eq!(reads_entry.pct, 100);
    }

    #[test]
    fn compute_cap_status_never_includes_storage_dimension() {
        let status = compute_cap_status(uuid::Uuid::nil(), &HashMap::new());
        assert!(!status
            .entries
            .iter()
            .any(|e| e.dimension == UsageDimension::Storage));
    }
}

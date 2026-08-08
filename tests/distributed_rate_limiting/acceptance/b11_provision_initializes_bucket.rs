// @driving_port @real-io @adapter-integration @US-11
//! US-11 — Provisioning creates rate_buckets row atomically.
//!
//! Acceptance criteria verified here:
//!   AC-11-01: POST /admin/v1/projects creates a rate_buckets row atomically with the projects row.
//!   AC-11-02: If rate_buckets INSERT fails, the projects INSERT is rolled back (no orphaned row).
//!
//! Driving port: admin HTTP :9090 (reqwest → POST /admin/v1/projects).
//! Assertion level: real Postgres container (testcontainers-rs).
//!
//! All tests are #[ignore] — DELIVER unskips them as part of DRL-05.
//!
//! Implementation note for DELIVER (DRL-05):
//!   - `provision_project` handler must wrap both INSERTs in a `sqlx::Transaction`.
//!   - `OperatorState` must expose `rate_limit_capacity: f64`.
//!   - To test AC-11-02 atomicity, the test triggers an FK violation on rate_buckets INSERT.
//!
//! Scaffold classification target: RED — todo! panics; both tests compile.

#[path = "../common/mod.rs"]
mod common;
use common::DrlTestContext;

// ─── AC-11-01: provisioning creates rate_bucket row ──────────────────────────

/// Provisioning a project creates a rate_buckets row in the same transaction.
///
/// AC-11-01
///
/// Journey:
///   Given: empty system DB (no projects, no rate_buckets)
///   When:  operator provisions a new project via POST /admin/v1/projects {name, backend_mode}
///   Then:  a rate_buckets row exists for the new project_id
///   And:   the row's initial token count equals the configured EMBYR_RATE_LIMIT_RPS capacity
///
/// @driving_port @real-io @US-11 @AC-11-01
#[tokio::test]
#[ignore]
async fn provision_creates_rate_bucket_row() {
    // SCAFFOLD: true — DRL-05 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx.with_admin_url(...) — start admin server with real DI, call start_test_server_admin()
    //   2. POST /admin/v1/projects {name: "test-project", backend_mode: "direct_pg"}
    //   3. Extract project_id from response body
    //   4. Assert ctx.rate_bucket_row_count(&project_id) == 1
    //   5. Assert bucket.tokens == ctx.rate_limit_rps (configured capacity)
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-05 DELIVER: start admin server, POST /admin/v1/projects, assert rate_buckets row created atomically"
    )
}

// ─── AC-11-02: provision atomicity — rate_buckets failure rolls back project ─

/// A failed rate_buckets INSERT during provisioning rolls back the projects row.
///
/// AC-11-02
///
/// Journey (chained — Given reuses state from AC-11-01):
///   Given: system DB with one successfully provisioned project (rate_buckets row present)
///   When:  operator attempts to provision a project with an ID that already has a rate_buckets row
///          (FK primary key conflict on rate_buckets — rate_buckets INSERT would fail)
///   Then:  the second provision request returns an error (conflict)
///   And:   the projects table has no orphaned row for the conflicting ID
///   And:   the rate_buckets table still has exactly one row (no duplicate, no orphan)
///
/// @driving_port @real-io @US-11 @AC-11-02
#[tokio::test]
#[ignore]
async fn provision_rate_bucket_is_transactional() {
    // SCAFFOLD: true — DRL-05 DELIVER implements this.
    // DELIVER steps:
    //   1. Seed system DB: one valid project with rate_buckets row via ctx.insert_project_with_bucket()
    //   2. Attempt second provision: POST /admin/v1/projects with same project_id (FK conflict)
    //   3. Assert HTTP 409 or 500 from admin server
    //   4. Assert ctx.project_row_count(conflicting_id) == 1 (original only, no orphan)
    //   5. Assert ctx.rate_bucket_row_count(conflicting_id) == 1 (unchanged)
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-05 DELIVER: assert rate_buckets INSERT failure rolls back projects INSERT — no orphaned row"
    )
}

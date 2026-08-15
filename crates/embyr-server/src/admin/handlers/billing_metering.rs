//! `run_metering` — `POST /admin/v1/billing/run-metering` (US-205).
//!
//! Not a Tokio-interval background task (CPB-AD-09 — DISCUSS Slice 05
//! explicitly descopes scheduling infrastructure to DEVOPS-wave); this is a
//! plain async function invoked synchronously by the HTTP route, so an
//! external scheduler (k8s CronJob or similar) can trigger it. Deliberate
//! asymmetry with `CapUsageRefresher`, not an inconsistency.

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use crate::admin::state::OperatorState;

/// Per-run summary returned to the operator (illustrative shape — the exact
/// audit-log surface, `GET /admin/v1/billing/metering-log`, is OQ-CP-4,
/// explicitly not built in this feature).
#[derive(Debug, Serialize)]
pub struct MeteringRunSummary {
    pub projects_processed: u32,
    pub usage_records_pushed: u32,
    pub projects_failed: u32,
}

/// A dimension name paired with its accessor into a `DailyMetricsRow`.
type DimensionAccessor = (&'static str, fn(&DailyMetricsRow) -> i64);

/// One non-zero (dimension, quantity) pair for a project's prior-day usage.
const DIMENSIONS: [DimensionAccessor; 3] = [
    ("reads", |row| row.read_ops),
    ("writes", |row| row.write_ops),
    ("deletes", |row| row.delete_ops),
];

struct DailyMetricsRow {
    project_id: String,
    read_ops: i64,
    write_ops: i64,
    delete_ops: i64,
}

/// `POST /admin/v1/billing/run-metering` — operator-authed only (Bearer
/// `EMBYR_ADMIN_KEY`, mirrors `operator_router` — no in-handler auth check
/// needed, `operator_auth_middleware` already gates this sub-router,
/// AC-205-05). Reads yesterday's `daily_project_metrics` rows and pushes one
/// Stripe Usage Record per project per non-zero dimension (AC-205-01/02).
/// Idempotent via Stripe's native idempotency-key parameter, keyed
/// `{stripe_customer_id}:{project_id}:{dimension}:{date}` (AC-205-03; see
/// `push_project_usage` below for why the customer id is included). Per-project failure
/// isolation: one project's Stripe API failure does not abort the rest of
/// the run (AC-205-04) — each project's pushes are wrapped in independent
/// `Result` handling; a `StripeError` for one project increments
/// `projects_failed` and the loop continues.
pub async fn run_metering(
    State(state): State<OperatorState>,
) -> Result<Json<MeteringRunSummary>, StatusCode> {
    let pool = state.system_db.pool();

    // Yesterday's per-project counters, joined to the owning account's
    // Stripe customer id — usage records attach to the Stripe customer
    // (feature-delta.md), not a subscription item (see stripe_gateway.rs's
    // push_usage_record doc comment for why).
    let rows: Vec<(String, i64, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT m.project_id, m.read_ops, m.write_ops, m.delete_ops, a.stripe_customer_id \
         FROM daily_project_metrics m \
         JOIN projects p ON p.id = m.project_id \
         JOIN accounts a ON a.id = p.account_id \
         WHERE m.date = CURRENT_DATE - INTERVAL '1 day'",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("run_metering: failed to read daily_project_metrics: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let today = chrono::Utc::now().date_naive();
    let mut projects_processed = 0u32;
    let mut projects_failed = 0u32;
    let mut usage_records_pushed = 0u32;

    for (project_id, read_ops, write_ops, delete_ops, stripe_customer_id) in rows {
        let Some(stripe_customer_id) = stripe_customer_id else {
            // No Stripe customer provisioned for this project's account yet
            // (never called GET /billing/subscription) — nothing to attach
            // usage to. Reported as a failed project, not silently dropped.
            projects_failed += 1;
            continue;
        };

        let row = DailyMetricsRow { project_id: project_id.clone(), read_ops, write_ops, delete_ops };
        match push_project_usage(&state, &row, &stripe_customer_id, today).await {
            Ok(pushed) => {
                projects_processed += 1;
                usage_records_pushed += pushed;
            }
            Err(e) => {
                tracing::error!(
                    "run_metering: project {project_id} usage push failed, isolated from the rest of the run: {e}"
                );
                projects_failed += 1;
            }
        }
    }

    Ok(Json(MeteringRunSummary {
        projects_processed,
        usage_records_pushed,
        projects_failed,
    }))
}

/// Push every non-zero dimension for one project (AC-205-02: zero-usage
/// dimensions are never pushed). Returns the count of dimensions Stripe
/// genuinely recorded as new (excludes idempotent replays, AC-205-03). Any
/// single dimension's `StripeError` aborts the REST of this project's own
/// pushes and propagates to the caller — the per-PROJECT isolation
/// (AC-205-04) happens one level up in `run_metering`'s loop.
async fn push_project_usage(
    state: &OperatorState,
    row: &DailyMetricsRow,
    stripe_customer_id: &str,
    today: chrono::NaiveDate,
) -> Result<u32, crate::adapters::stripe_gateway::StripeError> {
    let yesterday = today - chrono::Duration::days(1);
    let mut pushed = 0u32;

    for (dimension, quantity_of) in DIMENSIONS {
        let quantity = quantity_of(row);
        if quantity <= 0 {
            continue;
        }

        // Scoped by `stripe_customer_id` in addition to project/dimension/date:
        // `projects.id` is a real PK in production (globally unique — this
        // extra scoping changes nothing for a real deployment, since a given
        // project id only ever belongs to one Stripe customer there). It
        // matters for THIS shared Stripe test-mode account, which otherwise
        // sees the SAME idempotency key reused across isolated test runs
        // that legitimately push different quantities under the same
        // project id, which Stripe's real idempotency guarantee correctly
        // rejects (`idempotency_error`: "same key, different parameters").
        let idempotency_key =
            format!("{stripe_customer_id}:{}:{dimension}:{yesterday}", row.project_id);
        let is_new = state
            .stripe_gateway
            .push_usage_record(
                stripe_customer_id,
                dimension,
                quantity as u64,
                yesterday
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is a valid time")
                    .and_utc(),
                &idempotency_key,
            )
            .await?;

        if is_new {
            pushed += 1;
        }
    }

    Ok(pushed)
}

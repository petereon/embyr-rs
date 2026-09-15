// @driving_port @real-io @adapter-integration @infrastructure-failure @US-01
#![allow(unused_imports)]
//! rate-limiter-fail-open (finding #20, Medium/Security) — a real Postgres
//! ERROR encountered inside `check_pg` (not merely a slow 20ms timeout) must
//! degrade to the SAME bounded per-instance fallback (`check_in_process`)
//! the timeout path already uses, never to an unconditional allow with a
//! freshly-inserted full-capacity `rate_buckets` row.
//!
//! Acceptance criteria verified here (feature-delta.md DISCUSS+DESIGN):
//!   AC-RLFO-01: Postgres healthy -> allow/reject behaviour unchanged (regression).
//!   AC-RLFO-02: a real error on the atomic UPDATE routes to the per-instance
//!               fallback bucket, never an unconditional allow, never inserts
//!               a fresh full-capacity row.
//!   AC-RLFO-03: a real error on the disambiguating `SELECT EXISTS` routes to
//!               the same fallback — never misread as "row absent, allow."
//!   AC-RLFO-04: a genuinely successful EXISTS returning `false` (row truly
//!               absent, zero Postgres error) still inserts a default row and
//!               allows — unchanged regression.
//!   AC-RLFO-05: during a sustained window of real errors, no project is ever
//!               unconditionally allowed — every project's excess requests
//!               beyond its own per-instance capacity are rejected, proven
//!               for 2 independent projects.
//!   AC-RLFO-06: the EXISTING 20ms-timeout fallback (ADR-015 D3) and its
//!               `rate_limit_pg_timeout_total` counter are unchanged by this
//!               fix.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient -> GetDocument), same
//! entry point b12/b13 already use — this fix changes no port, only
//! `check_pg`'s internal `Result` handling (feature-delta.md, DESIGN).
//!
//! Postgres fault-injection mechanism (DISTILL's own choice — DESIGN's
//! Handoff Package left the exact mechanism to DISTILL, naming
//! `pg_terminate_backend` / an immediately-failing pool acquire as valid
//! candidates): a dedicated, non-superuser Postgres ROLE plus row-level
//! security (RLS) policies keyed on STATEMENT TYPE (`FOR SELECT` vs
//! `FOR UPDATE`), scoped to `rate_buckets` only. See
//! `tests/distributed_rate_limiting/common/mod.rs`'s own
//! `force_select_errors_on_rate_buckets` / `force_update_errors_on_rate_buckets`
//! / `force_all_errors_on_rate_buckets` doc comments for why container
//! stop/start (this workspace's `pr08_realtime_listener_reconnect.rs`
//! precedent) and superuser-role privilege tricks CANNOT isolate one of
//! `check_pg`'s two sequential queries from the other, and why this
//! mechanism can. This is real, non-mocked Postgres fault injection — a
//! genuine `sqlx::Error::Database` — never a `tokio::time::timeout` elapsing
//! (that path is AC-RLFO-06's own, deliberately UNCHANGED, concern).
//!
//! "Real DB error" vs "legitimately not found" fixture distinction (the
//! task's own core testing challenge): AC-RLFO-04's fixture is a HEALTHY
//! Postgres pool (no RLS, default superuser role) with a project row that
//! was simply never inserted into `rate_buckets` — both the UPDATE and the
//! EXISTS query genuinely SUCCEED and correctly report "no row" (`Ok(None)`/
//! `Ok(false)`), never `Err`. AC-RLFO-02/03/05's fixtures are the OPPOSITE:
//! the project row DOES exist (seeded via the superuser `ctx.pool`), but the
//! scoped role's own query against it is forced to `Err` by an RLS policy —
//! the row's mere existence is irrelevant; the point is the QUERY never
//! completes successfully. These are two structurally different DB setups
//! (schema/data state vs. privilege/policy state), never the same fixture
//! reused for both.
//!
//! Layer: subprocess/FS acceptance (real Postgres container + real
//! in-process gRPC server, ~100ms-1s each) per `nw-test-design-mandates`
//! Layered Test Discipline — example-only (Mandate 11: layer 3+ sad paths
//! are enumerated, never PBT-generated), state-delta + Universe applied
//! where the claim is a DB state mutation (Mandate 8).
//!
//! Scaffold state: NONE. `check_pg`/`check_inner`/`check_in_process` all
//! already exist and compile — this is a bug fix, not a new module. All 6
//! tests are expected to FAIL against today's code because the CURRENT
//! (buggy) behaviour allows the request unconditionally instead of routing
//! to the per-instance fallback — see feature-delta.md's own RED-verification
//! note. Not a test-setup bug.
//!
//! Walking skeleton (first test NOT `#[ignore]`): `AC-RLFO-02`
//! (`atomic_update_pg_error_falls_back_to_per_instance_bucket`) — matches
//! DISCUSS's own locked Walking Skeleton Strategy verbatim (real Postgres
//! error on the atomic UPDATE, real running server, per-instance fallback).
//! DELIVER unskips the remaining 5 one at a time.

#[path = "../common/mod.rs"]
mod common;
use common::{
    classify_grpc_status, force_all_errors_on_rate_buckets, force_select_errors_on_rate_buckets,
    force_update_errors_on_rate_buckets, get_document_request, get_metrics,
    scoped_rate_limiter_pool, start_distributed_grpc_server, start_distributed_grpc_server_with_pool,
    universe, assert_state_delta, set_to, warm_credential_cache_and_reset_bucket, DrlTestContext,
};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use std::collections::HashMap;

/// Read a label-free Prometheus counter's current value from a scraped
/// `/metrics` body — `embyr_rate_limit_pg_timeout_total` and the new
/// `embyr_rate_limit_pg_error_total` are both registered with NO labels
/// (`metrics::counter!("name").increment(1)`), which the exporter renders as
/// `name value` (a bare space, never `name{...} value`) — distinct from
/// `common::metric_value`'s label-aware `name{...}` matching, which would
/// never match a labelless line. Mirrors
/// `tests/customer_db_transaction_sweeper/acceptance/us02_purge_terminal_transactions.rs`'s
/// own `read_counter` (same labelless-counter shape), adapted to read from
/// an already-scraped HTTP body rather than the in-process recorder.
fn labelless_metric(body: &str, name: &str) -> f64 {
    let prefix = format!("{name} ");
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix(&prefix) {
            return value.trim().parse().unwrap_or(0.0);
        }
    }
    0.0
}

/// Snapshot the `rate_buckets` Universe for one project — port-exposed DB
/// state, read via the SUPERUSER `ctx.pool` (unaffected by any RLS policy
/// wired onto the scoped role) so the assertion itself never depends on the
/// same fault being injected.
async fn capture_bucket_universe(
    ctx: &DrlTestContext,
    project_id: &str,
) -> HashMap<&'static str, Option<String>> {
    let mut snapshot = HashMap::new();
    let row = ctx.query_rate_bucket(project_id).await;
    snapshot.insert(universe::RATE_BUCKET_EXISTS, Some(row.is_some().to_string()));
    snapshot.insert(
        universe::RATE_BUCKET_TOKENS,
        row.map(|(tokens, _)| format!("{tokens:.1}")),
    );
    snapshot
}

// ─── AC-RLFO-01: healthy-path regression ─────────────────────────────────────

/// With Postgres healthy, allow/reject behaviour is byte-for-byte unchanged
/// from before this fix — no fallback path is triggered.
///
/// Journey:
///   Given: Fernbank Analytics' rate_buckets row has 2 tokens remaining
///   And:   Postgres is healthy (no fault injection)
///   When:  Fernbank Analytics sends 3 requests in quick succession
///   Then:  the first 2 requests are allowed
///   And:   the 3rd request is rejected with RESOURCE_EXHAUSTED
///   And:   no `embyr_rate_limit_pg_error_total` increment occurs
///
/// @driving_port @real-io @US-01 @AC-RLFO-01
#[tokio::test]
async fn pg_healthy_rate_limiting_behaves_unchanged() {
    let ctx = DrlTestContext::new(2.0).await;
    ctx.insert_project_with_bucket("fernbank-analytics", "key-fb-01", 2.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    // Pay the one-time Argon2id cost + populate the credential cache before
    // the timed, boundary-sensitive sequence below (see helper doc comment).
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "fernbank-analytics", "key-fb-01", 2.0)
        .await;

    let before_body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let before_errors = labelless_metric(&before_body, "embyr_rate_limit_pg_error_total");

    let t0 = std::time::Instant::now();
    for i in 0..2 {
        let result = client
            .get_document(get_document_request("fernbank-analytics", "key-fb-01"))
            .await;
        eprintln!("DEBUG req {i} at {:?}: {:?}", t0.elapsed(), classify_grpc_status(&result));
        assert_ne!(classify_grpc_status(&result), "resource_exhausted");
    }
    let third = client
        .get_document(get_document_request("fernbank-analytics", "key-fb-01"))
        .await;
    eprintln!("DEBUG req 3 at {:?}: {:?}", t0.elapsed(), classify_grpc_status(&third));
    assert_eq!(classify_grpc_status(&third), "resource_exhausted");

    let after_body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let after_errors = labelless_metric(&after_body, "embyr_rate_limit_pg_error_total");
    assert_eq!(
        after_errors, before_errors,
        "healthy-path requests must never increment the new pg-error fallback counter"
    );
}

// ─── AC-RLFO-02 (walking skeleton): atomic UPDATE error -> bounded fallback ──

/// A real Postgres error on the atomic UPDATE routes the request through the
/// per-instance fallback bucket, never through an unconditional allow, and
/// never inserts a fresh full-capacity `rate_buckets` row as a side effect.
///
/// Journey:
///   Given: Solstice Retail's rate_buckets row has 3 tokens (healthy value)
///   And:   the shared system Postgres pool is forced to error on every
///          atomic UPDATE against `rate_buckets` (not a timeout)
///   When:  Solstice Retail sends 4 requests (capacity + 1) in quick succession
///   Then:  the first 3 requests are allowed (per-instance fallback, capped
///          at the same configured capacity)
///   And:   the 4th request is rejected with RESOURCE_EXHAUSTED
///   And:   Solstice Retail's rate_buckets row is unchanged by the error
///          (no fresh full-capacity bucket was written as a side effect)
///
/// @walking_skeleton @driving_port @real-io @infrastructure-failure @US-01 @AC-RLFO-02
///
/// NOT #[ignore] — this is the walking skeleton.
#[tokio::test]
async fn atomic_update_pg_error_falls_back_to_per_instance_bucket() {
    let ctx = DrlTestContext::new(3.0).await;
    ctx.insert_project_with_bucket("solstice-retail", "key-solstice-02", 3.0).await;
    let scoped_pool = scoped_rate_limiter_pool(&ctx).await;

    let (addr, server) = start_distributed_grpc_server_with_pool(&ctx, scoped_pool).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    // Warm up the credential cache (real, healthy Postgres — no RLS yet) and
    // reset the bucket before installing the fault, so the timed loop below
    // never pays a mid-sequence Argon2id tax (see helper doc comment).
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "solstice-retail", "key-solstice-02", 3.0)
        .await;
    force_update_errors_on_rate_buckets(&ctx.pool).await;

    let before = capture_bucket_universe(&ctx, "solstice-retail").await;

    for attempt in 1..=3 {
        let result = client
            .get_document(get_document_request("solstice-retail", "key-solstice-02"))
            .await;
        assert_ne!(
            classify_grpc_status(&result),
            "resource_exhausted",
            "expected request {attempt} (within the per-instance capacity) to be allowed \
             via the bounded fallback bucket, not rejected"
        );
    }
    let fourth = client
        .get_document(get_document_request("solstice-retail", "key-solstice-02"))
        .await;
    assert_eq!(
        classify_grpc_status(&fourth),
        "resource_exhausted",
        "expected the 4th request to exceed the per-instance fallback's own capacity cap \
         — an unconditional allow here would mean the UPDATE error was silently treated \
         as success (the finding #20 defect)"
    );

    let after = capture_bucket_universe(&ctx, "solstice-retail").await;
    let mut expected = HashMap::new();
    expected.insert(universe::RATE_BUCKET_TOKENS, set_to(Some("3.0".to_string())));
    expected.insert(universe::RATE_BUCKET_EXISTS, set_to(Some("true".to_string())));
    assert_state_delta(
        &before,
        &after,
        &[universe::RATE_BUCKET_TOKENS, universe::RATE_BUCKET_EXISTS],
        &expected,
    );

    let body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let error_count = labelless_metric(&body, "embyr_rate_limit_pg_error_total");
    assert!(
        error_count >= 1.0,
        "expected embyr_rate_limit_pg_error_total to have incremented at least once across \
         4 requests each hitting the forced UPDATE error, got {error_count}"
    );
}

// ─── AC-RLFO-03: disambiguating EXISTS error -> bounded fallback ────────────

/// A real Postgres error on the disambiguating `SELECT EXISTS` (reached only
/// after the atomic UPDATE genuinely returns 0 rows updated) routes the
/// request through the per-instance fallback bucket — never assumes "row
/// absent, allow" on the basis of the error.
///
/// Journey:
///   Given: Meridian Labs' rate_buckets row genuinely has 0 tokens (a real,
///          successful "rate limited" outcome — the atomic UPDATE's own
///          WHERE clause legitimately matches 0 rows, no error there)
///   And:   the follow-up SELECT EXISTS query is forced to error
///   When:  Meridian Labs sends 3 requests (capacity + 1) in quick succession
///   Then:  the first 2 requests are allowed via the per-instance fallback
///   And:   the 3rd request is rejected with RESOURCE_EXHAUSTED
///   And:   Meridian Labs' rate_buckets row is unchanged (0 tokens) — no
///          fresh full-capacity row was inserted on the assumption the row
///          was absent
///
/// @driving_port @real-io @infrastructure-failure @US-01 @AC-RLFO-03
#[tokio::test]
async fn exists_check_pg_error_falls_back_to_per_instance_bucket() {
    let ctx = DrlTestContext::new(2.0).await;
    ctx.insert_project_with_bucket("meridian-labs", "key-meridian-03", 0.0).await;
    let scoped_pool = scoped_rate_limiter_pool(&ctx).await;

    let (addr, server) = start_distributed_grpc_server_with_pool(&ctx, scoped_pool).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    // Warm up the credential cache (real, healthy Postgres — no RLS yet) and
    // reset the bucket to its real 0.0-tokens precondition before installing
    // the fault (see helper doc comment).
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "meridian-labs", "key-meridian-03", 0.0)
        .await;
    force_select_errors_on_rate_buckets(&ctx.pool).await;

    let before = capture_bucket_universe(&ctx, "meridian-labs").await;

    for attempt in 1..=2 {
        let result = client
            .get_document(get_document_request("meridian-labs", "key-meridian-03"))
            .await;
        assert_ne!(
            classify_grpc_status(&result),
            "resource_exhausted",
            "expected request {attempt} to be allowed via the bounded fallback bucket"
        );
    }
    let third = client
        .get_document(get_document_request("meridian-labs", "key-meridian-03"))
        .await;
    assert_eq!(
        classify_grpc_status(&third),
        "resource_exhausted",
        "expected the 3rd request to exceed the per-instance fallback cap — an unconditional \
         allow here would mean the EXISTS error was misread as \"row absent\" (finding #20)"
    );

    let after = capture_bucket_universe(&ctx, "meridian-labs").await;
    let mut expected = HashMap::new();
    expected.insert(universe::RATE_BUCKET_TOKENS, set_to(Some("0.0".to_string())));
    expected.insert(universe::RATE_BUCKET_EXISTS, set_to(Some("true".to_string())));
    assert_state_delta(
        &before,
        &after,
        &[universe::RATE_BUCKET_TOKENS, universe::RATE_BUCKET_EXISTS],
        &expected,
    );

    let body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let error_count = labelless_metric(&body, "embyr_rate_limit_pg_error_total");
    assert!(
        error_count >= 1.0,
        "expected embyr_rate_limit_pg_error_total to have incremented for the forced \
         EXISTS-query error path, got {error_count}"
    );
}

// ─── AC-RLFO-04 (regression guard): legitimately absent row ─────────────────

/// A genuinely absent `rate_buckets` row (pre-migration-0018 project, ZERO
/// Postgres error — both queries succeed and correctly report "not found")
/// still inserts a default row and allows the request, unchanged from
/// before this fix.
///
/// Journey:
///   Given: Heritage Mutual is a provisioned project with NO rate_buckets row
///   And:   the shared system Postgres pool is healthy (no fault injection)
///   When:  Heritage Mutual sends a request
///   Then:  a default rate_buckets row is inserted for the project
///   And:   the request is allowed
///
/// @driving_port @real-io @US-01 @AC-RLFO-04
#[tokio::test]
async fn legitimately_absent_row_still_allowed_with_fresh_bucket() {
    let ctx = DrlTestContext::new(5.0).await;
    ctx.insert_project_without_bucket("heritage-mutual", "key-heritage-04").await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    let before = capture_bucket_universe(&ctx, "heritage-mutual").await;

    let result = client
        .get_document(get_document_request("heritage-mutual", "key-heritage-04"))
        .await;
    assert_ne!(
        classify_grpc_status(&result),
        "resource_exhausted",
        "a genuinely absent row (no Postgres error) must still be allowed with a fresh bucket"
    );

    let after = capture_bucket_universe(&ctx, "heritage-mutual").await;
    let mut expected = HashMap::new();
    expected.insert(universe::RATE_BUCKET_EXISTS, set_to(Some("true".to_string())));
    expected.insert(universe::RATE_BUCKET_TOKENS, set_to(Some("4.0".to_string())));
    assert_state_delta(
        &before,
        &after,
        &[universe::RATE_BUCKET_EXISTS, universe::RATE_BUCKET_TOKENS],
        &expected,
    );
}

// ─── AC-RLFO-05: sustained errors do not disable rate limiting for any project ─

/// During a sustained window of real Postgres errors (full outage of the
/// rate limiter's own data path), no project's requests are unconditionally
/// allowed — every project's excess requests beyond its OWN per-instance
/// capacity are rejected, proven for 2 independent projects.
///
/// Journey:
///   Given: Atlas Freight and Borealis Health each have a healthy
///          rate_buckets row
///   And:   the shared system Postgres pool is forced to error on every
///          statement against rate_buckets (sustained outage)
///   When:  each project sends more requests than its own per-instance
///          capacity allows
///   Then:  each project's excess requests are rejected by its OWN bucket
///   And:   no project receives unlimited allowed requests during the outage
///
/// @driving_port @real-io @infrastructure-failure @US-01 @AC-RLFO-05
#[tokio::test]
async fn sustained_pg_errors_do_not_disable_rate_limiting_for_any_project() {
    let ctx = DrlTestContext::new(2.0).await;
    ctx.insert_project_with_bucket("atlas-freight", "key-atlas-05", 2.0).await;
    ctx.insert_project_with_bucket("borealis-health", "key-borealis-05", 2.0).await;
    let scoped_pool = scoped_rate_limiter_pool(&ctx).await;

    let (addr, _server) = start_distributed_grpc_server_with_pool(&ctx, scoped_pool).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    // Warm up both projects' credential cache (real, healthy Postgres — no
    // RLS yet) and reset their buckets before installing the sustained
    // fault (see helper doc comment).
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "atlas-freight", "key-atlas-05", 2.0)
        .await;
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "borealis-health", "key-borealis-05", 2.0)
        .await;
    force_all_errors_on_rate_buckets(&ctx.pool).await;

    for (project_id, api_key) in [
        ("atlas-freight", "key-atlas-05"),
        ("borealis-health", "key-borealis-05"),
    ] {
        for attempt in 1..=2 {
            let result = client.get_document(get_document_request(project_id, api_key)).await;
            assert_ne!(
                classify_grpc_status(&result),
                "resource_exhausted",
                "expected {project_id}'s request {attempt} to be allowed via its own \
                 per-instance fallback bucket during the sustained outage"
            );
        }
        let excess = client.get_document(get_document_request(project_id, api_key)).await;
        assert_eq!(
            classify_grpc_status(&excess),
            "resource_exhausted",
            "expected {project_id}'s excess request to be rejected — an unconditional allow \
             here, for EITHER project, means sustained Postgres errors silently turned off \
             rate limiting cluster-wide (finding #20's own security property)"
        );
    }
}

// ─── AC-RLFO-06 (regression guard): existing 20ms-timeout fallback unchanged ─

/// The EXISTING 20ms-timeout-triggered fallback (ADR-015 D3) — a genuinely
/// SLOW Postgres call, not an error — continues to fire exactly as before
/// this fix, incrementing the pre-existing `rate_limit_pg_timeout_total`
/// counter, never the new `embyr_rate_limit_pg_error_total` counter this fix
/// adds.
///
/// Slowness mechanism: the rate limiter's own pool is capped at exactly 1
/// connection, and that single connection is held (acquired, never
/// returned) for the test's duration — the NEXT `check_pg` call blocks on
/// pool-acquire well past the 20ms budget, exercising the SAME
/// `tokio::time::timeout` wrap `check_inner` already has. This is real pool
/// exhaustion (one of the concrete causes DISCUSS's own Reading
/// Confirmation names for a slow Postgres round-trip), not a mocked delay.
///
/// Journey:
///   Given: Summit Ventures has a healthy rate_buckets row
///   And:   the rate limiter's own Postgres pool has its only connection
///          held open (unavailable) for the whole request
///   When:  Summit Ventures sends a request
///   Then:  the request is evaluated against the per-instance fallback
///          bucket, exactly as before this fix
///   And:   `embyr_rate_limit_pg_timeout_total` increments by 1
///   And:   `embyr_rate_limit_pg_error_total` does NOT increment (this is
///          the timeout path, not the error path this fix adds)
///
/// @driving_port @real-io @infrastructure-failure @US-01 @AC-RLFO-06
#[tokio::test]
async fn existing_timeout_fallback_is_unchanged_by_this_fix() {
    let ctx = DrlTestContext::new(5.0).await;
    ctx.insert_project_with_bucket("summit-ventures", "key-summit-06", 5.0).await;

    let starved_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&ctx.db_url)
        .await
        .expect("connect single-connection pool for timeout simulation");
    // Acquire and hold the pool's ONLY connection so the rate limiter's own
    // `check_pg` call can never acquire one within the 20ms budget.
    let _held_connection = starved_pool
        .acquire()
        .await
        .expect("acquire the single connection to starve the pool");

    let (addr, server) = start_distributed_grpc_server_with_pool(&ctx, starved_pool).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    let before_body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let before_timeouts = labelless_metric(&before_body, "embyr_rate_limit_pg_timeout_total");
    let before_errors = labelless_metric(&before_body, "embyr_rate_limit_pg_error_total");

    let result = client
        .get_document(get_document_request("summit-ventures", "key-summit-06"))
        .await;
    assert_ne!(
        classify_grpc_status(&result),
        "resource_exhausted",
        "expected the first request on a fresh per-instance fallback bucket to be allowed"
    );

    let after_body = get_metrics(&ctx.admin_client, server.admin_addr).await;
    let after_timeouts = labelless_metric(&after_body, "embyr_rate_limit_pg_timeout_total");
    let after_errors = labelless_metric(&after_body, "embyr_rate_limit_pg_error_total");

    assert_eq!(
        after_timeouts,
        before_timeouts + 1.0,
        "expected the EXISTING pg-timeout counter to increment exactly as before this fix"
    );
    assert_eq!(
        after_errors, before_errors,
        "expected the NEW pg-error counter to stay untouched by a timeout (not an error) \
         — conflating the two would be a regression to this fix's own OQ-RLFO-02 resolution"
    );
}

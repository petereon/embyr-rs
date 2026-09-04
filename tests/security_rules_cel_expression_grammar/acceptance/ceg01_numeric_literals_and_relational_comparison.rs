//! CEG01 (Slice 01, US-01, Release 1, Walking Skeleton) — Alex's Numeric-Bound
//! Clause Parses and Enforces on Reads.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-065):
//!   AC-CEG-01: `resource.data.<field> <=/</>/>=  <integer literal>` parses
//!              into a new `Condition::Compare` variant carrying the new
//!              relational operator.
//!   AC-CEG-02: a bare integer literal parses into `Operand::IntLiteral` —
//!              distinguishable from the pre-existing `SyntaxError`
//!              catch-all a digit currently always hits.
//!   AC-CEG-03: a real `GetDocument` request against a document whose
//!              numeric field satisfies (or doesn't satisfy) the bound is
//!              allowed (or denied) correctly.
//!   AC-CEG-04: a `Double` literal parses and compares correctly against a
//!              `FieldValue::Double` resource field.
//!   AC-CEG-05: comparing a numeric operand against a non-numeric resource
//!              field is a well-defined `Deny`, never a panic.
//!
//! Driving ports: Admin HTTP :9090 (`define_access_rule`, import proof) +
//! gRPC :8080 `GetDocument` (`SecurityRulesFullContext`) — real enforcement,
//! not merely parsed-and-discarded (mirrors every prior JOB-17 epic's own
//! Slice 01 discipline).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

fn resource_name(project_id: &str, path: &str) -> String {
    format!("projects/{project_id}/databases/(default)/documents/{path}")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-01/02/03: an integer-literal `<=` bound parses, imports, and gates a
// real GetDocument call correctly in both directions.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex imports `resource.data.photo_count <= 20` on
///          `journal_entries`
///   When:  Maria reads a journal entry whose `photo_count` is 15 (within
///          bound), and separately one whose `photo_count` is 25 (over
///          bound)
///   Then:  the first read succeeds, the second is denied
///
/// AC-CEG-01, AC-CEG-02, AC-CEG-03
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CEG-01 @AC-CEG-02 @AC-CEG-03
#[tokio::test]
async fn a_numeric_bound_clause_imports_and_gates_a_real_read_in_both_directions() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg01-numericbound").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.photo_count <= 20",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(
        define_resp.status().as_u16(),
        200,
        "AC-CEG-01/02: a numeric-bound relational-comparison condition must be accepted, not \
         rejected as a SyntaxError"
    );

    ctx.seed_document(
        "journal_entries",
        "within-bound",
        serde_json::json!({ "photo_count": {"t": "I", "v": 15} }),
    )
    .await;
    ctx.seed_document(
        "journal_entries",
        "over-bound",
        serde_json::json!({ "photo_count": {"t": "I", "v": 25} }),
    )
    .await;

    let within = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/within-bound"),
            None,
        )
        .await;
    assert!(
        within.is_ok(),
        "AC-CEG-03: photo_count=15 satisfies '<= 20' and must be allowed: {:?}",
        within.err()
    );

    let over = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/over-bound"),
            None,
        )
        .await;
    let err = over.expect_err("AC-CEG-03: photo_count=25 violates '<= 20' and must be denied");
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-04: a Double literal parses and compares correctly.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CEG-04
///
/// @driving_port @real-io @US-01 @AC-CEG-04
#[tokio::test]
async fn a_double_literal_bound_imports_and_gates_a_real_read() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg01-doublebound").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "trail_guides",
            "condition": "resource.data.difficulty_rating < 4.5",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(define_resp.status().as_u16(), 200, "AC-CEG-04: a Double literal must be accepted");

    ctx.seed_document(
        "trail_guides",
        "moderate",
        serde_json::json!({ "difficulty_rating": {"t": "D", "v": 3.2} }),
    )
    .await;

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "trail_guides/moderate"),
            None,
        )
        .await;
    assert!(
        resp.is_ok(),
        "AC-CEG-04: difficulty_rating=3.2 satisfies '< 4.5' and must be allowed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-CEG-05: a type mismatch (numeric operand vs. non-numeric field) denies,
// never panics.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-CEG-05
///
/// @error @driving_port @real-io @US-01 @AC-CEG-05
#[tokio::test]
async fn comparing_a_numeric_operand_against_a_non_numeric_field_denies_without_panicking() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ceg01-typemismatch").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let define_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "resource.data.title <= 20",
        }))
        .send()
        .await
        .expect("define_access_rule request failed");
    assert_eq!(define_resp.status().as_u16(), 200);

    ctx.seed_document(
        "journal_entries",
        "string-field",
        serde_json::json!({ "title": {"t": "S", "v": "Day One"} }),
    )
    .await;

    let resp = ctx
        .get_document(
            &resource_name(&ctx.project_id, "journal_entries/string-field"),
            None,
        )
        .await;
    let err = resp.expect_err(
        "AC-CEG-05: a numeric comparison against a string field must deny cleanly, never panic \
         (evaluate() stays total/infallible by construction)",
    );
    assert_eq!(err.code(), tonic::Code::PermissionDenied, "got {:?}", err.code());
}

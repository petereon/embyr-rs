//! CF03 (Slice 03, US-03, Release 1, LAST slice of Release 1) — Alex
//! Simulates a Function-Call-Bearing Candidate Rule.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-067):
//!   AC-CF-06: `simulate_access_rule`, given the STORED (already-expanded)
//!             condition from a function-authored rule, produces the
//!             correct allow/deny outcome — zero new `simulate_access_rule`
//!             code path.
//!
//! This slice is CONFIRMATORY (ADR-067 § Resolution 5): once Slice 01's own
//! import expands the function call into the stored `condition_source`,
//! `simulate_access_rule` behaves identically to how it already does for
//! any hand-authored condition — the stored text carries no trace of the
//! function-call syntax that produced it.
//!
//! Driving port: admin HTTP :9090 (import, then `POST .../access_rules/
//! simulate`), via `SecurityRulesAdminContext` (no gRPC needed — mirrors
//! CEG07's own `simulate_access_rule` precedent).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

const EDITOR_RULES_FILE: &str = r#"
service cloud.firestore {
  function isEditor() {
    return request.auth.uid == resource.data.editor_id;
  }
  match /databases/{database}/documents {
    match /trail_guides/{guideId} {
      allow read: if isEditor();
    }
  }
}
"#;

/// AC-CF-06
///
/// @driving_port @real-io @US-03 @AC-CF-06
#[tokio::test]
async fn simulating_the_stored_already_expanded_condition_returns_the_correct_outcome() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let import_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": EDITOR_RULES_FILE }))
        .send()
        .await
        .expect("import request failed");
    assert_eq!(import_resp.status().as_u16(), 200, "AC-CF-06: import must succeed");

    // The candidate condition passed to simulate is the SAME literal text
    // Alex would copy from the imported rule's own storage — already fully
    // expanded, carrying no `isEditor()` call syntax at all.
    let expanded_condition = "(request.auth.uid == resource.data.editor_id)";

    let allow_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": expanded_condition,
            "auth": {"uid": "maria-santos"},
            "resource": {"editor_id": "maria-santos"},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(allow_resp.status().as_u16(), 200);
    let allow_body: serde_json::Value = allow_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        allow_body["outcome"], "allow",
        "AC-CF-06: the document's own editor must simulate as 'allow'"
    );

    let deny_resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": expanded_condition,
            "auth": {"uid": "dana-kim"},
            "resource": {"editor_id": "maria-santos"},
        }))
        .send()
        .await
        .expect("simulate_access_rule request failed");
    assert_eq!(deny_resp.status().as_u16(), 200);
    let deny_body: serde_json::Value = deny_resp.json().await.expect("response body must be JSON");
    assert_eq!(
        deny_body["outcome"], "deny",
        "AC-CF-06: a different caller must simulate as 'deny'"
    );
}

//! Slice 06 (US-06, ADR-034) — Alex Compares a Claim (or a Resource Field)
//! to a String Literal.
//!
//! Acceptance criteria verified here (feature-delta.md US-06):
//!   AC-17-151: a claim compared to a string literal (`request.auth.token.
//!              department == "billing"`) parses and evaluates correctly —
//!              matching and non-matching claim values.
//!   AC-17-152: a RESOURCE FIELD compared to a string literal
//!              (`resource.data.status == "published"`, no claim involved)
//!              also works — proving the tokenizer/parser fix is general,
//!              not claims-specific.
//!   AC-17-153: an unterminated string literal is rejected as a plain
//!              `SyntaxError` via the admin API's own `define_access_rule`,
//!              distinguishable from `UnsupportedConstruct`; a string VALUE
//!              containing a `**`/`{`/call-shaped substring is correctly
//!              treated as opaque content, not misclassified as
//!              `UnsupportedConstruct` — proving `detect_unsupported_construct`'s
//!              required companion quote-awareness fix (ADR-034 § StringLiteral
//!              and the tokenizer).
//!
//! Driving ports: gRPC :8080 `GetDocument` (AC-17-151/152, via
//! `SecurityRulesFullContext`, mirroring `boolean_claim_gates_getdocument.rs`'s
//! own precedent) and Admin HTTP :9090 `POST .../access_rules` (AC-17-153,
//! `define_access_rule` — real rejection-reason taxonomy, not a re-derived
//! copy).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token_with_claims, now_unix, string_field,
    SecurityRulesFullContext,
};

fn resource_name(ctx: &SecurityRulesFullContext, collection: &str, doc_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        ctx.project_id, collection, doc_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-151: a claim compared to a string literal correctly gates access —
// matching value allows.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `support_tickets` has a rule `request.auth.token.department ==
///          "billing"`
///   And:   Jordan Lee holds a verified identity with claim
///          `department: "billing"`
///   When:  Jordan calls `getDoc()` on a `support_tickets` document
///   Then:  the read succeeds
///
/// AC-17-151
///
/// @driving_port @real-io @US-06 @AC-17-151
#[tokio::test]
async fn a_claim_matching_a_string_literal_allows() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc06-allow").await;
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "support_tickets",
        "request.auth.token.department == \"billing\"",
    )
    .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("subject".to_string(), string_field("billing question"));
    create_document(&ctx, "support_tickets", "cc06-allow-doc", fields, None)
        .await
        .expect("seed support_tickets document");

    let jordans_token = mint_client_identity_token_with_claims(
        &signing_key,
        "jordan-lee",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"department": "billing"}),
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "support_tickets", "cc06-allow-doc"),
            Some(&jordans_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-151: a matching department claim/string-literal must allow: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-151: a claim compared to a string literal correctly gates access —
// non-matching value denies.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `support_tickets` has a rule `request.auth.token.department ==
///          "billing"`
///   And:   a colleague from engineering holds a verified identity with
///          claim `department: "engineering"`
///   When:  that colleague calls `getDoc()` on the same document
///   Then:  the read is denied with PermissionDenied
///
/// AC-17-151
///
/// @error @driving_port @real-io @US-06 @AC-17-151
#[tokio::test]
async fn a_claim_not_matching_a_string_literal_denies() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc06-deny").await;
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "support_tickets",
        "request.auth.token.department == \"billing\"",
    )
    .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("subject".to_string(), string_field("billing question"));
    create_document(&ctx, "support_tickets", "cc06-deny-doc", fields, None)
        .await
        .expect("seed support_tickets document");

    let engineers_token = mint_client_identity_token_with_claims(
        &signing_key,
        "sam-osei",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"department": "engineering"}),
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "support_tickets", "cc06-deny-doc"),
            Some(&engineers_token),
        )
        .await;

    let err = resp.expect_err(
        "AC-17-151: a mismatched department claim/string-literal must deny, not allow",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-151: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-152: a RESOURCE FIELD compared to a string literal also works — no
// claim involved at all, proving the fix is general, not claims-specific.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `articles` has a rule `resource.data.status == "published"`
///   And:   a document exists with `status: "published"`
///   When:  any authorized caller calls `getDoc()` on that document
///   Then:  the condition correctly evaluates true — the read succeeds
///
/// AC-17-152
///
/// @driving_port @real-io @US-06 @AC-17-152
#[tokio::test]
async fn a_resource_field_matching_a_string_literal_allows_proving_the_fix_is_general() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc06-field").await;
    ctx.seed_access_rule("articles", "resource.data.status == \"published\"")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("status".to_string(), string_field("published"));
    create_document(&ctx, "articles", "cc06-field-doc", fields, None)
        .await
        .expect("seed articles document");

    // No claim, no verified identity at all — the rule never references
    // `request.auth.*`, so an anonymous caller must be admitted purely on
    // the resource-field/string-literal comparison.
    let resp = ctx
        .get_document(&resource_name(&ctx, "articles", "cc06-field-doc"), None)
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-152: a matching resource-field/string-literal comparison must allow, \
         independent of any claim: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-153: an unterminated string literal is a plain syntax error via the
// admin API's own define_access_rule, distinguishable from
// UnsupportedConstruct.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a valid admin session
///   When:  Alex submits the condition `request.auth.token.department ==
///          "billing` (no closing quote) via `define_access_rule`
///   Then:  it is rejected as a plain syntax error, not an
///          `UnsupportedConstruct`
///
/// AC-17-153
///
/// @error @driving_port @real-io @US-06 @AC-17-153
#[tokio::test]
async fn an_unterminated_string_literal_is_rejected_as_a_plain_syntax_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc06-unterminated").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "support_tickets",
            "condition": "request.auth.token.department == \"billing",
        }))
        .send()
        .await
        .expect("define request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-17-153: an unterminated string literal must be rejected 400"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"], "SYNTAX_ERROR",
        "AC-17-153: an unterminated string literal must be a plain syntax error, \
         distinguishable from UNSUPPORTED_CONSTRUCT"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-153: a string value containing a `**`/`{`/call-shaped substring is
// opaque string content, NOT misclassified as UnsupportedConstruct — proves
// the detect_unsupported_construct companion fix (ADR-034).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex holds a valid admin session
///   When:  Alex submits a condition comparing a claim to a string literal
///          whose CONTENT happens to look like a wildcard path (`**`) or a
///          call-shaped substring (`get(weird)`)
///   Then:  the condition is accepted (200) — the content is opaque string
///          data, not a grammar construct
///
/// AC-17-153
///
/// @driving_port @real-io @US-06 @AC-17-153
#[tokio::test]
async fn a_string_value_resembling_wildcard_or_call_syntax_is_treated_as_opaque_content() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc06-opaque").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let client = reqwest::Client::new();

    for suspicious_value in ["a**b", "get(weird)"] {
        let condition = format!("request.auth.token.label == \"{suspicious_value}\"");
        let resp = client
            .post(ctx.admin_url(&format!(
                "/admin/v1/projects/{}/access_rules",
                ctx.project_id
            )))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "support_tickets",
                "condition": condition,
            }))
            .send()
            .await
            .expect("define request failed");

        assert_eq!(
            resp.status().as_u16(),
            200,
            "AC-17-153: a string literal containing '{suspicious_value}' must parse as \
             opaque content, not be misclassified as UnsupportedConstruct"
        );
    }
}

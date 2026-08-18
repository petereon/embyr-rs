//! Common test infrastructure — security-rules-write-path acceptance tests
//! (Slice 01, ADR-030).
//!
//! Reuses `SecurityRulesAdminContext` from `security-rules`' own fixture
//! module via a path import (per the DISTILL/DELIVER dispatch instructions
//! — this feature extends the SAME admin surface, no new driving-port
//! context class needed). `access_rules`, its adapter methods, and its own
//! test file are never touched — only imported, read-only, from here.
//!
//! `write_access_rules` is a NEW, independent table (ADR-030 § Decision —
//! Storage Shape). Seed helpers below write it directly via raw SQL
//! (bypassing the define endpoint), mirroring how `security-rules`' own
//! fixture reads/seeds `access_rules` directly. Slice 02 (US-02) adds real
//! `CreateDocument` coverage (`create_document`/`seed_write_access_rule_full`
//! below), which is the first caller of `SystemDb::get_write_access_rule`
//! (indirectly, via `handle_create_document`'s new composition).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
mod security_rules_common;
pub use security_rules_common::{
    assert_state_delta, mint_client_identity_token, now_unix, set_to, unchanged, universe,
    SecurityRulesAdminContext, SecurityRulesFullContext,
};

/// Directly seed a `write_access_rules` row (bypassing the define
/// endpoint) — used by AC-17-21's redefine-precondition setup, mirroring
/// `SecurityRulesAdminContext::seed_access_rule`'s identical
/// bypass-the-endpoint allowance for the read-rule table.
pub async fn seed_write_access_rule(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO write_access_rules (project_id, collection_path, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(condition_source)
    .execute(&ctx.pool)
    .await
    .expect("insert write_access_rules row");
}

/// Read the currently-stored `condition_source` for `(project_id,
/// collection_path)` in `write_access_rules` — the state-delta observable
/// for AC-17-20/21, structurally disjoint from
/// `SecurityRulesAdminContext::access_rule_condition_source` (a DIFFERENT
/// table, `FROM write_access_rules` only).
pub async fn write_access_rule_condition_source(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT condition_source FROM write_access_rules WHERE project_id = $1 AND collection_path = $2",
    )
    .bind(project_id)
    .bind(collection_path)
    .fetch_optional(&ctx.pool)
    .await
    .unwrap_or(None)
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 02 (US-02, ADR-030) — real `CreateDocument` gRPC calls against
// `SecurityRulesFullContext` (imported above), plus a `write_access_rules`
// seed helper against THAT context's `sys_pool` field (distinct from
// `SecurityRulesAdminContext::pool` above — same bypass-the-endpoint
// allowance, different context/field name).
// ─────────────────────────────────────────────────────────────────────────────

/// Directly seed a `write_access_rules` row against a `SecurityRulesFullContext`
/// (real gRPC + admin composition root) — this slice tests write-rule
/// EVALUATION at create-time, not definition (that is Slice 01's own job),
/// mirroring `SecurityRulesFullContext::seed_access_rule`'s identical
/// bypass-the-endpoint allowance for the read-rule table.
pub async fn seed_write_access_rule_full(
    ctx: &SecurityRulesFullContext,
    collection_path: &str,
    condition_source: &str,
) {
    sqlx::query(
        "INSERT INTO write_access_rules (project_id, collection_path, condition_source) \
         VALUES ($1, $2, $3)",
    )
    .bind(&ctx.project_id)
    .bind(collection_path)
    .bind(condition_source)
    .execute(&ctx.sys_pool)
    .await
    .expect("insert write_access_rules row");
}

/// A single-field string value, the shape every domain example in this
/// slice's ACs needs (`owner_id` equality) — kept minimal rather than a
/// full `serde_json`-style value builder (no other `FieldValue` shape is
/// exercised by this slice's scenarios).
pub fn string_field(value: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::StringValue(
            value.to_string(),
        )),
    }
}

/// Real gRPC `CreateDocument` call — driving port entry (Pillar 3), mirroring
/// `SecurityRulesFullContext::get_document`'s own shape exactly (real
/// `FirestoreClient`, real `authorization` + optional
/// `x-embyr-client-identity` metadata). A free function here, not a method
/// on `SecurityRulesFullContext` itself, since that struct lives in
/// `tests/security_rules/common/mod.rs` — this feature's own scope boundary
/// forbids touching any `tests/security_rules/` file.
pub async fn create_document(
    ctx: &SecurityRulesFullContext,
    collection_id: &str,
    document_id: &str,
    fields: std::collections::HashMap<String, embyr_proto::firestore::Value>,
    client_identity_token: Option<&str>,
) -> Result<tonic::Response<embyr_proto::firestore::Document>, tonic::Status> {
    use embyr_proto::firestore::{firestore_client::FirestoreClient, CreateDocumentRequest, Document};

    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(CreateDocumentRequest {
        parent: format!(
            "projects/{}/databases/(default)/documents",
            ctx.project_id
        ),
        collection_id: collection_id.to_string(),
        document_id: document_id.to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    client.create_document(request).await
}

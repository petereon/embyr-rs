//! Common test infrastructure — security-rules-operations acceptance tests
//! (Slice 01, US-01, ADR-035).
//!
//! Reuses `SecurityRulesAdminContext` from `security-rules`' own fixture
//! module via a path import (per the established `security_rules_write_path`/
//! `security_rules_collection_group_rules` reuse-chain precedent — this
//! feature extends the SAME admin surface, `POST .../access_rules`, no new
//! driving-port context class needed). `access_rules`/`define_access_rule`
//! and their own test file are never touched — only imported, read-only,
//! from here.
//!
//! `access_rule_history` is a NEW, additive table (ADR-035 § Decision —
//! Schema), fused into the existing `upsert_access_rule` transaction. The
//! helper below reads it directly via raw SQL — the real domain-observable
//! proof this slice's ACs require — ordered by `id` (the authoritative
//! ordering key, not `captured_at`, per ADR-035).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
mod security_rules_common;
pub use security_rules_common::{
    assert_state_delta, mint_client_identity_token, now_unix, set_to, unchanged, universe,
    SecurityRulesAdminContext, SecurityRulesFullContext,
};

/// One `access_rule_history` row, read directly via SQL — the port-exposed
/// observable surface for AC-17-156/157/158 (a driven-port boundary read,
/// never an internal struct field).
#[derive(Debug, Clone)]
pub struct AccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Read every `access_rule_history` row for `(project_id, collection_path)`,
/// ordered `id ASC` (oldest first — the authoritative chronological
/// ordering key, ADR-035 § Decision — Schema).
pub async fn access_rule_history_rows(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
) -> Vec<AccessRuleHistoryRow> {
    let rows: Vec<(i64, String, uuid::Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, condition_source, actor_account_id, captured_at \
         FROM access_rule_history WHERE project_id = $1 AND collection_path = $2 \
         ORDER BY id ASC",
    )
    .bind(project_id)
    .bind(collection_path)
    .fetch_all(&ctx.pool)
    .await
    .expect("query access_rule_history");

    rows.into_iter()
        .map(
            |(id, condition_source, actor_account_id, captured_at)| AccessRuleHistoryRow {
                id,
                condition_source,
                actor_account_id,
                captured_at,
            },
        )
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 04 (US-04, ADR-035) — the identical history mechanism, applied to
// `write_access_rules`. `write_access_rule_history` is a wholly separate
// table (AC-17-169's structural-independence guarantee) — mirrors
// `AccessRuleHistoryRow`/`access_rule_history_rows` above exactly.
// ─────────────────────────────────────────────────────────────────────────────

/// One `write_access_rule_history` row, read directly via SQL — the
/// port-exposed observable surface for AC-17-168/169.
#[derive(Debug, Clone)]
pub struct WriteAccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Read every `write_access_rule_history` row for `(project_id,
/// collection_path)`, ordered `id ASC` (oldest first) — mirrors
/// `access_rule_history_rows` exactly, against the independent write table.
pub async fn write_access_rule_history_rows(
    ctx: &SecurityRulesAdminContext,
    project_id: &str,
    collection_path: &str,
) -> Vec<WriteAccessRuleHistoryRow> {
    let rows: Vec<(i64, String, uuid::Uuid, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, condition_source, actor_account_id, captured_at \
         FROM write_access_rule_history WHERE project_id = $1 AND collection_path = $2 \
         ORDER BY id ASC",
    )
    .bind(project_id)
    .bind(collection_path)
    .fetch_all(&ctx.pool)
    .await
    .expect("query write_access_rule_history");

    rows.into_iter()
        .map(
            |(id, condition_source, actor_account_id, captured_at)| WriteAccessRuleHistoryRow {
                id,
                condition_source,
                actor_account_id,
                captured_at,
            },
        )
        .collect()
}

/// A single-field string value (mirrors
/// `security_rules_write_path::common::string_field` exactly) — kept as a
/// local copy rather than a cross-module import: pulling in
/// `security_rules_write_path/common/mod.rs`'s own `#[path]` inclusion of
/// `security_rules/common/mod.rs` alongside this module's identical
/// inclusion would compile the same file twice into this one test binary,
/// producing two nominally-distinct `SecurityRulesFullContext` types that
/// don't unify — so AC-17-170's real-`CreateDocument` proof uses this
/// module's own re-exported `SecurityRulesFullContext` throughout instead.
pub fn string_field(value: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::StringValue(
            value.to_string(),
        )),
    }
}

/// Real gRPC `CreateDocument` call — driving port entry (Pillar 3), mirroring
/// `security_rules_write_path::common::create_document` exactly, against
/// THIS module's own `SecurityRulesFullContext` (see `string_field` doc
/// comment above for why this is a local copy, not a cross-module import).
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

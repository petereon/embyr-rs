//! Access-control rule handlers — BC-4 Access Control (feature
//! `security-rules`, ADR-027/028/029).
//!
//! define_access_rule (POST /admin/v1/projects/:project_id/access_rules):
//!   Session auth, Owner/Admin only (US-01). Defines OR redefines
//!   (Resolution 3: idempotent upsert, the SAME action either way) the
//!   access rule for a named collection. 200 { project_id, collection_path,
//!   condition, created_at, updated_at } on success — no distinct 201 vs
//!   200 (ADR-028: the SQL statement shape does not itself distinguish
//!   first-time from redefine, so neither does the response). 400 on a
//!   condition that fails to parse — the response `reason` distinguishes
//!   `SYNTAX_ERROR` (AC-17-04) from `UNSUPPORTED_CONSTRUCT` (AC-17-03).
//!   401/403 on missing/insufficient session (AC-17-05).
//!
//! simulate_access_rule (POST /admin/v1/projects/:project_id/access_rules/simulate):
//!   Session auth, ANY role (US-05, read-only, zero writes — AC-17-18).
//!   Parses a caller-supplied CANDIDATE condition (never read from or
//!   written to `access_rules`) and evaluates it against a caller-supplied
//!   synthetic identity (or none, for the anonymous case — AC-17-19) and
//!   synthetic document payload. Calls the IDENTICAL
//!   `embyr_core::access_control::{parse_condition, evaluate}` real
//!   enforcement uses (ADR-029 § Simulation shares the exact evaluation
//!   routine) — never a second, independently-maintained copy. 200
//!   { outcome: "allow" | "deny" } on success; 400 with the same
//!   SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT taxonomy as define/redefine if the
//!   candidate condition itself fails to parse. Implemented (DELIVER,
//!   step 06-01).
//!
//! Both handlers implemented (DELIVER steps 01-01 through 06-01) — mirrors
//! `client_identity.rs`'s own doc-comment convention of marking each handler
//! "Implemented" once its RED scaffolds (`embyr_core::access_control::
//! {parse_condition,evaluate}`, `SystemDb::{upsert_access_rule,
//! get_access_rule}`) are real, tested code.

use std::collections::BTreeMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::access_control::{evaluate, parse_condition, AuthContext, ConditionParseError, EvaluationOutcome};
use embyr_core::admin::account::Role;
use embyr_core::domain::field_value::FieldValue;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Body for POST /admin/v1/projects/:project_id/access_rules.
/// `collection_path` is a single-segment collection id in v1 (Trailmark's
/// domain examples: `journal_entries`, `trail_guides`, `app_config` — no
/// subcollection paths, ADR-028 § Decision — Schema).
#[derive(Deserialize)]
pub struct DefineAccessRuleBody {
    pub collection_path: String,
    pub condition: String,
}

/// Response for POST /admin/v1/projects/:project_id/access_rules — 200,
/// either first-time definition OR redefinition (Resolution 3: same
/// response either way, no distinct 201).
#[derive(Serialize)]
pub struct AccessRuleResponse {
    pub project_id: String,
    pub collection_path: String,
    pub condition: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A synthetic identity for simulation (US-05). `None`/absent represents
/// the anonymous case (AC-17-19) — matches real evaluation's
/// `Option<AuthContext>` exactly (ADR-029 § Identity reuse).
#[derive(Deserialize)]
pub struct SimulatedAuth {
    pub uid: String,
}

/// Body for POST /admin/v1/projects/:project_id/access_rules/simulate.
/// `resource` is a flat field-name -> JSON-value map, translated to
/// `FieldValue` via `json_value_to_field_value` below — deliberately a
/// SEPARATE, ordinary utility function, not a scaffold (it is a structural
/// JSON translation, not the business logic this feature tests; see module
/// doc comment).
#[derive(Deserialize)]
pub struct SimulateAccessRuleBody {
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
}

/// Response for POST .../access_rules/simulate — 200. `outcome` is
/// `"allow"` or `"deny"`, matching exactly what real evaluation
/// (`grpc::handler::handle_get_document`) would produce for the identical
/// `(condition, auth, resource)` triple (AC-17-17).
#[derive(Serialize)]
pub struct SimulateAccessRuleResponse {
    pub outcome: &'static str,
}

/// Distinguishable rejection-reason body for a condition that fails to
/// parse (AC-17-03 vs. AC-17-04), shared by both handlers below.
#[derive(Serialize)]
struct ConditionRejectionResponse {
    reason: &'static str,
    error: String,
}

fn condition_parse_error_response(err: ConditionParseError) -> Response {
    let body = match err {
        ConditionParseError::SyntaxError { detail } => ConditionRejectionResponse {
            reason: "SYNTAX_ERROR",
            error: detail,
        },
        ConditionParseError::UnsupportedConstruct { detail, .. } => ConditionRejectionResponse {
            reason: "UNSUPPORTED_CONSTRUCT",
            error: detail,
        },
    };
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

/// Translate a simulation request's flat JSON resource-field map into the
/// `BTreeMap<String, FieldValue>` shape `evaluate()` expects. Ordinary,
/// non-scaffolded structural translation — the v1 grammar only ever
/// compares against string-uid equality and boolean literals (Resolution
/// 1), so only `String`/`Bool`/`Null` JSON shapes are meaningful; any other
/// JSON shape (number, array, object) is passed through as `FieldValue`'s
/// closest structural analog so a simulated comparison against it is well
/// -defined (never a value the v1 grammar's own comparisons would treat as
/// equal to a string uid or a bool literal), never a parse failure — a
/// malformed simulation payload should surface as a normal `deny`/`allow`
/// outcome via evaluation, not a 500.
fn json_value_to_field_value(value: &serde_json::Value) -> FieldValue {
    match value {
        serde_json::Value::Null => FieldValue::Null,
        serde_json::Value::Bool(b) => FieldValue::Boolean(*b),
        serde_json::Value::String(s) => FieldValue::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                FieldValue::Integer(i)
            } else {
                FieldValue::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::Array(items) => {
            FieldValue::Array(items.iter().map(json_value_to_field_value).collect())
        }
        serde_json::Value::Object(map) => FieldValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), json_value_to_field_value(v)))
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /admin/v1/projects/:project_id/access_rules
///
/// Owner or Admin only (AC-17-05, Viewer -> 403 — mirrors
/// `client_identity.rs::register_client_identity_credential`'s identical
/// role-gate shape).
pub async fn define_access_rule(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<DefineAccessRuleBody>,
) -> Result<Response, StatusCode> {
    // AC-17-05: Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-17-03/AC-17-04: validate the condition against the locked v1
    // grammar BEFORE storing anything (ADR-028: `condition_source` is
    // stored only after successful parse — never a raw, unvalidated string).
    // The parsed AST itself is discarded — it is re-derivable from
    // `body.condition` on every future `GetDocument` call (ADR-028 § Store
    // Source, Not AST); this parse is purely a define-time validation gate.
    if let Err(e) = parse_condition(&body.condition) {
        return Ok(condition_parse_error_response(e));
    }

    match state
        .system_db
        .upsert_access_rule(&project_id, &body.collection_path, &body.condition)
        .await
    {
        // Resolution 3 / AC-17-01/AC-17-02: the SAME response shape whether
        // this was a first-time definition or a full replacement — there is
        // no branch here that could tell the two apart, matching ADR-028's
        // single `INSERT ... ON CONFLICT ... DO UPDATE` statement shape.
        Ok(()) => Ok((
            StatusCode::OK,
            Json(AccessRuleResponse {
                project_id,
                collection_path: body.collection_path,
                condition: body.condition,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("define_access_rule upsert error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /admin/v1/projects/:project_id/access_rules/simulate
///
/// Any role (US-05, AC-17-18: read-only by construction — never calls
/// `upsert_access_rule`, never touches a live document).
pub async fn simulate_access_rule(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<SimulateAccessRuleBody>,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let condition = match parse_condition(&body.condition) {
        Ok(c) => c,
        Err(e) => return Ok(condition_parse_error_response(e)),
    };

    // AC-17-19: `body.auth` absent represents the anonymous caller,
    // identical in shape to real evaluation's `Option<AuthContext>`
    // (ADR-029 § Identity reuse) — no separate "anonymous simulation" code
    // path.
    let auth_ctx = body.auth.map(|a| AuthContext { uid: a.uid });
    let resource_fields: BTreeMap<String, FieldValue> = body
        .resource
        .iter()
        .map(|(k, v)| (k.clone(), json_value_to_field_value(v)))
        .collect();

    // ADR-029 § Simulation shares the exact evaluation routine: the SAME
    // `evaluate()` real enforcement (`grpc::handler::handle_get_document`)
    // calls — no second, independently-maintained copy anywhere.
    let outcome = evaluate(&condition, auth_ctx.as_ref(), &resource_fields);

    Ok((
        StatusCode::OK,
        Json(SimulateAccessRuleResponse {
            outcome: match outcome {
                EvaluationOutcome::Allow => "allow",
                EvaluationOutcome::Deny => "deny",
            },
        }),
    )
        .into_response())
}

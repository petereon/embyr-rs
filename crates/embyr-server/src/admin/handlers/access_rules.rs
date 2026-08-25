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
//!   Session auth, ANY role (US-05, read-only, zero writes — AC-17-18/47).
//!   Parses a caller-supplied CANDIDATE condition (never read from or
//!   written to `access_rules`/`write_access_rules`) and evaluates it
//!   against a caller-supplied synthetic identity (or none, for the
//!   anonymous case — AC-17-19/48), a synthetic `resource` (pre-write
//!   state), and a synthetic `request_resource` (proposed new state,
//!   security-rules-write-path US-07, ADR-030 — AC-17-46). Which of the two
//!   maps are populated vs. empty drives create/update/delete semantics
//!   identically to real write enforcement (Slices 02-04); `operation` is a
//!   documentation-only annotation, never consumed. Calls the IDENTICAL
//!   `embyr_core::access_control::{parse_condition, evaluate}` real
//!   enforcement (read AND write) uses (ADR-029/ADR-030 § Simulation shares
//!   the exact evaluation routine) — never a second, independently
//!   -maintained copy. 200 { outcome: "allow" | "deny" } on success; 400
//!   with the same SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT taxonomy as
//!   define/redefine if the candidate condition itself fails to parse.
//!   Implemented (DELIVER, step 06-01; extended step 07-01, US-07).
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
use embyr_core::access_control::{
    check_query_compliance, evaluate, parse_condition, AuthContext, ConditionParseError,
    EvaluationOutcome, QueryComplianceOutcome,
};
use embyr_core::admin::account::Role;
use embyr_core::domain::field_value::FieldValue;
use embyr_core::domain::query::{FieldFilter, FilterOp, QueryFilter};

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

/// Body for POST /admin/v1/projects/:project_id/write_access_rules
/// (security-rules-write-path, US-01, ADR-030). Same shape as
/// `DefineAccessRuleBody` — a distinct type, not a shared struct, since the
/// two request bodies are independently evolvable (ADR-030 § Decision —
/// Composition, "two single-purpose routes, not one route with a
/// discriminated body").
#[derive(Deserialize)]
pub struct DefineWriteAccessRuleBody {
    pub collection_path: String,
    pub condition: String,
}

/// Response for POST /admin/v1/projects/:project_id/write_access_rules —
/// 200, either first-time definition OR redefinition, mirroring
/// `AccessRuleResponse`'s shape exactly. A separate type (not shared) so it
/// can never be mistaken for — or accidentally echo — the read condition
/// (ADR-030 § Decision — Composition, "never echo the raw condition back in
/// a way that implies it's the read condition").
#[derive(Serialize)]
pub struct WriteAccessRuleResponse {
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
    /// Documentation-only (security-rules-write-path, US-07, ADR-030):
    /// `"create"|"update"|"delete"`, NOT consumed by `evaluate()`. Evaluation
    /// semantics are driven entirely by which of `resource`/`request_resource`
    /// are populated vs. empty — the same natural create/update/delete
    /// differentiation real write enforcement uses.
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
    /// NEW (security-rules-write-path, US-07, ADR-030): the proposed new
    /// document state, translated via `json_value_to_field_value` exactly
    /// like `resource` above — reused, not duplicated.
    #[serde(default)]
    pub request_resource: BTreeMap<String, serde_json::Value>,
}

/// Response for POST .../access_rules/simulate — 200. `outcome` is
/// `"allow"` or `"deny"`, matching exactly what real evaluation
/// (`grpc::handler::handle_get_document`) would produce for the identical
/// `(condition, auth, resource)` triple (AC-17-17).
#[derive(Serialize)]
pub struct SimulateAccessRuleResponse {
    pub outcome: &'static str,
}

/// Body for POST /admin/v1/projects/:project_id/access_rules/simulate_query
/// (security-rules-query-path, US-07, ADR-031). A candidate condition
/// (never read from or written to `access_rules`), a candidate identity (or
/// none, anonymous), and a candidate query filter shape — the SAME
/// `QueryFilter`-shaped input `handle_run_query` produces from a real proto
/// query, expressed here as a flat JSON list for admin-API ergonomics
/// (translated to `QueryFilter::Composite` internally via
/// `translate_query_filters` below — AND-only, mirroring the domain shape
/// exactly; no OR input accepted, consistent with the locked v1 scope).
#[derive(Deserialize)]
pub struct SimulateQueryComplianceBody {
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub query_filters: Vec<SimulatedQueryFilter>,
}

#[derive(Deserialize)]
pub struct SimulatedQueryFilter {
    pub field_path: String,
    pub op: String,
    pub value: serde_json::Value,
}

/// Response for POST .../access_rules/simulate_query — 200. GENUINELY
/// DIFFERENT CONTRACT from `SimulateAccessRuleResponse` (shape compliance,
/// not an allow/deny document-evaluation outcome) — ADR-031 § Decision —
/// Release 2 Simulation Extension. `reasons` carries one
/// `UnsatisfiedConjunct::reason_code()` per unmet conjunct, or exactly
/// `["UNSUPPORTED_RULE_SHAPE"]` for a whole-rule undecidable verdict — the
/// SAME vocabulary the gRPC rejection message embeds, never a second one.
#[derive(Serialize)]
pub struct SimulateQueryComplianceResponse {
    pub compliant: bool,
    #[serde(default)]
    pub reasons: Vec<&'static str>,
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

/// Translate `op`'s string spelling into `FilterOp`. Ordinary, non-scaffolded
/// structural translation — `check_query_compliance` only ever tests
/// `op == FilterOp::Equal` (`filter_binds_field_to_uid`), so an unrecognized
/// spelling maps to `NotEqual` (fail-closed: it can never satisfy an
/// equality-bound conjunct), never a parse failure.
fn simulated_filter_op(op: &str) -> FilterOp {
    match op {
        "==" => FilterOp::Equal,
        "!=" => FilterOp::NotEqual,
        "<" => FilterOp::LessThan,
        "<=" => FilterOp::LessThanOrEqual,
        ">" => FilterOp::GreaterThan,
        ">=" => FilterOp::GreaterThanOrEqual,
        _ => FilterOp::NotEqual,
    }
}

/// Translate a simulation request's flat `query_filters` list into the
/// `Option<QueryFilter>` shape `check_query_compliance` expects — the SAME
/// AND-only `QueryFilter::Composite` domain shape `handle_run_query`'s own
/// `translate_filter` produces from a real proto query (ADR-031 § Decision
/// — Release 2 Simulation Extension). An empty list translates to `None`
/// (unfiltered query), mirroring a real `RunQuery` with no `where` clause.
fn translate_query_filters(filters: &[SimulatedQueryFilter]) -> Option<QueryFilter> {
    let fields: Vec<QueryFilter> = filters
        .iter()
        .map(|f| {
            QueryFilter::Field(FieldFilter {
                field_path: f.field_path.clone(),
                op: simulated_filter_op(&f.op),
                value: json_value_to_field_value(&f.value),
            })
        })
        .collect();

    match fields.len() {
        0 => None,
        1 => fields.into_iter().next(),
        _ => Some(QueryFilter::Composite(fields)),
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

/// POST /admin/v1/projects/:project_id/write_access_rules
/// (security-rules-write-path, US-01, ADR-030).
///
/// Owner or Admin only (AC-17-25), mirrors `define_access_rule`'s exact
/// shape — session auth, role gate, `verify_project_ownership` reuse,
/// `parse_condition` validation before storage, same
/// `condition_parse_error_response`/SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT
/// taxonomy (AC-17-24). Storage + admin-API surface ONLY — this handler
/// never touches `access_rules` and is never called by any write-time
/// evaluation path (that is Slices 02-04, out of this slice's scope).
pub async fn define_write_access_rule(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<DefineWriteAccessRuleBody>,
) -> Result<Response, StatusCode> {
    // AC-17-25: Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-17-23/AC-17-24: validate against the SAME, unmodified locked v1
    // grammar `define_access_rule` uses — this slice adds no grammar
    // extension (`request.resource.data.<field>` is Slice 02's job).
    if let Err(e) = parse_condition(&body.condition) {
        return Ok(condition_parse_error_response(e));
    }

    match state
        .system_db
        .upsert_write_access_rule(&project_id, &body.collection_path, &body.condition)
        .await
    {
        // AC-17-20/AC-17-21: the SAME response shape whether this was a
        // first-time definition or a full replacement — mirrors
        // `define_access_rule`'s identical no-branch shape, against
        // `write_access_rules` exclusively (AC-17-22/AC-17-43: this
        // statement never reads or writes `access_rules`).
        Ok(()) => Ok((
            StatusCode::OK,
            Json(WriteAccessRuleResponse {
                project_id,
                collection_path: body.collection_path,
                condition: body.condition,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("define_write_access_rule upsert error: {e}");
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
    // security-rules-write-path (US-07, ADR-030): the caller-supplied
    // `request_resource` map (the proposed new document state), translated
    // via the SAME `json_value_to_field_value` helper as `resource` above —
    // reused, not duplicated.
    let request_resource_fields: BTreeMap<String, FieldValue> = body
        .request_resource
        .iter()
        .map(|(k, v)| (k.clone(), json_value_to_field_value(v)))
        .collect();

    // ADR-029/ADR-030 § Simulation shares the exact evaluation routine: the
    // SAME `evaluate()` real write/read enforcement calls (Slices 02-04's
    // `handle_create_document`/`handle_update_document`/
    // `handle_delete_document`, and `handle_get_document`) — no second,
    // independently-maintained copy anywhere. Which of `resource`/
    // `request_resource` are populated vs. empty drives create/update/delete
    // semantics identically to real enforcement — `body.operation` is never
    // read here.
    let outcome = evaluate(
        &condition,
        auth_ctx.as_ref(),
        &resource_fields,
        &request_resource_fields,
    );

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

/// POST /admin/v1/projects/:project_id/access_rules/simulate_query
/// (security-rules-query-path, US-07, ADR-031).
///
/// Any role (mirrors `simulate_access_rule`'s identical any-role, read-only
/// precedent — AC-17-18/47's shape, not `define_access_rule`'s Owner/Admin
/// gate). A NEW, DISTINCT sibling handler/route (ADR-031 § Decision —
/// Release 2 Simulation Extension) — never extends `simulate_access_rule`'s
/// own body/response, since this feature's response contract (admit/reject
/// PLUS which conjunct(s) failed) is a genuinely different shape. Calls the
/// SAME `check_query_compliance` function Slices 01-06 built and real
/// `handle_run_query` enforcement calls — no second, independently
/// -maintained shape-compliance implementation. Read-only by construction:
/// never calls `upsert_access_rule`/`upsert_write_access_rule`, never
/// touches a live document, never issues a real `RunQuery`.
pub async fn simulate_query_compliance(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<SimulateQueryComplianceBody>,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let condition = match parse_condition(&body.condition) {
        Ok(c) => c,
        Err(e) => return Ok(condition_parse_error_response(e)),
    };

    let auth_ctx = body.auth.map(|a| AuthContext { uid: a.uid });
    let filter = translate_query_filters(&body.query_filters);

    let outcome = check_query_compliance(&condition, filter.as_ref(), auth_ctx.as_ref());

    // AC-17-74/75: the reason vocabulary must distinguish "the query is
    // missing a required filter" from "this rule shape can't be enforced
    // for queries at all" -- collapsing both into an empty reasons list
    // (as an earlier draft of this handler did) makes the two outcomes
    // indistinguishable to the caller, defeating the whole point of
    // reporting reasons. Mirrors handler.rs::query_compliance_rejection's
    // own "UNSUPPORTED_RULE_SHAPE" vocabulary for the undecidable case.
    let (compliant, reasons): (bool, Vec<&'static str>) = match outcome {
        QueryComplianceOutcome::Admitted => (true, Vec::new()),
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts } => (
            false,
            unsatisfied_conjuncts.iter().map(|c| c.reason_code()).collect(),
        ),
        QueryComplianceOutcome::RejectedUnsupportedRuleShape => {
            (false, vec!["UNSUPPORTED_RULE_SHAPE"])
        }
    };

    Ok((
        StatusCode::OK,
        Json(SimulateQueryComplianceResponse { compliant, reasons }),
    )
        .into_response())
}

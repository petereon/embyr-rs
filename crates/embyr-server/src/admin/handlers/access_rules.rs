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
//!   Implemented (DELIVER, step 06-01; extended step 07-01, US-07;
//!   extended again security-rules-cel-parity Slice 06, US-06, ADR-062 —
//!   additive `path_variable` synthetic-document-ID field, threaded into
//!   `evaluate()`'s 5th param, mirroring `security-rules-write-path`'s own
//!   `request_resource` in-place-extension precedent).
//!
//! simulate_group_query_compliance (POST /admin/v1/projects/:project_id/access_rules/simulate_group_query):
//!   Session auth, ANY role (US-07, ADR-032, read-only — mirrors
//!   simulate_query_compliance's identical any-role precedent). A NEW,
//!   sibling handler/route, not an extension of simulate_query_compliance's
//!   own body — `group_condition: Option<String>` is genuinely different
//!   from that handler's REQUIRED `condition` (US-04's "no group rule"
//!   default is a first-class candidate scenario here). Calls the IDENTICAL
//!   `check_query_compliance` real group-query enforcement uses. Reuses
//!   `SimulateQueryComplianceResponse` verbatim. Implemented (DELIVER,
//!   security-rules-collection-group-rules, step 07-01, US-07 — LAST slice
//!   of this feature).
//!
//! simulate_routed_access_rule (POST /admin/v1/projects/:project_id/access_rules/simulate_route):
//!   Session auth, ANY role (US-06, read-only, zero writes — AC-17-231). A
//!   NEW sibling handler to `simulate_access_rule` (never an extension of
//!   it — the response contract genuinely differs, DDD-PM-9): accepts a
//!   candidate multi-segment `pattern`, a candidate `condition`, a synthetic
//!   identity/resource, and a synthetic CONCRETE `concrete_path` to route
//!   against. Reuses `path_routing::bind_ancestor` (the SAME primitive real
//!   routing's `resolve_access_rule_pattern` calls) and `evaluate()` — never
//!   a second, independently-maintained implementation of either. 200
//!   `{outcome: "allow"|"deny"|"no_matching_pattern", bindings}` — a
//!   structural non-match (AC-17-230) is a distinct 3rd outcome, never a
//!   false "deny". Implemented (DELIVER, feature
//!   security-rules-cel-path-matching, Slice 06, US-06, ADR-063 — LAST
//!   slice of this feature).
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
    check_query_compliance, evaluate, parse_condition, path_routing, rules_file, AuthContext,
    ConditionParseError, EvaluationOutcome, QueryComplianceOutcome,
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

/// One entry in a collection's access-rule history (security-rules-operations,
/// US-02, ADR-035 § Decision — Admin Surface).
#[derive(Serialize)]
pub struct AccessRuleHistoryEntry {
    pub id: i64,
    pub condition: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Response for GET /admin/v1/projects/:project_id/access_rules/:collection_path/history
/// — 200, `history` newest first, empty (never an error) when the
/// collection has never had a rule defined (AC-17-161).
#[derive(Serialize)]
pub struct AccessRuleHistoryResponse {
    pub project_id: String,
    pub collection_path: String,
    pub history: Vec<AccessRuleHistoryEntry>,
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

/// One entry in a collection's WRITE-rule history (security-rules-operations,
/// Slice 04, ADR-035 § Decision — Admin Surface) — mirrors
/// `AccessRuleHistoryEntry` exactly.
#[derive(Serialize)]
pub struct WriteAccessRuleHistoryEntry {
    pub id: i64,
    pub condition: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Response for GET
/// /admin/v1/projects/:project_id/write_access_rules/:collection_path/history
/// — 200, `history` newest first, empty (never an error) when the
/// collection has never had a write rule defined. Mirrors
/// `AccessRuleHistoryResponse` exactly.
#[derive(Serialize)]
pub struct WriteAccessRuleHistoryResponse {
    pub project_id: String,
    pub collection_path: String,
    pub history: Vec<WriteAccessRuleHistoryEntry>,
}

/// Body for POST /admin/v1/projects/:project_id/group_access_rules
/// (security-rules-collection-group-rules, US-01, ADR-032). `collection_id`
/// is a BARE collection-group identifier — never a path (validated by
/// `validate_bare_collection_id` below, AC-17-80). A distinct type from
/// `DefineAccessRuleBody`/`DefineWriteAccessRuleBody` (field-renamed
/// `collection_path` -> `collection_id` to match ADR-032 § Decision —
/// Schema's column-naming rationale).
#[derive(Deserialize)]
pub struct DefineGroupAccessRuleBody {
    pub collection_id: String,
    pub condition: String,
}

/// Response for POST /admin/v1/projects/:project_id/group_access_rules —
/// 200, either first-time definition OR redefinition, mirroring
/// `WriteAccessRuleResponse`'s shape exactly.
#[derive(Serialize)]
pub struct GroupAccessRuleResponse {
    pub project_id: String,
    pub collection_id: String,
    pub condition: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One entry in a collection-group rule's history (security-rules-operations,
/// Slice 05, ADR-035 § Decision — Admin Surface) — mirrors
/// `WriteAccessRuleHistoryEntry` exactly.
#[derive(Serialize)]
pub struct GroupAccessRuleHistoryEntry {
    pub id: i64,
    pub condition: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Response for GET
/// /admin/v1/projects/:project_id/group_access_rules/:collection_id/history
/// — 200, `history` newest first, empty (never an error) when the
/// collection-group id has never had a rule defined. Mirrors
/// `WriteAccessRuleHistoryResponse` exactly.
#[derive(Serialize)]
pub struct GroupAccessRuleHistoryResponse {
    pub project_id: String,
    pub collection_id: String,
    pub history: Vec<GroupAccessRuleHistoryEntry>,
}

/// A synthetic identity for simulation (US-05). `None`/absent represents
/// the anonymous case (AC-17-19) — matches real evaluation's
/// `Option<AuthContext>` exactly (ADR-029 § Identity reuse).
///
/// `claims` (custom-claims, US-07, ADR-034): a synthetic claims map,
/// `#[serde(default)]` empty when omitted — an omitted/empty map exercises
/// the identical fail-closed path a real caller with no minted claim hits
/// (AC-17-155, US-04's real behavior). Shared verbatim across all 3
/// simulation handlers below (ADR-034 § Call-Site Propagation) — inert for
/// `simulate_query_compliance`/`simulate_group_query_compliance` (their own
/// `check_query_compliance` catch-all rejects any claim-referencing rule
/// regardless of what `claims` carries), observable only through
/// `simulate_access_rule` (US-07's own scope).
#[derive(Deserialize)]
pub struct SimulatedAuth {
    pub uid: String,
    #[serde(default)]
    pub claims: BTreeMap<String, serde_json::Value>,
}

/// Translate a `SimulatedAuth`'s synthetic claims map into `AuthContext.claims`
/// via the shared `FieldValue::from_json_value` (embyr-core, Slice 01) —
/// the IDENTICAL conversion real enforcement's own `VerifiedEndUserIdentity.claims`
/// -> `AuthContext.claims` propagation uses (ADR-034 § Call-Site Propagation),
/// never a second, independently-maintained translation.
fn simulated_auth_to_context(auth: SimulatedAuth) -> AuthContext {
    AuthContext {
        uid: auth.uid,
        claims: auth
            .claims
            .iter()
            .map(|(k, v)| (k.clone(), FieldValue::from_json_value(v)))
            .collect(),
    }
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
    /// NEW (security-rules-cel-parity, Slice 06, US-06, ADR-062): a
    /// synthetic document ID for a path-variable-bound candidate condition
    /// (e.g. `request.path.userId`) — a simulation has no real document to
    /// derive the path variable from (§ IN Scope). `None`/absent threads
    /// through to `evaluate()`'s `path_variable_value` param exactly like
    /// every other simulated input; a candidate condition with no
    /// `PathVariable` operand ignores it entirely.
    #[serde(default)]
    pub path_variable: Option<String>,
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

/// Body for POST /admin/v1/projects/:project_id/access_rules/simulate_group_query
/// (security-rules-collection-group-rules, US-07, ADR-032). A NEW, sibling
/// request type to `SimulateQueryComplianceBody` — NOT a reuse of that type
/// with an overloaded field, since `None`/absent has a genuinely different,
/// load-bearing meaning here: `group_condition: None` simulates the US-04
/// "no collection-group rule defined" default DIRECTLY, a first-class
/// candidate scenario (AC-17-103), never an error. Reuses
/// `SimulatedAuth`/`SimulatedQueryFilter` verbatim (unchanged).
#[derive(Deserialize)]
pub struct SimulateGroupQueryComplianceBody {
    pub group_condition: Option<String>,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub query_filters: Vec<SimulatedQueryFilter>,
}

/// Distinguishable rejection-reason body for a condition that fails to
/// parse (AC-17-03 vs. AC-17-04), shared by both handlers below.
#[derive(Serialize)]
struct ConditionRejectionResponse {
    reason: &'static str,
    error: String,
}

// ---------------------------------------------------------------------------
// security-rules-cel-parity (Slice 01, US-01, ADR-062) — rules-file import.
// ---------------------------------------------------------------------------

/// Body for POST /admin/v1/projects/:project_id/access_rules/import
/// (ADR-062 § Decision — Admin Surface).
#[derive(Deserialize)]
pub struct ImportRulesFileBody {
    pub rules_file: String,
}

/// One successfully-decomposed `match` block in an import response.
#[derive(Serialize)]
pub struct ImportedBlockSummary {
    pub collection_path: String,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

/// Response for POST .../access_rules/import — 200, every block applied
/// (ADR-062 § Decision — Admin Surface).
#[derive(Serialize)]
pub struct ImportRulesFileResponse {
    pub project_id: String,
    pub imported: Vec<ImportedBlockSummary>,
}

/// One offending `match` block, echoed back verbatim from
/// `rules_file::OffendingBlock` (ADR-062 § Decision — Admin Surface).
#[derive(Serialize)]
pub struct OffendingBlockResponse {
    pub path_pattern: String,
    pub construct: &'static str,
    pub detail: String,
}

/// Response for POST .../access_rules/import on any rejection — 400, names
/// EVERY offending block (DISCUSS Resolution 2 / US-04), zero storage
/// touched (ADR-062 § Decision — Import Atomicity).
#[derive(Serialize)]
pub struct RulesFileRejectionResponse {
    pub reason: &'static str,
    pub offending_blocks: Vec<OffendingBlockResponse>,
}

fn rules_file_rejection_response(err: rules_file::RulesFileError) -> Response {
    let offending_blocks = err
        .offending_blocks
        .into_iter()
        .map(|b| OffendingBlockResponse {
            path_pattern: b.path_pattern,
            construct: b.construct,
            detail: b.detail,
        })
        .collect();
    (
        StatusCode::BAD_REQUEST,
        Json(RulesFileRejectionResponse {
            reason: "IMPORT_REJECTED",
            offending_blocks,
        }),
    )
        .into_response()
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

/// AC-17-80 (security-rules-collection-group-rules, ADR-032 § Decision —
/// Admin Surface): a collection-group id is, by construction, a bare
/// identifier, never a path. Run BEFORE `parse_condition` — an invalid
/// collection id is rejected independent of the condition's own validity.
/// Reuses `ConditionRejectionResponse` verbatim (already generic — no new
/// response type for this one new rejection reason). The DB-level `CHECK
/// (collection_id NOT LIKE '%/%')` constraint (migration 0024) is a second,
/// independent defense-in-depth layer for the same invariant — this is the
/// friendly, user-facing 400 path.
fn validate_bare_collection_id(id: &str) -> Result<(), Box<Response>> {
    if id.contains('/') {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(ConditionRejectionResponse {
                    reason: "INVALID_COLLECTION_ID",
                    error: "a collection-group id must be a bare collection identifier, not a path"
                        .to_string(),
                }),
            )
                .into_response(),
        ));
    }
    Ok(())
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
        .upsert_access_rule(
            &project_id,
            &body.collection_path,
            &body.condition,
            session.account_id,
        )
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

/// GET /admin/v1/projects/:project_id/access_rules/:collection_path/history
/// (security-rules-operations, US-02, ADR-035 § Decision — Admin Surface).
///
/// Any role (AC-17-162 — mirrors `simulate_access_rule`'s identical any-role,
/// read-only shape: `verify_project_ownership` only, no role gate). Missing/
/// invalid session is rejected 401 automatically via `SessionContext`'s
/// existing `FromRequestParts` rejection (AC-17-163) — no in-handler code
/// for that case. A collection with no rule ever defined returns 200 with
/// an empty `history` list, never an error (AC-17-161) — a natural
/// consequence of `get_access_rule_history` returning zero matching rows.
pub async fn get_access_rule_history(
    Path((project_id, collection_path)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    match state
        .system_db
        .get_access_rule_history(&project_id, &collection_path)
        .await
    {
        Ok(rows) => Ok((
            StatusCode::OK,
            Json(AccessRuleHistoryResponse {
                project_id,
                collection_path,
                history: rows
                    .into_iter()
                    .map(|r| AccessRuleHistoryEntry {
                        id: r.id,
                        condition: r.condition_source,
                        actor_account_id: r.actor_account_id,
                        captured_at: r.captured_at,
                    })
                    .collect(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("get_access_rule_history error: {e}");
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
        .upsert_write_access_rule(
            &project_id,
            &body.collection_path,
            &body.condition,
            session.account_id,
        )
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

/// GET /admin/v1/projects/:project_id/write_access_rules/:collection_path/history
/// (security-rules-operations, Slice 04, ADR-035 § Decision — Admin
/// Surface). Mirrors `get_access_rule_history`'s exact shape — any role,
/// read-only (`verify_project_ownership` only, no role gate), 401 on
/// missing/invalid session via `SessionContext`'s existing rejection, empty
/// `history` (never an error) when the collection has never had a write
/// rule defined.
pub async fn get_write_access_rule_history(
    Path((project_id, collection_path)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    match state
        .system_db
        .get_write_access_rule_history(&project_id, &collection_path)
        .await
    {
        Ok(rows) => Ok((
            StatusCode::OK,
            Json(WriteAccessRuleHistoryResponse {
                project_id,
                collection_path,
                history: rows
                    .into_iter()
                    .map(|r| WriteAccessRuleHistoryEntry {
                        id: r.id,
                        condition: r.condition_source,
                        actor_account_id: r.actor_account_id,
                        captured_at: r.captured_at,
                    })
                    .collect(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("get_write_access_rule_history error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /admin/v1/projects/:project_id/group_access_rules
/// (security-rules-collection-group-rules, US-01, ADR-032).
///
/// Owner or Admin only (mirrors `define_write_access_rule`'s exact shape —
/// session auth, role gate, `verify_project_ownership` reuse, same
/// `condition_parse_error_response`/SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT
/// taxonomy) plus ONE new step, run BEFORE `parse_condition`
/// (`validate_bare_collection_id`, AC-17-80). Storage + admin-API surface
/// ONLY — this handler never touches `access_rules`/`write_access_rules`
/// and is never called by any query-time evaluation path (that is Slices
/// 02-04, out of this slice's scope).
pub async fn define_group_access_rule(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<DefineGroupAccessRuleBody>,
) -> Result<Response, StatusCode> {
    // Owner or Admin only, mirrors define_write_access_rule/define_access_rule.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-17-80: a collection-group id must be a bare identifier, never a
    // path — validated BEFORE the condition itself (domain example: an
    // invalid collection id is rejected independent of condition validity).
    if let Err(resp) = validate_bare_collection_id(&body.collection_id) {
        return Ok(*resp);
    }

    // Validate against the SAME, unmodified locked v1 grammar
    // define_access_rule/define_write_access_rule use — this feature adds
    // no grammar extension (ADR-032 § Decision Drivers 3).
    if let Err(e) = parse_condition(&body.condition) {
        return Ok(condition_parse_error_response(e));
    }

    match state
        .system_db
        .upsert_group_access_rule(
            &project_id,
            &body.collection_id,
            &body.condition,
            session.account_id,
        )
        .await
    {
        // AC-17-77/78: the SAME response shape whether this was a
        // first-time definition or a full replacement — mirrors
        // define_write_access_rule's identical no-branch shape, against
        // group_access_rules exclusively (AC-17-79: this statement never
        // reads or writes access_rules/write_access_rules).
        Ok(()) => Ok((
            StatusCode::OK,
            Json(GroupAccessRuleResponse {
                project_id,
                collection_id: body.collection_id,
                condition: body.condition,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("define_group_access_rule upsert error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /admin/v1/projects/:project_id/group_access_rules/:collection_id/history
/// (security-rules-operations, Slice 05, ADR-035 § Decision — Admin
/// Surface). Mirrors `get_write_access_rule_history`'s exact shape — any
/// role, read-only (`verify_project_ownership` only, no role gate), 401 on
/// missing/invalid session via `SessionContext`'s existing rejection, empty
/// `history` (never an error) when the collection-group id has never had a
/// rule defined.
pub async fn get_group_access_rule_history(
    Path((project_id, collection_id)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    match state
        .system_db
        .get_group_access_rule_history(&project_id, &collection_id)
        .await
    {
        Ok(rows) => Ok((
            StatusCode::OK,
            Json(GroupAccessRuleHistoryResponse {
                project_id,
                collection_id,
                history: rows
                    .into_iter()
                    .map(|r| GroupAccessRuleHistoryEntry {
                        id: r.id,
                        condition: r.condition_source,
                        actor_account_id: r.actor_account_id,
                        captured_at: r.captured_at,
                    })
                    .collect(),
            }),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("get_group_access_rule_history error: {e}");
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
    // path. custom-claims (US-07, ADR-034): `body.auth`'s synthetic
    // `claims` map now translates into `AuthContext.claims` via
    // `simulated_auth_to_context`, giving this handler's own claim-
    // referencing simulations (AC-17-154/155) real effect.
    let auth_ctx = body.auth.map(simulated_auth_to_context);
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
    // security-rules-cel-parity (Slice 06, US-06, ADR-062): `evaluate()`'s
    // 5th parameter, now threaded from the caller-supplied synthetic
    // document ID — the identical mechanism `handle_get_document`/the 3
    // write handlers use for a real document ID (ADR-062 § Decision —
    // evaluate() signature), never a second resolution path.
    let outcome = evaluate(
        &condition,
        auth_ctx.as_ref(),
        &resource_fields,
        &request_resource_fields,
        body.path_variable.as_deref(),
        // security-rules-cel-path-matching (Slice 02, ADR-063): mechanical
        // empty-map argument — this route simulates 4a's own single-leaf-
        // variable rules only; a routed-pattern simulation is US-06
        // (Release 2, `simulate_route`), out of this slice's scope.
        &std::collections::BTreeMap::new(),
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

/// Body for POST /admin/v1/projects/:project_id/access_rules/simulate_route
/// (security-rules-cel-path-matching, Slice 06, US-06, ADR-063 § Decision —
/// Admin Surface Extensions / DDD-PM-9). A NEW, sibling request type to
/// `SimulateAccessRuleBody` — genuinely different contract: `pattern` +
/// `concrete_path` exercise ROUTING itself
/// (`embyr_core::access_control::path_routing::bind_ancestor`), not merely
/// leaf-value evaluation (see module doc comment).
#[derive(Deserialize)]
pub struct SimulateRoutedAccessRuleBody {
    /// The candidate multi-segment pattern's own full path text, e.g.
    /// `"expeditions/{expeditionId}/journal_entries/{entryId}"` — never
    /// read from or written to `access_rule_patterns`.
    pub pattern: String,
    /// The candidate condition, in already-evaluator-ready form
    /// (`request.path.<name>` for every ancestor/leaf wildcard the pattern
    /// captures) — mirrors `SimulateAccessRuleBody.condition`'s identical
    /// convention (cp06 precedent).
    pub condition: String,
    pub auth: Option<SimulatedAuth>,
    #[serde(default)]
    pub resource: BTreeMap<String, serde_json::Value>,
    /// The synthetic CONCRETE path to route against (AC-17-229) — e.g.
    /// `"expeditions/test-expedition/journal_entries/test-entry"`. NEW vs.
    /// 4a's own `SimulateAccessRuleBody.path_variable: Option<String>` — a
    /// full concrete path routing must resolve, not a single pre-known
    /// document ID.
    pub concrete_path: String,
}

/// Response for POST .../access_rules/simulate_route — 200. Genuinely
/// different contract from `SimulateAccessRuleResponse` (DDD-PM-9): a 3rd
/// `"no_matching_pattern"` outcome (AC-17-230) the 2-state
/// `SimulateAccessRuleResponse` cannot express, plus the routing's own
/// resolved bindings (AC-17-228, ancestor + leaf, name-keyed) — empty when
/// unmatched.
#[derive(Serialize)]
pub struct SimulateRoutedAccessRuleResponse {
    pub outcome: &'static str,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

/// POST /admin/v1/projects/:project_id/access_rules/simulate_route
/// (security-rules-cel-path-matching, Slice 06, US-06, LAST slice of this
/// feature, ADR-063 § Decision — Admin Surface Extensions).
///
/// Any role (mirrors `simulate_access_rule`'s identical any-role, read-only
/// shape — AC-17-231: read-only by construction, never calls
/// `upsert_access_rule_pattern`, never touches a live document or a stored
/// pattern row). Reuses `path_routing::bind_ancestor` — the SAME routing
/// primitive real enforcement's own `resolve_access_rule_pattern`
/// (`grpc::handler`) is built on — and the SAME `evaluate()` real
/// enforcement/`simulate_access_rule` use, never a second,
/// independently-maintained implementation of either (DDD-PM-9).
pub async fn simulate_routed_access_rule(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<SimulateRoutedAccessRuleBody>,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let pattern_segments = match rules_file::parse_path_segments(&body.pattern) {
        Ok(s) => s,
        Err(e) => return Ok(rules_file_rejection_response(e)),
    };
    let concrete_segments = match rules_file::parse_path_segments(&body.concrete_path) {
        Ok(s) => s,
        Err(e) => return Ok(rules_file_rejection_response(e)),
    };
    // A routed pattern requires at least one ancestor segment plus a leaf
    // (document-ID) segment — defensive shape guard, not itself an AC this
    // slice targets (every domain example supplies a well-formed 4-segment
    // pattern).
    if pattern_segments.len() < 2 || concrete_segments.len() < 2 {
        return Ok(rules_file_rejection_response(rules_file::RulesFileError {
            offending_blocks: vec![rules_file::OffendingBlock {
                path_pattern: body.pattern.clone(),
                construct: "SYNTAX_ERROR",
                detail: "a routed pattern requires at least an ancestor segment and a leaf segment"
                    .to_string(),
            }],
        }));
    }

    let (pattern_ancestor, pattern_leaf) = pattern_segments.split_at(pattern_segments.len() - 1);
    let (concrete_ancestor, concrete_leaf) =
        concrete_segments.split_at(concrete_segments.len() - 1);

    // AC-17-230: a structural ancestor mismatch is a DISTINCT 3rd outcome,
    // never a false "deny" — bindings stay empty, the condition is never
    // even parsed/evaluated. The SAME `bind_ancestor` primitive real
    // request-time routing (`resolve_access_rule_pattern`) calls.
    let Some(ancestor_bindings) = path_routing::bind_ancestor(pattern_ancestor, concrete_ancestor)
    else {
        return Ok((
            StatusCode::OK,
            Json(SimulateRoutedAccessRuleResponse {
                outcome: "no_matching_pattern",
                bindings: BTreeMap::new(),
            }),
        )
            .into_response());
    };

    let leaf_variable = match pattern_leaf.first() {
        Some(rules_file::PathSegment::Wildcard(name)) => Some(name.clone()),
        _ => None,
    };
    let leaf_value = match concrete_leaf.first() {
        Some(rules_file::PathSegment::Literal(v)) => Some(v.clone()),
        _ => None,
    };

    let condition = match parse_condition(&body.condition) {
        Ok(c) => c,
        Err(e) => return Ok(condition_parse_error_response(e)),
    };
    let auth_ctx = body.auth.map(simulated_auth_to_context);
    let resource_fields: BTreeMap<String, FieldValue> = body
        .resource
        .iter()
        .map(|(k, v)| (k.clone(), json_value_to_field_value(v)))
        .collect();
    // A routed simulation has no "proposed new document" concept, mirroring
    // `simulate_access_rule`'s own GetDocument-shaped simulation.
    let empty_request_resource: BTreeMap<String, FieldValue> = BTreeMap::new();

    // The SAME evaluate() real enforcement/simulate_access_rule call — the
    // leaf capture threads through the UNCHANGED `path_variable_value` slot
    // (ADR-062), the NEW ancestor bindings thread through the 6th
    // parameter (ADR-063), identical to real routed GetDocument evaluation.
    let outcome = evaluate(
        &condition,
        auth_ctx.as_ref(),
        &resource_fields,
        &empty_request_resource,
        leaf_value.as_deref(),
        &ancestor_bindings,
    );

    // AC-17-228: report EVERY bound variable value, ancestor and leaf alike
    // — matching the domain example's own expectation
    // (`expeditionId="test-expedition", entryId="test-entry"`).
    let mut bindings = ancestor_bindings;
    if let (Some(name), Some(value)) = (leaf_variable, leaf_value) {
        bindings.insert(name, value);
    }

    Ok((
        StatusCode::OK,
        Json(SimulateRoutedAccessRuleResponse {
            outcome: match outcome {
                EvaluationOutcome::Allow => "allow",
                EvaluationOutcome::Deny => "deny",
            },
            bindings,
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
///
/// security-rules-realtime (ADR-033 § Decision — Admin Surface, US-08): this
/// handler also models a candidate `Listen` subscription's own subscribe
/// -time compliance gate, unmodified — `realtime::listen_handler::
/// handle_add_target` calls `check_query_compliance()` with the IDENTICAL
/// `(condition, filter, auth)` input shape `handle_run_query` uses, so no
/// new route/handler/response type is needed for Listen simulation.
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

    // custom-claims (US-07, ADR-034): `claims` is inert here — any
    // claim-referencing rule is rejected by `check_query_compliance`'s
    // unchanged catch-all regardless of what this map carries (§ Decision —
    // Query-Path Safety) — translated anyway since `SimulatedAuth` is one
    // shared type across all 3 simulation handlers (ADR-034 § Call-Site
    // Propagation).
    let auth_ctx = body.auth.map(simulated_auth_to_context);
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
        QueryComplianceOutcome::Rejected {
            unsatisfied_conjuncts,
        } => (
            false,
            unsatisfied_conjuncts
                .iter()
                .map(|c| c.reason_code())
                .collect(),
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

/// POST /admin/v1/projects/:project_id/access_rules/simulate_group_query
/// (security-rules-collection-group-rules, US-07, ADR-032 §
/// `simulate_group_query_compliance` — a genuine, evaluated departure from
/// both DISCUSS's own Technical Note and ADR-031's own precedent).
///
/// Any role (mirrors `simulate_query_compliance`'s identical any-role,
/// read-only precedent — no role gate). A NEW, DISTINCT sibling
/// handler/route — never a branch bolted onto `simulate_query_compliance`'s
/// own body, since the two request contracts genuinely differ (`condition`
/// REQUIRED there vs. `group_condition` OPTIONAL here, load-bearing per
/// US-04). The RESPONSE contract is identical and reused VERBATIM —
/// `SimulateQueryComplianceResponse`, no new response type. Read-only by
/// construction: never calls `upsert_group_access_rule`, never issues a real
/// `RunQuery`, and for the `group_condition: None` arm, never even reads
/// `group_access_rules` (zero storage read, matching US-04's real default
/// and `simulate_query_compliance`'s own "never reads or writes
/// access_rules for the candidate case" discipline).
pub async fn simulate_group_query_compliance(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<SimulateGroupQueryComplianceBody>,
) -> Result<Response, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-17-103: `group_condition: None` (or omitted from the JSON body)
    // simulates US-04's real "no collection-group rule defined" default
    // DIRECTLY -- without ever touching group_access_rules, matching the
    // real handle_run_query arm's own zero-storage-read discipline for this
    // exact case.
    let Some(group_condition) = body.group_condition else {
        return Ok((
            StatusCode::OK,
            Json(SimulateQueryComplianceResponse {
                compliant: false,
                reasons: vec!["GROUP_RULE_NOT_DEFINED"],
            }),
        )
            .into_response());
    };

    let condition = match parse_condition(&group_condition) {
        Ok(c) => c,
        Err(e) => return Ok(condition_parse_error_response(e)),
    };

    // custom-claims (US-07, ADR-034): `claims` is inert here — any
    // claim-referencing rule is rejected by `check_query_compliance`'s
    // unchanged catch-all regardless of what this map carries (§ Decision —
    // Query-Path Safety) — translated anyway since `SimulatedAuth` is one
    // shared type across all 3 simulation handlers (ADR-034 § Call-Site
    // Propagation).
    let auth_ctx = body.auth.map(simulated_auth_to_context);
    let filter = translate_query_filters(&body.query_filters);

    // SAME check_query_compliance() real, group-query enforcement uses
    // (handle_run_query's all_descendants=true arm) -- no second,
    // independently-maintained shape-compliance implementation.
    let outcome = check_query_compliance(&condition, filter.as_ref(), auth_ctx.as_ref());

    let (compliant, reasons): (bool, Vec<&'static str>) = match outcome {
        QueryComplianceOutcome::Admitted => (true, Vec::new()),
        QueryComplianceOutcome::Rejected {
            unsatisfied_conjuncts,
        } => (
            false,
            unsatisfied_conjuncts
                .iter()
                .map(|c| c.reason_code())
                .collect(),
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

/// One offending block naming a colliding pattern (Slice 04, US-04, ADR-063
/// § Decision — Overlap Detection). `path_pattern` is the offending pattern
/// itself; `detail` names the OTHER pattern it structurally overlaps —
/// called once per pattern in a colliding pair so the response names BOTH
/// (AC-17-218/219/223).
fn overlap_offending_block(path_pattern: &str, other: &str) -> rules_file::OffendingBlock {
    rules_file::OffendingBlock {
        path_pattern: path_pattern.to_string(),
        construct: "PATTERN_OVERLAP",
        detail: format!("structurally overlaps pattern '{other}'"),
    }
}

/// Two-sided `PATTERN_OVERLAP` rejection, naming BOTH colliding patterns
/// (security-rules-cel-recursive-wildcards, Slice 04, US-04) — the SAME
/// `overlap_offending_block` shape 4b's own equal-length checks already
/// produce inline, factored out for reuse by this feature's own
/// recursive-wildcard-involving checks (ADR-064 § Decision — Overlap
/// Detection, Generalized: `AmbiguousOverlap` -> reject both, `PATTERN_OVERLAP`
/// reused unchanged — covers BOTH a genuine same-specificity tie and an
/// unrelated-shape overlap, Resolution 2's own single carve-out).
fn overlap_rejection(a: &str, b: &str) -> rules_file::RulesFileError {
    rules_file::RulesFileError {
        offending_blocks: vec![overlap_offending_block(a, b), overlap_offending_block(b, a)],
    }
}

/// Parses a recursive-wildcard pattern's own FIXED PREFIX text into segments
/// (US-04) — `""` (the project-wide catch-all case) is the empty prefix,
/// never passed to `rules_file::parse_path_segments` (which rejects an empty
/// match path as a plain syntax error; that error is about the OUTER `.rules`
/// grammar, not about this feature's own valid, deliberately-empty prefix
/// representation).
fn parse_recursive_prefix(fixed_prefix_pattern: &str) -> Vec<rules_file::PathSegment> {
    if fixed_prefix_pattern.is_empty() {
        Vec::new()
    } else {
        rules_file::parse_path_segments(fixed_prefix_pattern)
            .expect("fixed_prefix_pattern was produced by decompose() or a prior import")
    }
}

/// Whole-file overlap validation pass (Slice 04, US-04, ADR-063 § Decision —
/// Overlap Detection; widened for recursive-wildcard patterns by
/// `security-rules-cel-recursive-wildcards`, Slice 04, US-04, ADR-064 §
/// Decision — Overlap Detection, Generalized) — runs BEFORE any storage
/// write (`import_rules_file` calls this immediately after `decompose()`
/// succeeds), mirroring `decompose()`'s own validate-then-apply atomicity
/// discipline (DISCUSS Resolution 2): on any overlap, zero
/// `upsert_access_rule_pattern` calls are made and every existing
/// pattern/rule is left untouched (AC-17-220/AC-17-252).
///
/// 4b-vs-4b (equal-length only, ADR-063, UNCHANGED):
/// 1. **Intra-file**: pairwise `path_routing::structurally_overlap` against
///    every other multi-segment pattern in the SAME import (AC-17-219).
/// 2. **Cross-import**: `structurally_overlap` against every ALREADY-STORED
///    pattern sharing the candidate's own `(ancestor_segment_count,
///    literal_skeleton)` bucket — the SAME indexed narrowing request-time
///    routing uses (AC-17-218). A stored row whose `collection_path_pattern`
///    text is byte-identical to the candidate is skipped — that is an
///    idempotent re-import (AC-17-205/222), never an overlap.
///
/// Recursive-wildcard-involving (US-04, ADR-064, NEW): the SAME
/// `path_routing::classify_prefix_relation` — `AmbiguousOverlap` rejects both
/// (naming them via `PATTERN_OVERLAP`, reused unchanged), `Contains`/
/// `Disjoint` both import — is applied to every pair where at least one side
/// is a recursive-wildcard pattern: recursive-vs-recursive (a genuine
/// same-specificity tie, AC-17-250/251) and recursive-vs-4b (an
/// unrelated-shape overlap, AC-17-253's own "no containment relationship"
/// pass-through), both intra-file and cross-import (against
/// `list_all_access_rule_patterns` — this feature's own new, project-wide,
/// import-time-only scan; 4b's own reverse direction — a NEW 4b pattern
/// against an already-stored recursive pattern — is checked here too,
/// Decision Driver 3: one shared classification, never two independently
/// -maintained rules that could silently disagree).
async fn check_pattern_overlap(
    state: &UserAdminState,
    project_id: &str,
    decomposed: &[rules_file::DecomposedTarget],
) -> Result<Option<rules_file::RulesFileError>, StatusCode> {
    let patterns: Vec<&rules_file::DecomposedPatternRule> = decomposed
        .iter()
        .filter_map(|t| match t {
            rules_file::DecomposedTarget::MultiSegmentPattern(p) => Some(p),
            _ => None,
        })
        .collect();
    let recursive_patterns: Vec<&rules_file::DecomposedRecursivePattern> = decomposed
        .iter()
        .filter_map(|t| match t {
            rules_file::DecomposedTarget::RecursiveWildcardPattern(p) => Some(p),
            _ => None,
        })
        .collect();
    if patterns.is_empty() && recursive_patterns.is_empty() {
        return Ok(None);
    }

    let ancestors: Vec<Vec<rules_file::PathSegment>> = patterns
        .iter()
        .map(|p| {
            rules_file::parse_path_segments(&p.collection_path_pattern)
                .expect("collection_path_pattern was produced by decompose() itself")
        })
        .collect();
    let recursive_prefixes: Vec<Vec<rules_file::PathSegment>> = recursive_patterns
        .iter()
        .map(|p| parse_recursive_prefix(&p.fixed_prefix_pattern))
        .collect();

    // --- Intra-file, 4b-vs-4b (UNCHANGED) ---
    for i in 0..patterns.len() {
        for j in (i + 1)..patterns.len() {
            if path_routing::structurally_overlap(&ancestors[i], &ancestors[j]) {
                return Ok(Some(overlap_rejection(
                    &patterns[i].collection_path_pattern,
                    &patterns[j].collection_path_pattern,
                )));
            }
        }
    }

    // --- Intra-file, recursive-vs-recursive (NEW, AC-17-250/251) ---
    for i in 0..recursive_prefixes.len() {
        for j in (i + 1)..recursive_prefixes.len() {
            if path_routing::classify_prefix_relation(
                &recursive_prefixes[i],
                &recursive_prefixes[j],
            ) == path_routing::PrefixRelation::AmbiguousOverlap
            {
                return Ok(Some(overlap_rejection(
                    &recursive_patterns[i].fixed_prefix_pattern,
                    &recursive_patterns[j].fixed_prefix_pattern,
                )));
            }
        }
    }

    // --- Intra-file, recursive-vs-4b (NEW, AC-17-253's pass-through case) ---
    for (r_prefix, r_pattern) in recursive_prefixes.iter().zip(recursive_patterns.iter()) {
        for (ancestor, pattern) in ancestors.iter().zip(patterns.iter()) {
            let full_reach = path_routing::fixed_depth_full_reach(ancestor);
            if path_routing::classify_prefix_relation(r_prefix, &full_reach)
                == path_routing::PrefixRelation::AmbiguousOverlap
            {
                return Ok(Some(overlap_rejection(
                    &r_pattern.fixed_prefix_pattern,
                    &pattern.collection_path_pattern,
                )));
            }
        }
    }

    // --- Cross-import, 4b-vs-stored-4b (UNCHANGED) ---
    for (pattern, ancestor) in patterns.iter().zip(ancestors.iter()) {
        let stored = state
            .system_db
            .list_access_rule_patterns_by_skeleton(
                project_id,
                pattern.ancestor_segment_count as i16,
                &pattern.literal_skeleton,
            )
            .await
            .map_err(|e| {
                tracing::error!(
                    "import_rules_file list_access_rule_patterns_by_skeleton error: {e}"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        for row in &stored {
            if row.collection_path_pattern == pattern.collection_path_pattern {
                continue;
            }
            let row_ancestor = rules_file::parse_path_segments(&row.collection_path_pattern)
                .expect("stored collection_path_pattern was produced by decompose() itself");
            if path_routing::structurally_overlap(ancestor, &row_ancestor) {
                return Ok(Some(overlap_rejection(
                    &pattern.collection_path_pattern,
                    &row.collection_path_pattern,
                )));
            }
        }
    }

    // --- Cross-import, recursive-involving (NEW): a project-wide scan,
    // reused for BOTH directions — a new recursive pattern against every
    // already-stored pattern (either kind), AND a new 4b pattern against
    // every already-stored RECURSIVE pattern (the reverse direction,
    // ADR-064 § Decision — Overlap Detection, Generalized).
    let all_stored = state
        .system_db
        .list_all_access_rule_patterns(project_id)
        .await
        .map_err(|e| {
            tracing::error!("import_rules_file list_all_access_rule_patterns error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    for (r_prefix, r_pattern) in recursive_prefixes.iter().zip(recursive_patterns.iter()) {
        for row in &all_stored {
            if row.is_recursive && row.collection_path_pattern == r_pattern.fixed_prefix_pattern {
                continue; // idempotent re-import of the SAME recursive pattern
            }
            let row_segments = if row.is_recursive {
                parse_recursive_prefix(&row.collection_path_pattern)
            } else {
                let row_ancestor = rules_file::parse_path_segments(&row.collection_path_pattern)
                    .expect("stored collection_path_pattern was produced by decompose() itself");
                path_routing::fixed_depth_full_reach(&row_ancestor)
            };
            if path_routing::classify_prefix_relation(r_prefix, &row_segments)
                == path_routing::PrefixRelation::AmbiguousOverlap
            {
                return Ok(Some(overlap_rejection(
                    &r_pattern.fixed_prefix_pattern,
                    &row.collection_path_pattern,
                )));
            }
        }
    }

    for (ancestor, pattern) in ancestors.iter().zip(patterns.iter()) {
        let full_reach = path_routing::fixed_depth_full_reach(ancestor);
        for row in &all_stored {
            if !row.is_recursive {
                continue; // non-recursive already covered by the by-skeleton loop above
            }
            let row_prefix = parse_recursive_prefix(&row.collection_path_pattern);
            if path_routing::classify_prefix_relation(&full_reach, &row_prefix)
                == path_routing::PrefixRelation::AmbiguousOverlap
            {
                return Ok(Some(overlap_rejection(
                    &pattern.collection_path_pattern,
                    &row.collection_path_pattern,
                )));
            }
        }
    }

    Ok(None)
}

/// POST /admin/v1/projects/:project_id/access_rules/import
/// (security-rules-cel-parity, US-01, Slice 01, ADR-062 § Decision — Admin
/// Surface).
///
/// Owner or Admin only — mirrors `define_access_rule`'s exact gate (this
/// writes rules, unlike the any-role simulate handlers above).
/// `rules_file::parse_rules_file` then `rules_file::decompose` run over the
/// WHOLE file BEFORE any storage call (ADR-062 § Decision — Import
/// Atomicity): on any rejection, zero `upsert_access_rule`/
/// `upsert_write_access_rule` calls are made and every existing rule is
/// left untouched (DISCUSS Resolution 2). Only once every block validates
/// does this loop over the decomposed rules, calling the existing,
/// byte-for-byte unmodified `upsert_access_rule`/`upsert_write_access_rule`
/// once per (collection, bucket) pair — the SAME calls
/// `define_access_rule`/`define_write_access_rule` make, so re-importing an
/// unchanged file is idempotent by the identical upsert-on-conflict
/// mechanism those handlers already rely on (AC-17-176).
pub async fn import_rules_file(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<ImportRulesFileBody>,
) -> Result<Response, StatusCode> {
    // Owner or Admin only, mirrors define_access_rule/define_write_access_rule.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let blocks = match rules_file::parse_rules_file(&body.rules_file) {
        Ok(b) => b,
        Err(e) => return Ok(rules_file_rejection_response(e)),
    };
    let decomposed = match rules_file::decompose(blocks) {
        Ok(d) => d,
        Err(e) => return Ok(rules_file_rejection_response(e)),
    };

    if let Some(rejection) = check_pattern_overlap(&state, &project_id, &decomposed).await? {
        return Ok(rules_file_rejection_response(rejection));
    }

    let mut imported = Vec::with_capacity(decomposed.len());
    for target in decomposed {
        match target {
            rules_file::DecomposedTarget::SingleCollection(rule) => {
                // AC-17-195 (idempotent re-import, ADR-062): `upsert_access_rule`/
                // `upsert_write_access_rule` unconditionally append a history entry
                // on every call (security-rules-operations, ADR-035 § Decision —
                // Capture Mechanism Placement, deliberately: AC-17-166 requires even
                // a no-op hand-authored redefine to capture a new entry). That
                // per-call semantics is correct and untouched for
                // `define_access_rule`/`define_write_access_rule` — but re-importing
                // a byte-identical file must NOT spuriously grow history (AC-17-195),
                // so THIS caller skips the upsert entirely when the stored condition
                // already matches, a pure equality check specific to the import path.
                if let Some(condition) = &rule.read_condition {
                    let already_current = state
                        .system_db
                        .get_access_rule(&project_id, &rule.collection_path)
                        .await
                        .map_err(|e| {
                            tracing::error!("import_rules_file get_access_rule error: {e}");
                            StatusCode::INTERNAL_SERVER_ERROR
                        })?
                        .is_some_and(|existing| existing.condition_source == *condition);
                    if !already_current {
                        if let Err(e) = state
                            .system_db
                            .upsert_access_rule(
                                &project_id,
                                &rule.collection_path,
                                condition,
                                session.account_id,
                            )
                            .await
                        {
                            tracing::error!("import_rules_file upsert_access_rule error: {e}");
                            return Err(StatusCode::INTERNAL_SERVER_ERROR);
                        }
                    }
                }
                if let Some(condition) = &rule.write_condition {
                    let already_current = state
                        .system_db
                        .get_write_access_rule(&project_id, &rule.collection_path)
                        .await
                        .map_err(|e| {
                            tracing::error!("import_rules_file get_write_access_rule error: {e}");
                            StatusCode::INTERNAL_SERVER_ERROR
                        })?
                        .is_some_and(|existing| existing.condition_source == *condition);
                    if !already_current {
                        if let Err(e) = state
                            .system_db
                            .upsert_write_access_rule(
                                &project_id,
                                &rule.collection_path,
                                condition,
                                session.account_id,
                            )
                            .await
                        {
                            tracing::error!(
                                "import_rules_file upsert_write_access_rule error: {e}"
                            );
                            return Err(StatusCode::INTERNAL_SERVER_ERROR);
                        }
                    }
                }
                imported.push(ImportedBlockSummary {
                    collection_path: rule.collection_path,
                    read_condition: rule.read_condition,
                    write_condition: rule.write_condition,
                });
            }
            // security-rules-cel-path-matching (Slice 01, US-01, ADR-063):
            // a multi-segment pattern — mirrors the SingleCollection arm's
            // identical idempotency-check-before-upsert shape (AC-17-205),
            // against the new, structurally independent `access_rule_patterns`
            // table. `ImportedBlockSummary.collection_path` is reused
            // unchanged to carry the pattern's own ancestor text (ADR-063 §
            // Decision — Admin Surface Extensions).
            rules_file::DecomposedTarget::MultiSegmentPattern(pattern) => {
                let already_current = state
                    .system_db
                    .get_access_rule_pattern(&project_id, &pattern.collection_path_pattern)
                    .await
                    .map_err(|e| {
                        tracing::error!("import_rules_file get_access_rule_pattern error: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
                    .is_some_and(|existing| {
                        existing.leaf_variable == pattern.leaf_variable
                            && existing.read_condition == pattern.read_condition
                            && existing.write_condition == pattern.write_condition
                    });
                if !already_current {
                    if let Err(e) = state
                        .system_db
                        .upsert_access_rule_pattern(
                            &project_id,
                            &pattern.collection_path_pattern,
                            pattern.ancestor_segment_count as i16,
                            &pattern.literal_skeleton,
                            pattern.leaf_variable.as_deref(),
                            pattern.read_condition.as_deref(),
                            pattern.write_condition.as_deref(),
                            session.account_id,
                            false,
                        )
                        .await
                    {
                        tracing::error!("import_rules_file upsert_access_rule_pattern error: {e}");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                }
                imported.push(ImportedBlockSummary {
                    collection_path: pattern.collection_path_pattern,
                    read_condition: pattern.read_condition,
                    write_condition: pattern.write_condition,
                });
            }
            // security-rules-cel-recursive-wildcards (Slice 01, US-01,
            // ADR-064): a terminal, even-prefix recursive-wildcard pattern —
            // mirrors the MultiSegmentPattern arm's identical idempotency
            // -check-before-upsert shape (AC-17-235), against the SAME
            // `access_rule_patterns` table with `is_recursive: true`.
            // `get_access_rule_pattern` is not filtered by `is_recursive`
            // (ADR-064's own structural non-collision proof: a 4b ancestor's
            // rendered text is always odd-length, a recursive prefix's own
            // rendered text is always even-length, so they can never collide
            // at the same `collection_path_pattern` text) — routing/overlap
            // detection against other patterns is Slice 02/04's own concern,
            // not this slice's.
            rules_file::DecomposedTarget::RecursiveWildcardPattern(pattern) => {
                let already_current = state
                    .system_db
                    .get_access_rule_pattern(&project_id, &pattern.fixed_prefix_pattern)
                    .await
                    .map_err(|e| {
                        tracing::error!("import_rules_file get_access_rule_pattern error: {e}");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
                    .is_some_and(|existing| {
                        existing.read_condition == pattern.read_condition
                            && existing.write_condition == pattern.write_condition
                    });
                if !already_current {
                    if let Err(e) = state
                        .system_db
                        .upsert_access_rule_pattern(
                            &project_id,
                            &pattern.fixed_prefix_pattern,
                            pattern.fixed_prefix_segment_count as i16,
                            &pattern.literal_skeleton_prefix,
                            None,
                            pattern.read_condition.as_deref(),
                            pattern.write_condition.as_deref(),
                            session.account_id,
                            true,
                        )
                        .await
                    {
                        tracing::error!("import_rules_file upsert_access_rule_pattern (recursive) error: {e}");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                }
                imported.push(ImportedBlockSummary {
                    collection_path: pattern.fixed_prefix_pattern,
                    read_condition: pattern.read_condition,
                    write_condition: pattern.write_condition,
                });
            }
        }
    }

    Ok((
        StatusCode::OK,
        Json(ImportRulesFileResponse {
            project_id,
            imported,
        }),
    )
        .into_response())
}

// SCAFFOLD: true
//! `signInWithCustomToken()` bridge endpoint (US-02, ADR-026).
//!
//! POST /v1/projects/{project_id}/accounts:signInWithCustomToken
//!   Body:     { "token": "<trailmark-minted token>" }
//!   200:      { "localId": "<end_user_id>", "expiresIn": "<seconds-until-exp>" }
//!   400:      { "reason": "MISSING_TOKEN" | "MALFORMED_TOKEN" | "TOKEN_EXPIRED" | "PROJECT_MISMATCH" }
//!
//! No auth header of its own — the token presented in the body IS the
//! credential being verified (ADR-026). Calls the IDENTICAL
//! `embyr_core::client_identity::verify_client_identity_token()` used by the
//! debug-verify check (US-04) — single shared verification routine,
//! satisfying the DISCUSS-flagged shared-artifact integration risk
//! structurally (one function, two call sites).
//!
//! Transport shape (exact URL path) is provisional pending OQ-CA-01's
//! required DISTILL/DELIVER-wave empirical spike against the real Firebase
//! JS SDK (see feature-delta.md § Open Questions — client-auth). The
//! *logical* contract below (what gets verified, against what, with what
//! rejection taxonomy) is implementation-ready regardless of the spike's
//! outcome (ADR-026).

use std::sync::Arc;

use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use crate::adapters::system_db::SystemDb;

/// State for the sign-in route — only needs read access to
/// `client_identity_credentials` via `SystemDb`. Deliberately NOT
/// `UserAdminState` (this endpoint has no session/operator auth of its own —
/// ADR-026 — so it does not need any of `UserAdminState`'s other fields).
#[derive(Clone)]
pub struct SignInState {
    pub system_db: Arc<SystemDb>,
}

#[derive(Deserialize)]
pub struct SignInWithCustomTokenBody {
    pub token: String,
}

#[derive(Serialize)]
pub struct SignInSuccessResponse {
    #[serde(rename = "localId")]
    pub local_id: String,
    #[serde(rename = "expiresIn")]
    pub expires_in: String,
}

#[derive(Serialize)]
pub struct SignInFailureResponse {
    pub reason: &'static str,
}

/// POST /v1/projects/{project_id}/accounts:signInWithCustomToken
pub async fn sign_in_with_custom_token(
    Path(project_id): Path<String>,
    State(state): State<SignInState>,
    Json(body): Json<SignInWithCustomTokenBody>,
) -> Result<Json<SignInSuccessResponse>, (StatusCode, Json<SignInFailureResponse>)> {
    let _ = (project_id, body, state);
    panic!(
        "sign_in_with_custom_token: RED scaffold (DISTILL, client-auth) — not yet implemented. \
         See ADR-026 § Sign-in action: load client_identity_credentials via SystemDb, call \
         embyr_core::client_identity::verify_client_identity_token() (the SAME routine US-04's \
         debug-verify uses), 200 {{localId, expiresIn}} on success mapping AC-16-06, 400 \
         {{reason}} on failure mapping AC-16-07's four rejection reasons exactly."
    )
}

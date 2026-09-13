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

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use embyr_core::client_identity::{self, ClientIdentityVerifyError};
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
    #[serde(default)]
    pub token: Option<String>,
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

pub(crate) fn malformed_response() -> (StatusCode, Json<SignInFailureResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(SignInFailureResponse {
            reason: "MALFORMED_TOKEN",
        }),
    )
}

/// POST /v1/projects/{project_id}/accounts:signInWithCustomToken
///
/// The route pattern registered in `lib.rs`
/// (`/v1/projects/:project_id/accounts:signInWithCustomToken`) contains a
/// literal `:` inside the last path segment ("accounts:signInWithCustomToken",
/// the Firebase-style RPC method syntax). `matchit` (axum's router) treats
/// that embedded `:` as introducing a SECOND named path parameter, so the
/// route actually captures two params, not one — extracting via
/// `Path<String>` (which expects exactly one) fails with a 500 before this
/// handler body ever runs. `Path<HashMap<String, String>>` tolerates the
/// extra captured param and looks up `project_id` by name, independent of
/// capture order or count.
pub async fn sign_in_with_custom_token(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<SignInState>,
    Json(body): Json<SignInWithCustomTokenBody>,
) -> Result<Json<SignInSuccessResponse>, (StatusCode, Json<SignInFailureResponse>)> {
    let project_id = params.get("project_id").cloned().unwrap_or_default();
    // ADR-024: MISSING_TOKEN is checked before any credential row is loaded
    // or verify_client_identity_token() is called (mirrors 01-01's contract).
    let Some(token) = body.token else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(SignInFailureResponse {
                reason: "MISSING_TOKEN",
            }),
        ));
    };

    let credential_row = state
        .system_db
        .get_client_identity_credential(&project_id)
        .await
        .map_err(|_| malformed_response())?;

    // No credential row registered for this project: treated identically to
    // Malformed for sign-in purposes (no separate, fifth rejection reason).
    let Some(credential_row) = credential_row else {
        return Err(malformed_response());
    };

    let public_key_current: [u8; 32] = credential_row
        .public_key_current
        .as_slice()
        .try_into()
        .map_err(|_| malformed_response())?;
    let public_key_previous: Option<[u8; 32]> = match credential_row.public_key_previous {
        Some(bytes) => Some(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| malformed_response())?,
        ),
        None => None,
    };
    let credential = client_identity::ClientIdentityCredential {
        public_key_current,
        public_key_previous,
    };

    match client_identity::verify_client_identity_token(Some(&token), &project_id, &credential) {
        Ok(identity) => {
            let expires_in = (identity.expires_at_unix - chrono::Utc::now().timestamp()).max(0);
            Ok(Json(SignInSuccessResponse {
                local_id: identity.end_user_id,
                expires_in: expires_in.to_string(),
            }))
        }
        Err(ClientIdentityVerifyError::MissingToken) => Err((
            StatusCode::BAD_REQUEST,
            Json(SignInFailureResponse {
                reason: "MISSING_TOKEN",
            }),
        )),
        Err(ClientIdentityVerifyError::Malformed) => Err(malformed_response()),
        Err(ClientIdentityVerifyError::Expired) => Err((
            StatusCode::BAD_REQUEST,
            Json(SignInFailureResponse {
                reason: "TOKEN_EXPIRED",
            }),
        )),
        Err(ClientIdentityVerifyError::ProjectMismatch) => Err((
            StatusCode::BAD_REQUEST,
            Json(SignInFailureResponse {
                reason: "PROJECT_MISMATCH",
            }),
        )),
    }
}

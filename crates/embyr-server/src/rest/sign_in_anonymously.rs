//! `accounts:signInAnonymously()` — Maria gets a real identity with zero
//! prior credential (US-02, ADR-043 Decision 5).
//!
//! POST /v1/projects/{project_id}/accounts:signInAnonymously?key={api_key}
//!   200:   { "localId": "<uuidv4>", "idToken": "<embyr-minted token>", "expiresIn": "<seconds>" }
//!   400:   { "reason": "ANONYMOUS_AUTH_NOT_ENABLED" }
//!   401:   { "reason": "INVALID_API_KEY" }
//!
//! `?key=` is REQUIRED, structurally (AC-20-07) — anonymous sign-in is the
//! ONLY one of the four identity mechanisms presenting literally zero
//! credential of any kind; `?key=` is the only admission bar this endpoint
//! has (ADR-043 Decision 5). Verified via `SystemDb::get_project_for_auth` +
//! `embyr_core::auth::argon2::verify_api_key` directly — NO Customer DB
//! adapter is ever resolved (Resolution 3, stateless minting, ADR-043
//! Decision 4; ADR-044: no `backend_mode` gating anywhere in this handler).
//!
//! `end_user_id` is a fresh `uuid::Uuid::new_v4()` per call, never persisted
//! (AC-20-08: two calls never collide). `200`, not `201` — mirrors
//! `sign_in_with_idp`'s own "mint and go, nothing created" REST semantics.
//!
//! Dispatch mechanics: `OQ-AS-01` (whether the real Firebase JS SDK's
//! `signInAnonymously()` reuses `accounts:signUp`'s action verb) is an
//! unresolved pre-DELIVER spike (ADR-043 Decision 5) — this handler
//! implements ADR-043's own recommended DEFAULT outcome (a distinct
//! `signInAnonymously` action verb, a sixth `accounts_bridge_dispatch` match
//! arm, mirrors `signInWithIdp`'s own addition exactly) since the spike was
//! not run in this session. The handler function itself is byte-identical
//! either way — only the dispatch entry point in `lib.rs` would differ.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use embyr_core::{auth::argon2, client_identity};
use serde::{Deserialize, Serialize};

use crate::adapters::{encryption::decrypt_with_rotation, system_db::SystemDb};

/// State for `accounts:signInAnonymously` — mirrors `OAuthProviderState`'s
/// own minimality discipline (ADR-043 Decision 5's exact struct). Wired
/// into the SAME `AccountsBridgeState`/`accounts_bridge_dispatch` mechanism
/// every other `accounts:<verb>` endpoint already uses, reusing the SAME
/// `encryption_key`/`encryption_key_previous` `spawn_all_servers` already
/// threads for `OAuthProviderState` — no new composition-root parameter.
#[derive(Clone)]
pub struct AnonymousIdentityState {
    pub system_db: Arc<SystemDb>,
    pub encryption_key: [u8; 32],
    pub encryption_key_previous: Option<[u8; 32]>,
}

#[derive(Deserialize)]
pub struct SignInAnonymouslyQuery {
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Serialize)]
pub struct AnonymousSignInSuccessResponse {
    #[serde(rename = "localId")]
    pub local_id: String,
    #[serde(rename = "idToken")]
    pub id_token: String,
    #[serde(rename = "expiresIn")]
    pub expires_in: String,
}

#[derive(Serialize)]
pub struct AnonymousSignInFailureResponse {
    pub reason: &'static str,
}

fn failure(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(AnonymousSignInFailureResponse { reason })).into_response()
}

/// POST /v1/projects/{project_id}/accounts:signInAnonymously
///
/// The route pattern (`accounts:signInAnonymously`) contains a literal `:`
/// inside the last path segment — `matchit` treats it as a second named
/// path parameter, so `Path<HashMap<String, String>>` is required
/// (identical reasoning to `rest/sign_up.rs::sign_up`'s own doc comment).
pub async fn sign_in_anonymously(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<AnonymousIdentityState>,
    Query(query): Query<SignInAnonymouslyQuery>,
) -> Response {
    let project_id = params.get("project_id").cloned().unwrap_or_default();

    // AC-20-07: missing/empty api_key is rejected before any identity is minted.
    let Some(api_key) = query.key.filter(|k| !k.is_empty()) else {
        return failure(StatusCode::UNAUTHORIZED, "INVALID_API_KEY");
    };

    // AC-20-07: verify the api_key directly — no Customer DB adapter is
    // ever resolved (Resolution 3, ADR-044).
    let auth_row = match state.system_db.get_project_for_auth(&project_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return failure(StatusCode::UNAUTHORIZED, "INVALID_API_KEY"),
        Err(e) => {
            tracing::error!("sign_in_anonymously: get_project_for_auth failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let verified = match argon2::verify_api_key(api_key.as_bytes(), &auth_row.api_key_hash_current)
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("sign_in_anonymously: argon2 verify error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if !verified {
        return failure(StatusCode::UNAUTHORIZED, "INVALID_API_KEY");
    }

    // AC-20-06: anonymous auth must be enabled for this project (US-01 must
    // have run) — distinguishable from the INVALID_API_KEY failure above,
    // checked only after the api_key has already been verified.
    let signing_key_row = match state.system_db.get_anonymous_signing_key(&project_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return failure(StatusCode::BAD_REQUEST, "ANONYMOUS_AUTH_NOT_ENABLED"),
        Err(e) => {
            tracing::error!("sign_in_anonymously: get_anonymous_signing_key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-20-08: a fresh UUIDv4 per call, never persisted — stateless
    // minting (Resolution 3, ADR-043 Decision 4).
    let end_user_id = uuid::Uuid::new_v4().to_string();

    let seed_bytes = match decrypt_with_rotation(
        &state.encryption_key,
        state.encryption_key_previous.as_ref(),
        &signing_key_row.private_key_enc,
    ) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("sign_in_anonymously: decrypt signing key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let seed: [u8; 32] = match seed_bytes.as_slice().try_into() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("sign_in_anonymously: decrypted signing key seed is not 32 bytes");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // ADR-045: reuse TOKEN_TTL_SECS unchanged — no new constant, no
    // refresh-token mechanism built in this feature.
    let expires_at_unix = chrono::Utc::now().timestamp() + crate::rest::sign_up::TOKEN_TTL_SECS;
    let token = client_identity::mint_client_identity_token(
        &seed,
        &end_user_id,
        &project_id,
        expires_at_unix,
    );

    (
        StatusCode::OK,
        Json(AnonymousSignInSuccessResponse {
            local_id: end_user_id,
            id_token: token,
            expires_in: crate::rest::sign_up::TOKEN_TTL_SECS.to_string(),
        }),
    )
        .into_response()
}

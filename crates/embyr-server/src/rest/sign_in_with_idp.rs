//! `accounts:signInWithIdp()` — Maria signs in with her Google account
//! (US-02, ADR-037 Decision 6).
//!
//! POST /v1/projects/{project_id}/accounts:signInWithIdp
//!   Body:  { "idToken": "<google-issued ID token>" }
//!   200:   { "localId": "google:<sub>", "idToken": "<embyr-minted token>", "expiresIn": "<seconds>" }
//!   400:   { "reason": "MISSING_ID_TOKEN" | "GOOGLE_SIGN_IN_NOT_ENABLED"
//!                     | "INVALID_ID_TOKEN" | "ID_TOKEN_EXPIRED" | "AUDIENCE_MISMATCH" }
//!   503:   { "reason": "GOOGLE_JWKS_UNREACHABLE" }
//!
//! No `?key=` query parameter (ADR-037 Decision 6's own positive
//! consequence): neither Customer DB resolution (Resolution 3(B),
//! stateless) nor signing-key decryption (AES-256-GCM under
//! `EMBYR_ENCRYPTION_KEY`, not ECIES/`api_key`) requires it — the SIMPLEST
//! of the three Identity-track sign-in shapes.
//!
//! `400`, not `401`, for every verification-failure reason — mirrors
//! `rest/sign_in.rs::sign_in_with_custom_token`'s own established
//! convention (token-in-body-IS-the-credential, stateless), not
//! `sign_in_with_password.rs`'s 401 (that shape is oracle-protected
//! password-auth-specific). `503`, not `400`, for JWKS-unreachable — a
//! genuinely different failure class (infrastructure availability, not a
//! caller-supplied-bad-token case).

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use embyr_core::{client_identity, oauth_identity};
use serde::{Deserialize, Serialize};

use crate::adapters::{
    encryption::decrypt_with_rotation, google_jwks_cache::GoogleJwksCache, system_db::SystemDb,
};

/// State for `accounts:signInWithIdp` — mirrors `HostedIdentityState`'s own
/// minimality discipline (ADR-037 Decision 6's exact struct). Wired into
/// the SAME `AccountsBridgeState`/`accounts_bridge_dispatch` mechanism the
/// other `accounts:<verb>` endpoints already use.
#[derive(Clone)]
pub struct OAuthProviderState {
    pub system_db: Arc<SystemDb>,
    pub encryption_key: [u8; 32],
    pub encryption_key_previous: Option<[u8; 32]>,
    pub google_jwks_cache: Arc<GoogleJwksCache>,
}

#[derive(Deserialize)]
pub struct SignInWithIdpBody {
    #[serde(rename = "idToken", default)]
    pub id_token: Option<String>,
}

#[derive(Serialize)]
pub struct SignInWithIdpSuccessResponse {
    #[serde(rename = "localId")]
    pub local_id: String,
    #[serde(rename = "idToken")]
    pub id_token: String,
    #[serde(rename = "expiresIn")]
    pub expires_in: String,
}

#[derive(Serialize)]
pub struct SignInWithIdpFailureResponse {
    pub reason: &'static str,
}

fn failure(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(SignInWithIdpFailureResponse { reason })).into_response()
}

/// POST /v1/projects/{project_id}/accounts:signInWithIdp
///
/// The route pattern (`accounts:signInWithIdp`) contains a literal `:`
/// inside the last path segment — `matchit` treats it as a second named
/// path parameter, so `Path<HashMap<String, String>>` is required
/// (identical reasoning to `rest/sign_up.rs::sign_up`'s own doc comment).
pub async fn sign_in_with_idp(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<OAuthProviderState>,
    Json(body): Json<SignInWithIdpBody>,
) -> Response {
    let project_id = params.get("project_id").cloned().unwrap_or_default();

    let Some(id_token) = body.id_token.filter(|t| !t.is_empty()) else {
        return failure(StatusCode::BAD_REQUEST, "MISSING_ID_TOKEN");
    };

    // AC-19-08: Google sign-in must be registered for this project (Slice 01
    // must have run) — distinguishable from a token-validation failure.
    let provider_row = match state
        .system_db
        .get_oauth_provider_credential(&project_id)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return failure(StatusCode::BAD_REQUEST, "GOOGLE_SIGN_IN_NOT_ENABLED"),
        Err(e) => {
            tracing::error!("sign_in_with_idp: get_oauth_provider_credential failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-19-11: any JWKS fetch-failure fault class folds into the SAME
    // distinguishable, retryable-sounding 503.
    let jwks = match state.google_jwks_cache.get().await {
        Ok(j) => j,
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "GOOGLE_JWKS_UNREACHABLE"),
    };

    let verified = match oauth_identity::verify_google_id_token(
        Some(&id_token),
        &provider_row.client_id,
        &jwks,
    ) {
        Ok(v) => v,
        Err(oauth_identity::OAuthIdentityVerifyError::MissingToken) => {
            return failure(StatusCode::BAD_REQUEST, "MISSING_ID_TOKEN")
        }
        Err(oauth_identity::OAuthIdentityVerifyError::Malformed) => {
            return failure(StatusCode::BAD_REQUEST, "INVALID_ID_TOKEN")
        }
        Err(oauth_identity::OAuthIdentityVerifyError::Expired) => {
            return failure(StatusCode::BAD_REQUEST, "ID_TOKEN_EXPIRED")
        }
        Err(oauth_identity::OAuthIdentityVerifyError::AudienceMismatch) => {
            return failure(StatusCode::BAD_REQUEST, "AUDIENCE_MISMATCH")
        }
    };

    // Resolution 3(B): deterministic, stateless — no Customer DB write.
    let end_user_id = oauth_identity::derive_end_user_id("google", &verified.sub);

    // Decrypt the project's embyr-owned OAuth signing key seed — System DB
    // via SystemDb, AES-256-GCM under EMBYR_ENCRYPTION_KEY (ADR-037
    // Decision 2), NOT ECIES/api_key (unlike hosted-identity's own
    // sign_up.rs decrypt call).
    let signing_key_row = match state.system_db.get_oauth_signing_key(&project_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            // The transactional register_oauth_provider insert makes this
            // row's existence coincide exactly with oauth_provider_credentials's
            // own — reaching here means that invariant broke.
            tracing::error!(
                "sign_in_with_idp: oauth_provider_credentials row exists but \
                 oauth_signing_keys row missing for project {project_id}"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => {
            tracing::error!("sign_in_with_idp: get_oauth_signing_key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let seed_bytes = match decrypt_with_rotation(
        &state.encryption_key,
        state.encryption_key_previous.as_ref(),
        &signing_key_row.private_key_enc,
    ) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("sign_in_with_idp: decrypt signing key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let seed: [u8; 32] = match seed_bytes.as_slice().try_into() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("sign_in_with_idp: decrypted signing key seed is not 32 bytes");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let expires_at_unix = chrono::Utc::now().timestamp() + crate::rest::sign_up::TOKEN_TTL_SECS;
    let token = client_identity::mint_client_identity_token(
        &seed,
        &end_user_id,
        &project_id,
        expires_at_unix,
    );

    (
        StatusCode::OK,
        Json(SignInWithIdpSuccessResponse {
            local_id: end_user_id,
            id_token: token,
            expires_in: crate::rest::sign_up::TOKEN_TTL_SECS.to_string(),
        }),
    )
        .into_response()
}

// @US-B03 @driving_port @real-io
//! Slice B-03 — SDK Key CRUD.
//!
//! All tests are #[ignore] (RED). DELIVER unskips one at a time.
//!
//! Key invariants:
//! - SDK key plaintext returned ONCE at creation, never again (AC-B03-02)
//! - Key stored as BLAKE3 hash (AC-B03-02, security constraint)
//! - Key creation goes through RotateAuthKey command (AC-B03-03, D4 constraint)
//! - Role enforcement: Owner/Admin only for create/delete (AC-B03-05)
//!
//! Error ratio: 5 error/edge / 12 total = 42% ✓
//!
//! OQ-B01 resolution: pre-existing projects with backend_pg_dsn_enc = NULL
//! skip re-encryption silently (known limitation; test coverage in B-04).

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-01, AC-B03-02: Create SDK key, key shown once, absent from subsequent GET
// ─────────────────────────────────────────────────────────────────────────────

/// Owner creates an SDK key: 201 + {id, name, key, prefix, created_at}.
/// key = "embyr_sdk_<32chars>". prefix = first 8 chars.
/// Key absent from all subsequent GET responses.
///
/// AC-B03-01, AC-B03-02
// @US-B03 @AC-B03-01 @AC-B03-02 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn owner_creates_sdk_key_and_key_shown_once_then_absent() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Create SDK key
    let create_resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "ci-pipeline-key"}))
        .send()
        .await
        .expect("POST sdk_keys failed");

    assert_eq!(
        create_resp.status().as_u16(),
        201,
        "AC-B03-02: create SDK key must return 201"
    );

    let created: serde_json::Value = create_resp.json().await.expect("response must be JSON");

    let key = created["key"].as_str().expect("AC-B03-02: 'key' field must be present in 201 response");
    assert!(
        key.starts_with("embyr_sdk_"),
        "AC-B03-02: key must start with 'embyr_sdk_'; got: {}",
        key
    );
    assert_eq!(
        key.len(),
        "embyr_sdk_".len() + 32,
        "AC-B03-02: key must be 'embyr_sdk_' + 32 chars"
    );

    let prefix = created["prefix"].as_str().expect("AC-B03-02: 'prefix' must be present");
    assert_eq!(
        prefix.len(),
        8,
        "AC-B03-02: prefix must be 8 chars; got '{}'",
        prefix
    );

    let key_id = created["id"].as_str().expect("id must be present").to_string();

    // Subsequent GET must not include the key field (only prefix)
    let list_resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET sdk_keys failed");

    let keys: Vec<serde_json::Value> = list_resp.json().await.expect("list response must be JSON");
    let found = keys.iter().find(|k| k["id"].as_str() == Some(&key_id));

    assert!(found.is_some(), "created key must appear in list");
    let found = found.unwrap();
    assert!(
        found.get("key").is_none(),
        "AC-B03-02: 'key' field must be absent from GET responses; got: {}",
        found
    );
    assert!(
        found.get("prefix").is_some(),
        "AC-B03-02: 'prefix' must be present in GET response"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-02: BLAKE3 hash stored, plaintext never stored
// ─────────────────────────────────────────────────────────────────────────────

/// After key creation, the DB row for sdk_api_keys must have key_hash = BLAKE3(key)
/// and must NOT contain the raw key value.
///
/// AC-B03-02
// @US-B03 @AC-B03-02 @real-io @adapter-integration
#[ignore]
#[tokio::test]
async fn sdk_key_stored_as_blake3_hash_not_plaintext() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let create_resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "hash-test-key"}))
        .send()
        .await
        .expect("request failed");

    let created: serde_json::Value = create_resp.json().await.expect("response must be JSON");
    let key = created["key"].as_str().unwrap().to_string();

    // Direct DB query to verify storage.
    // Panics until B-03 implementation + AdminTestContext DB access.
    panic!(
        "Not yet implemented -- RED scaffold: verify BLAKE3 hash stored for key '{}...' in sdk_api_keys",
        &key[..16]
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-03: SDK key can authenticate a Firestore RPC
// ─────────────────────────────────────────────────────────────────────────────

/// Key generated via POST .../sdk_keys authenticates a real GetDocument gRPC call.
/// Proves the RotateAuthKey/ECIES integration is wired correctly.
///
/// AC-B03-03
// @US-B03 @AC-B03-03 @real-io @adapter-integration
#[ignore]
#[tokio::test]
async fn sdk_key_authenticates_to_firestore_grpc_rpc() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let create_resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "ecies-test-key"}))
        .send()
        .await
        .expect("request failed");

    let created: serde_json::Value = create_resp.json().await.expect("response must be JSON");
    let _sdk_key = created["key"].as_str().expect("key must be present").to_string();

    // Attempt a GetDocument RPC using the generated SDK key against the gRPC test server.
    // Panics until B-03 + gRPC integration test infrastructure is implemented.
    panic!("Not yet implemented -- RED scaffold: call GetDocument gRPC with generated SDK key")
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-04, AC-B03-01 (revoked): Revoke SDK key sets revoked_at and stays in list
// ─────────────────────────────────────────────────────────────────────────────

/// DELETE /admin/v1/projects/:id/sdk_keys/:key_id: 204.
/// Revoked key appears in list with revoked_at non-null.
/// Active-only filter (?active=true) excludes revoked keys.
///
/// AC-B03-04, AC-B03-01
// @US-B03 @AC-B03-04 @AC-B03-01 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn revoke_sdk_key_marks_revoked_and_excludes_from_active_filter() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Create a key to revoke
    let key_id = create_sdk_key(&ctx, &session_cookie, project_id, "to-revoke-key").await;

    // Revoke
    let delete_resp = ctx
        .client
        .delete(ctx.url(&format!(
            "/admin/v1/projects/{}/sdk_keys/{}",
            project_id, key_id
        )))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE sdk_key failed");

    assert_eq!(
        delete_resp.status().as_u16(),
        204,
        "AC-B03-04: revoke SDK key must return 204"
    );

    // Revoked key appears in full list (revoked_at non-null)
    let list_all_resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET sdk_keys failed");

    let all_keys: Vec<serde_json::Value> = list_all_resp.json().await.expect("response must be JSON");
    let revoked = all_keys.iter().find(|k| k["id"].as_str() == Some(&key_id));
    assert!(revoked.is_some(), "AC-B03-01: revoked key must still appear in full list");
    assert!(
        revoked.unwrap()["revoked_at"].as_str().is_some(),
        "AC-B03-01: revoked key must have revoked_at set"
    );

    // Active filter must exclude revoked key
    let list_active_resp = ctx
        .client
        .get(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys?active=true", project_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET sdk_keys?active=true failed");

    let active_keys: Vec<serde_json::Value> = list_active_resp.json().await.expect("response must be JSON");
    assert!(
        active_keys.iter().all(|k| k["id"].as_str() != Some(&key_id)),
        "AC-B03-01: active=true filter must exclude revoked key"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-05: Viewer cannot create or delete SDK keys (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Viewer role attempting POST /sdk_keys or DELETE /sdk_keys/:id → 403.
///
/// AC-B03-05
// @US-B03 @AC-B03-05 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn viewer_role_cannot_create_sdk_key() {
    let ctx = AdminTestContext::new().await;
    let viewer_cookie = sign_in_as_viewer(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &viewer_cookie)
        .json(&serde_json::json!({"name": "unauthorized-key"}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B03-05: Viewer must not create SDK keys; got {}",
        resp.status()
    );
}

/// Viewer role attempting DELETE /sdk_keys/:id → 403.
// @US-B03 @AC-B03-05 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn viewer_role_cannot_delete_sdk_key() {
    let ctx = AdminTestContext::new().await;
    let owner_cookie = sign_in_as_owner(&ctx).await;
    let viewer_cookie = sign_in_as_viewer(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    // Owner creates a key first
    let key_id = create_sdk_key(&ctx, &owner_cookie, project_id, "target-key").await;

    // Viewer tries to delete it
    let resp = ctx
        .client
        .delete(ctx.url(&format!(
            "/admin/v1/projects/{}/sdk_keys/{}",
            project_id, key_id
        )))
        .header("Cookie", &viewer_cookie)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B03-05: Viewer must not delete SDK keys; got {}",
        resp.status()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-07: Key name validation (error)
// ─────────────────────────────────────────────────────────────────────────────

/// Key name max 64 chars. Longer name → 422.
// @US-B03 @AC-B03-07 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn sdk_key_name_exceeding_64_chars_returns_422() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let long_name = "a".repeat(65);

    let resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": long_name}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "AC-B03-07: key name > 64 chars must return 422"
    );
}

/// Empty key name → 422.
// @US-B03 @AC-B03-07 @error @driving_port @real-io
#[ignore]
#[tokio::test]
async fn sdk_key_empty_name_returns_422() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    let project_id = "test-project-seeded-for-account";

    let resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": ""}))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "AC-B03-07: empty key name must return 422"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B03-06: Deleting a project cascades revocation of all SDK keys
// ─────────────────────────────────────────────────────────────────────────────

/// DELETE /admin/v1/projects/:id cascades: all sdk_api_keys for that project
/// get revoked_at = now() in the same transaction.
///
/// AC-B03-06
// @US-B03 @AC-B03-06 @driving_port @real-io
#[ignore]
#[tokio::test]
async fn deleting_project_cascades_sdk_key_revocation() {
    let ctx = AdminTestContext::new().await;
    // This test requires an operator Bearer token (DELETE /projects/:id is operator route)
    // and a session for verifying SDK key state after deletion.
    panic!(
        "Not yet implemented -- RED scaffold: \
        cascade revocation requires operator DELETE + session GET sdk_keys after deletion"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn sign_in_as_owner(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.user_email,
            "password":  ctx.user_password,
            "totp_code": ctx.totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in as owner failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in owner sign-in response")
}

/// Sign in as a Viewer role user (seeded separately in AdminTestContext).
async fn sign_in_as_viewer(ctx: &AdminTestContext) -> String {
    // Panics until AdminTestContext seeds a Viewer user.
    panic!(
        "Not yet implemented -- RED scaffold: \
        sign_in_as_viewer requires AdminTestContext to seed a Viewer role user"
    )
}

/// Helper: create an SDK key and return its ID.
async fn create_sdk_key(
    ctx: &AdminTestContext,
    session_cookie: &str,
    project_id: &str,
    name: &str,
) -> String {
    let resp = ctx
        .client
        .post(ctx.url(&format!("/admin/v1/projects/{}/sdk_keys", project_id)))
        .header("Cookie", session_cookie)
        .json(&serde_json::json!({"name": name}))
        .send()
        .await
        .expect("create_sdk_key helper: request failed");

    let json: serde_json::Value = resp.json().await.expect("create_sdk_key: response must be JSON");
    json["id"]
        .as_str()
        .expect("create_sdk_key: id must be present")
        .to_string()
}

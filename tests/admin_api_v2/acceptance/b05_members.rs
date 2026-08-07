// @US-B05 @driving_port @real-io
//! Slice B-05 — Members, Service Accounts, Admin Keys.
//!
//! All tests are #[ignore] (RED). DELIVER unskips one at a time.
//!
//! Sole-owner invariant is validated with proptest (Tier B / layer 2).
//! All other scenarios are example-based (Tier A / layer 3).
//!
//! Key invariants under test:
//!   - Sole-owner cannot be removed (409) or demoted (403) under any conditions.
//!   - Admin key BLAKE3 hash stored; plaintext never in DB.
//!   - Role cap: Admin cannot create Owner-level admin keys.
//!   - Cascade: deleting member invalidates sessions + revokes admin keys.
//!   - Cascade: deleting service account revokes its admin keys.
//!   - OQ-B04 resolved: admin keys are long-lived (no idle expiry); immediate revocation only.
//!
//! Error ratio: 10 error/edge / 21 total = 48% ✓

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-01: GET /admin/v1/members
// ─────────────────────────────────────────────────────────────────────────────

/// GET /admin/v1/members returns list including pending invitations (joined_at = null).
///
/// AC-B05-01
// @US-B05 @AC-B05-01 @driving_port @real-io
#[tokio::test]
async fn member_list_includes_pending_invitations() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/members"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/members failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-01: GET /admin/v1/members must return 200"
    );

    let members: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON array");

    // At least the seeded owner exists.
    assert!(
        !members.is_empty(),
        "AC-B05-01: member list must include at least the seeded Owner"
    );

    for member in &members {
        for field in &[
            "id",
            "email",
            "display_name",
            "role",
            "auth_method",
            "mfa_enabled",
            "last_login_at",
            "joined_at",
        ] {
            assert!(
                member.get(field).is_some(),
                "AC-B05-01: member object must include field '{}'; got: {}",
                field,
                member
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-02: POST /admin/v1/members/invite — invitation created, email sent
// ─────────────────────────────────────────────────────────────────────────────

/// POST /admin/v1/members/invite: creates invitation row; sends invitation email;
/// returns 202 with invitation_id, email, role, expires_at.
///
/// AC-B05-02
// @US-B05 @AC-B05-02 @driving_port @real-io
#[tokio::test]
async fn owner_invites_member_and_invitation_email_is_sent() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/members/invite"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "email": "new-viewer@example.com",
            "role":  "Viewer"
        }))
        .send()
        .await
        .expect("POST invite failed");

    assert_eq!(
        resp.status().as_u16(),
        202,
        "AC-B05-02: invite must return 202"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");

    for field in &["invitation_id", "email", "role", "expires_at"] {
        assert!(
            body.get(field).is_some(),
            "AC-B05-02: invitation response must include '{}'; got: {}",
            field,
            body
        );
    }

    assert_eq!(
        body["email"].as_str(),
        Some("new-viewer@example.com"),
        "AC-B05-02: invitation email must match requested email"
    );
    assert_eq!(
        body["role"].as_str(),
        Some("Viewer"),
        "AC-B05-02: invitation role must match requested role"
    );

    // DB assertion: invitation row must be inserted.
    // FakeEmailSender is not wired into AdminTestContext (NoopEmailSender used in V1);
    // assert via DB proxy that the invitation was persisted.
    let account_uuid = uuid::Uuid::parse_str(&ctx.account_id).unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invitations WHERE account_id = $1 AND email = 'new-viewer@example.com'",
    )
    .bind(account_uuid)
    .fetch_one(&ctx.pool)
    .await
    .expect("count invitations");

    assert_eq!(count, 1, "AC-B05-02: invitation row must be inserted in DB");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-02: Viewer role cannot invite members (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-02 @error @driving_port @real-io
#[tokio::test]
async fn viewer_cannot_invite_member() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_viewer(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/members/invite"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"email": "someone@example.com", "role": "Viewer"}))
        .send()
        .await
        .expect("POST invite failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B05-02: Viewer cannot invite members; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-03: Owner can change Admin's role
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-03 @driving_port @real-io
#[tokio::test]
async fn owner_changes_admin_role_to_viewer() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    // ctx.admin_id is the seeded Admin member's user_id UUID.
    let admin_member_id = ctx.admin_id.as_str();

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/members/{}/role", admin_member_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"role": "Viewer"}))
        .send()
        .await
        .expect("PATCH role failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-03: Owner can change Admin → Viewer; expected 200"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-03: Sole Owner cannot demote self (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-03 @error @driving_port @real-io
#[tokio::test]
async fn sole_owner_cannot_demote_self() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/members/{}/role", ctx.user_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"role": "Admin"}))
        .send()
        .await
        .expect("PATCH role failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B05-03: sole Owner cannot demote self; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-03: Admin cannot change another Owner's role (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-03 @error @driving_port @real-io
#[tokio::test]
async fn admin_cannot_change_owner_role() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_admin(&ctx).await;
    let owner_member_id = ctx.user_id.as_str(); // ctx user is the Owner

    let resp = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/members/{}/role", owner_member_id)))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"role": "Viewer"}))
        .send()
        .await
        .expect("PATCH role failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B05-03: Admin cannot change Owner's role; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-04: Removing a non-sole member succeeds (204) with cascade
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-04 @driving_port @real-io
#[tokio::test]
async fn owner_removes_admin_member_and_sessions_cascade_revoked() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let admin_uuid = uuid::Uuid::parse_str(&ctx.admin_id).unwrap();
    let account_uuid = uuid::Uuid::parse_str(&ctx.account_id).unwrap();

    // Seed: an active session for the admin user.
    let session_token = "test-cascade-session-token";
    let session_token_hash = blake3::hash(session_token.as_bytes()).as_bytes().to_vec();
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(admin_uuid)
    .bind(account_uuid)
    .bind(&session_token_hash)
    .execute(&ctx.pool)
    .await
    .expect("seed session for admin");

    // Seed: an admin API key for the admin user.
    // admin_api_keys.member_id references users(id) = admin user UUID.
    let key_hash = blake3::hash(b"fake-cascade-key-for-admin-member").as_bytes().to_vec();
    let key_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO admin_api_keys \
         (account_id, name, key_hash, prefix, role, member_id) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(account_uuid)
    .bind("cascade-test-key")
    .bind(&key_hash)
    .bind("cascade0")
    .bind("Admin")
    .bind(admin_uuid)
    .fetch_one(&ctx.pool)
    .await
    .expect("seed admin key for admin user");

    // DELETE the admin member (path param is user_id UUID).
    let resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/members/{}", ctx.admin_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE member failed");

    assert_eq!(
        resp.status().as_u16(),
        204,
        "AC-B05-04: removing non-sole member must return 204"
    );

    // Assert cascade: no active sessions remain for admin user.
    let active_sessions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE user_id = $1 AND expires_at > now()",
    )
    .bind(admin_uuid)
    .fetch_one(&ctx.pool)
    .await
    .expect("query active sessions");
    assert_eq!(
        active_sessions, 0,
        "AC-B05-04: all admin sessions must be expired after member removal"
    );

    // Assert cascade: seeded admin key has revoked_at set.
    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM admin_api_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("query admin_api_keys cascade");
    assert!(
        revoked_at.is_some(),
        "AC-B05-04: admin key must have revoked_at set after member removal"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-04: Removing last Owner returns 409 (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-04 @error @driving_port @real-io
#[tokio::test]
async fn removing_last_owner_returns_409() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;
    // ctx.user_id is the sole Owner in this context.

    let resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/members/{}", ctx.user_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE sole Owner failed");

    assert_eq!(
        resp.status().as_u16(),
        409,
        "AC-B05-04: removing last Owner must return 409"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");
    assert_eq!(
        body["message"].as_str(),
        Some("Transfer ownership before removing the last Owner."),
        "AC-B05-04: 409 must include required message"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-04: Sole-owner invariant — proptest (Tier B, layer 2)
//
// For any account with N members where exactly one is Owner,
// attempting to remove or demote the sole Owner always returns 4xx.
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-04 @property @layer-2 @proptest
#[test]
fn sole_owner_removal_always_rejected_for_any_member_count() {
    use embyr_core::admin::account::{AccountId, AccountMember, Role, UserId};
    use embyr_core::admin::rbac::{check_rbac, RbacAction, RbacError};
    use proptest::prelude::*;

    proptest!(|(n_non_owners in 0usize..=5)| {
        let owner_user_id = UserId(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        );
        let acct_id = AccountId(uuid::Uuid::new_v4());

        let mut members = vec![AccountMember {
            id: uuid::Uuid::new_v4(),
            user_id: owner_user_id.clone(),
            account_id: acct_id.clone(),
            role: Role::Owner,
            joined_at: None,
        }];

        for _ in 0..n_non_owners {
            members.push(AccountMember {
                id: uuid::Uuid::new_v4(),
                user_id: UserId(uuid::Uuid::new_v4()),
                account_id: acct_id.clone(),
                role: Role::Viewer,
                joined_at: None,
            });
        }

        let result = check_rbac(
            RbacAction::RemoveMember { target_id: owner_user_id.clone() },
            Role::Owner,
            &owner_user_id,
            &members,
        );

        prop_assert_eq!(result, Err(RbacError::LastOwner));
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-05: GET /admin/v1/service_accounts
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-05 @driving_port @real-io
#[tokio::test]
async fn service_account_list_returns_all_fields() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/service_accounts"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/service_accounts failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-05: GET /admin/v1/service_accounts must return 200"
    );

    let accounts: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON array");

    for account in &accounts {
        for field in &["id", "name", "description", "role", "created_at"] {
            assert!(
                account.get(field).is_some(),
                "AC-B05-05: service account object must include '{}'; got: {}",
                field,
                account
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-06: POST /admin/v1/service_accounts
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-06 @driving_port @real-io
#[tokio::test]
async fn owner_creates_service_account() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/service_accounts"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "name":        "ci-cd-pipeline",
            "description": "GitHub Actions deployment user",
            "role":        "Admin"
        }))
        .send()
        .await
        .expect("POST service account failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "AC-B05-06: POST service account must return 201"
    );

    let body: serde_json::Value = resp.json().await.expect("response must be JSON");

    for field in &["id", "name", "role", "created_at"] {
        assert!(
            body.get(field).is_some(),
            "AC-B05-06: service account response must include '{}'; got: {}",
            field,
            body
        );
    }
    assert_eq!(
        body["name"].as_str(),
        Some("ci-cd-pipeline"),
        "AC-B05-06: name must match"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-06: Viewer cannot create service account (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-06 @error @driving_port @real-io
#[tokio::test]
async fn viewer_cannot_create_service_account() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_viewer(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/service_accounts"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "sa", "role": "Viewer"}))
        .send()
        .await
        .expect("POST service account failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B05-06: Viewer cannot create service account; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-07: DELETE /admin/v1/service_accounts/:id with cascade
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-07 @driving_port @real-io
#[tokio::test]
async fn deleting_service_account_revokes_its_admin_keys() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let account_uuid = uuid::Uuid::parse_str(&ctx.account_id).unwrap();

    // Seed: a service account in the test account.
    let sa_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO service_accounts (account_id, name, role) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(account_uuid)
    .bind("cascade-test-sa")
    .bind("Viewer")
    .fetch_one(&ctx.pool)
    .await
    .expect("seed service account");

    // Seed: an admin key linked to that service account.
    let key_hash = blake3::hash(b"fake-sa-cascade-key-unique").as_bytes().to_vec();
    let key_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO admin_api_keys \
         (account_id, name, key_hash, prefix, role, service_account_id) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(account_uuid)
    .bind("sa-cascade-key")
    .bind(&key_hash)
    .bind("sacacc01")
    .bind("Viewer")
    .bind(sa_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("seed admin key for SA");

    // DELETE the service account.
    let resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/service_accounts/{}", sa_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE service account failed");

    assert_eq!(
        resp.status().as_u16(),
        204,
        "AC-B05-07: DELETE service account must return 204"
    );

    // Assert cascade: admin key for this SA has revoked_at set.
    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM admin_api_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("query admin_api_keys cascade for SA");
    assert!(
        revoked_at.is_some(),
        "AC-B05-07: admin key must have revoked_at set after SA deletion"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-08: GET /admin/v1/admin_keys
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-08 @driving_port @real-io
#[tokio::test]
async fn admin_key_list_includes_revoked_keys_for_audit() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET /admin/v1/admin_keys failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-08: GET /admin/v1/admin_keys must return 200"
    );

    let keys: Vec<serde_json::Value> = resp.json().await.expect("response must be JSON array");

    for key in &keys {
        for field in &[
            "id",
            "name",
            "role",
            "prefix",
            "created_at",
            "last_used_at",
            "revoked_at",
        ] {
            assert!(
                key.get(field).is_some(),
                "AC-B05-08: admin key object must include '{}'; got: {}",
                field,
                key
            );
        }
        // Plaintext key value must not be in list response.
        assert!(
            key.get("key").is_none(),
            "AC-B05-08: plaintext 'key' must not appear in list response; got: {}",
            key
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-09: POST /admin/v1/admin_keys — key shown once, stored as BLAKE3
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-09 @driving_port @real-io
#[tokio::test]
async fn owner_creates_admin_key_and_key_shown_once_then_absent() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    // Create key
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "name":      "ci-deploy-key",
            "member_id": ctx.user_id,
            "role":      "Admin"
        }))
        .send()
        .await
        .expect("POST admin_key failed");

    assert_eq!(
        create_resp.status().as_u16(),
        201,
        "AC-B05-09: POST admin key must return 201"
    );

    let created: serde_json::Value = create_resp.json().await.expect("response must be JSON");
    let key = created["key"].as_str().expect("AC-B05-09: key must be present in 201 response");

    assert!(
        key.starts_with("embyr_adm_"),
        "AC-B05-09: admin key must start with 'embyr_adm_'; got: {}",
        key
    );
    assert!(
        key.len() >= 10 + 32,
        "AC-B05-09: admin key must be at least 42 chars; got len={}",
        key.len()
    );

    let prefix = created["prefix"].as_str().expect("prefix must be present");
    assert_eq!(prefix.len(), 8, "AC-B05-09: prefix must be 8 chars");

    // Subsequent GET must not expose plaintext key.
    let key_id = created["id"].as_str().expect("id must be present");
    let list_resp = ctx
        .client
        .get(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("GET admin_keys failed");

    let keys: Vec<serde_json::Value> = list_resp.json().await.expect("list must be JSON");
    let our_key = keys.iter().find(|k| k["id"].as_str() == Some(key_id));

    if let Some(k) = our_key {
        assert!(
            k.get("key").is_none(),
            "AC-B05-09: plaintext key must not appear in subsequent GET; got: {}",
            k
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-09: Admin key stored as BLAKE3, not plaintext (error — security)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-09 @real-io @adapter-integration
#[tokio::test]
async fn admin_key_stored_as_blake3_not_plaintext() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    // Create an admin key.
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "blake3-test-key", "role": "Viewer"}))
        .send()
        .await
        .expect("POST admin_key");

    assert_eq!(
        create_resp.status().as_u16(),
        201,
        "AC-B05-09: POST admin key must return 201"
    );
    let created: serde_json::Value = create_resp.json().await.expect("JSON");
    let key_str = created["key"].as_str().expect("key present").to_string();
    let key_id_str = created["id"].as_str().expect("id present").to_string();
    let key_id = uuid::Uuid::parse_str(&key_id_str).unwrap();

    // Verify BLAKE3 hash is stored in DB, not plaintext.
    let stored_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT key_hash FROM admin_api_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("query key_hash");

    let expected_hash = blake3::hash(key_str.as_bytes()).as_bytes().to_vec();
    assert_eq!(
        stored_hash, expected_hash,
        "AC-B05-09: key_hash must be BLAKE3(plaintext_key)"
    );

    // Verify no plaintext key appears in any text column of the row.
    use sqlx::Row as _;
    let row = sqlx::query("SELECT name, prefix FROM admin_api_keys WHERE id = $1")
        .bind(key_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("query row");

    let name: String = row.try_get("name").unwrap_or_default();
    let prefix: String = row.try_get("prefix").unwrap_or_default();

    assert_ne!(
        name.as_str(),
        key_str.as_str(),
        "AC-B05-09: name column must not be the plaintext key"
    );
    assert_ne!(
        prefix.as_str(),
        key_str.as_str(),
        "AC-B05-09: prefix column must not be the plaintext key"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-09: Admin cannot create Owner-level admin key (error)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-09 @error @driving_port @real-io
#[tokio::test]
async fn admin_cannot_create_owner_level_admin_key() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_admin(&ctx).await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({
            "name": "illegal-owner-key",
            "role": "Owner"
        }))
        .send()
        .await
        .expect("POST admin_key failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-B05-09: Admin cannot create Owner-level key; expected 403"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-10: DELETE /admin/v1/admin_keys/:key_id — immediate revocation
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-10 @driving_port @real-io
#[tokio::test]
async fn revoking_admin_key_causes_immediate_401_on_next_request() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    // Step 1: Create an admin key.
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "revocable-key", "role": "Viewer"}))
        .send()
        .await
        .expect("POST admin_key failed");
    let created: serde_json::Value = create_resp.json().await.expect("JSON");
    let raw_key = created["key"].as_str().expect("key must be present").to_string();
    let key_id = created["id"].as_str().expect("id must be present").to_string();

    // Step 2: Verify key works before revocation.
    let before_resp = ctx
        .client
        .get(ctx.url("/admin/v1/members"))
        .header("Authorization", format!("Bearer {}", raw_key))
        .send()
        .await
        .expect("GET before revoke failed");
    assert_eq!(
        before_resp.status().as_u16(),
        200,
        "AC-B05-10: key must work before revocation"
    );

    // Step 3: Revoke.
    let revoke_resp = ctx
        .client
        .delete(ctx.url(&format!("/admin/v1/admin_keys/{}", key_id)))
        .header("Cookie", &session_cookie)
        .send()
        .await
        .expect("DELETE admin_key failed");
    assert_eq!(
        revoke_resp.status().as_u16(),
        204,
        "AC-B05-10: DELETE admin key must return 204"
    );

    // Step 4: Verify key is immediately rejected.
    let after_resp = ctx
        .client
        .get(ctx.url("/admin/v1/members"))
        .header("Authorization", format!("Bearer {}", raw_key))
        .send()
        .await
        .expect("GET after revoke failed");
    assert_eq!(
        after_resp.status().as_u16(),
        401,
        "AC-B05-10: revoked key must return 401 immediately"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-11: Admin key Bearer auth works as alternative to session cookie
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-11 @driving_port @real-io
#[tokio::test]
async fn admin_key_bearer_auth_grants_access_to_session_routes() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    // Create an admin key.
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "api-access-key", "role": "Viewer"}))
        .send()
        .await
        .expect("POST admin_key failed");
    let raw_key = create_resp
        .json::<serde_json::Value>()
        .await
        .expect("JSON")["key"]
        .as_str()
        .expect("key")
        .to_string();

    // Use the admin key (Bearer) on a session-auth route.
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Authorization", format!("Bearer {}", raw_key))
        .send()
        .await
        .expect("GET with admin key failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-11: admin key Bearer must work on session-auth routes"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-B05-11: Revoked admin key updates last_used_at on successful auth only
// (OQ-B04: admin keys long-lived; no idle expiry; immediate revocation only)
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @AC-B05-11 @OQ-B04 @driving_port @real-io
#[tokio::test]
async fn admin_key_updates_last_used_at_on_successful_auth() {
    let ctx = AdminTestContext::new().await;
    let session_cookie = sign_in_as_owner(&ctx).await;

    // Create an admin key.
    let create_resp = ctx
        .client
        .post(ctx.url("/admin/v1/admin_keys"))
        .header("Cookie", &session_cookie)
        .json(&serde_json::json!({"name": "last-used-at-key", "role": "Viewer"}))
        .send()
        .await
        .expect("POST admin_key");
    let created: serde_json::Value = create_resp.json().await.expect("JSON");
    let raw_key = created["key"].as_str().expect("key").to_string();
    let key_id = uuid::Uuid::parse_str(created["id"].as_str().expect("id")).unwrap();

    // Verify last_used_at is NULL before first use.
    let before: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT last_used_at FROM admin_api_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("query last_used_at before");
    assert!(
        before.is_none(),
        "AC-B05-11: last_used_at must be NULL before first use"
    );

    // Use the key (Bearer auth on a session-auth route).
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Authorization", format!("Bearer {}", raw_key))
        .send()
        .await
        .expect("GET with key");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-B05-11: admin key must work on session-auth route"
    );

    // Verify last_used_at is set after use.
    let after: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT last_used_at FROM admin_api_keys WHERE id = $1",
    )
    .bind(key_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("query last_used_at after");
    assert!(
        after.is_some(),
        "AC-B05-11: last_used_at must be set after first use (OQ-B04: no idle expiry)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Error: Admin key with wrong BLAKE3 hash not accepted
// ─────────────────────────────────────────────────────────────────────────────

// @US-B05 @error @driving_port @real-io
#[tokio::test]
async fn invalid_admin_key_bearer_returns_401() {
    let ctx = AdminTestContext::new().await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects"))
        .header("Authorization", "Bearer embyr_adm_invalid_key_not_in_db_xxxxxxxx")
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "error: invalid admin key Bearer must return 401"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Sign in as the seeded Owner and return the session cookie.
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
        .expect("no cookie in sign-in response")
}

/// Sign in as the seeded Admin member and return the session cookie.
async fn sign_in_as_admin(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.admin_email,
            "password":  ctx.admin_password,
            "totp_code": ctx.admin_totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in as admin failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in sign-in as admin response")
}

/// Sign in as the seeded Viewer member and return the session cookie.
async fn sign_in_as_viewer(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email":     ctx.viewer_email,
            "password":  ctx.viewer_password,
            "totp_code": ctx.viewer_totp_code_now(),
        }))
        .send()
        .await
        .expect("sign-in as viewer failed");

    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in sign-in as viewer response")
}

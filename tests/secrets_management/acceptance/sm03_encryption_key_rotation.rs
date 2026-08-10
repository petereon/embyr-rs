//! US-SM-03 — Rotate EMBYR_ENCRYPTION_KEY without orphaning existing encrypted data.
//!
//! Acceptance criteria verified here:
//!   AC-SM-03-01: `EMBYR_ENCRYPTION_KEY_PREVIOUS` (optional) accepts a second
//!                64-hex-char key, validated identically to `EMBYR_ENCRYPTION_KEY`.
//!   AC-SM-03-02: TOTP decrypt tries current key first, falls back to previous on
//!                AEAD authentication failure.
//!   AC-SM-03-03: current-then-previous key selection is a SINGLE shared function
//!                (`adapters::encryption::decrypt_with_rotation`, ADR-018 §5).
//!   AC-SM-03-04: absent `EMBYR_ENCRYPTION_KEY_PREVIOUS` → byte-for-byte identical
//!                to today (single-key decrypt).
//!   AC-SM-03-05: a ciphertext failing AEAD auth under BOTH keys returns the
//!                existing decrypt-failure behavior, never a false-positive success.
//!   AC-SM-03-06: `EMBYR_ENCRYPTION_KEY_PREVIOUS` == `EMBYR_ENCRYPTION_KEY` is a
//!                startup config error.
//!
//! Driving ports:
//!   - `POST /admin/v1/auth/signin` (real subprocess + real Postgres) for the
//!     HTTP-observable rotation-window scenarios (scenarios 1-3, 7).
//!   - `adapters::encryption::decrypt_with_rotation(...)` directly for the
//!     two scenarios exercising ciphertext shapes with NO live decrypt
//!     consumer today (`client_secret_enc`, `backend_pg_dsn_enc` — DISCUSS
//!     codebase-verification note) — the function's public signature IS the
//!     driving port for a pure domain-shaped helper (port-to-port at this scope).
//!
//! Assertion mode: HTTP-response scenarios use `assert_state_delta` (Mandate 8,
//! subprocess/FS-acceptance layer) with a Universe of port-exposed observables
//! (`response.signin.status_code`). The two direct-call scenarios are
//! `@in-memory` (layer 1-2 shape) and use plain `assert!`/`matches!` — a single
//! Result value has no meaningful "before/after" delta to declare.
//!
//! Scaffold classification target:
//!   - HTTP scenarios: RED once `EMBYR_ENCRYPTION_KEY_PREVIOUS` exists and
//!     `auth.rs:255` calls `decrypt_with_rotation` (today: sign-in ignores any
//!     `_PREVIOUS` var, so rotation-window scenarios fail for the right reason).
//!   - Direct-call scenarios: RED via the `adapters::encryption` scaffold's
//!     `panic!` body (Mandate 7).

use std::collections::HashMap;
use std::time::Duration;

use embyr_server::adapters::encryption::{decrypt_with_rotation, RotationDecryptError};

use crate::common::{
    assert_state_delta, corrupt_totp_secret, encrypt_totp_secret, seed_totp_user, set_to,
    start_postgres_container, totp_code_now, universe, ServerProcess, TEST_ENCRYPTION_KEY,
    TEST_ENCRYPTION_KEY_PREVIOUS,
};

fn hex_to_key(hex: &str) -> [u8; 32] {
    let bytes = hex::decode(hex).expect("valid hex");
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    key
}

// ─── AC-SM-03-04: behavior unchanged when no previous key is configured ─────

/// Existing TOTP secret decrypts via the current key when no rotation window
/// is open — byte-for-byte identical to today's single-key decrypt.
///
/// Journey:
///   Given: EMBYR_ENCRYPTION_KEY_PREVIOUS is not set
///   When:  an existing user signs in with a TOTP secret encrypted under the current key
///   Then:  sign-in succeeds using single-key decrypt, identical to pre-feature behavior
///
/// @real-io @US-SM-03 @AC-SM-03-04
#[tokio::test]
async fn totp_signin_decrypts_with_current_key() {
    let (_pg, db_url) = start_postgres_container().await;

    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect");
    system_db.migrate().await.expect("migrate");
    let pool = system_db.pool().clone();

    let current_key = hex_to_key(TEST_ENCRYPTION_KEY);
    let (_account_id, _user_id, totp_raw, password) =
        seed_totp_user(&pool, "current-key-signin@example.com", &current_key).await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            // EMBYR_ENCRYPTION_KEY_PREVIOUS deliberately absent.
        ],
    );
    assert!(
        server.wait_for_healthy(Duration::from_secs(30)).await,
        "server must start"
    );

    let client = reqwest::Client::new();
    let before: HashMap<&str, String> = HashMap::new();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/auth/signin",
            server.admin_port
        ))
        .json(&serde_json::json!({
            "email": "current-key-signin@example.com",
            "password": password,
            "totp_code": totp_code_now(&totp_raw),
        }))
        .send()
        .await
        .expect("POST /admin/v1/auth/signin");
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(universe::SIGNIN_STATUS, resp.status().as_u16().to_string());

    let mut expected = HashMap::new();
    expected.insert(universe::SIGNIN_STATUS, set_to("200".to_string()));
    assert_state_delta(
        &before,
        &after,
        &[universe::SIGNIN_STATUS],
        &expected,
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-03-02: the core rotation scenario ─────────────────────────────────

/// Existing TOTP secret (encrypted under the OLD key) decrypts via the
/// previous-key fallback during the rotation window.
///
/// Journey (chained from scenario 1's Given + When shape — real Postgres +
/// real subprocess + POST /admin/v1/auth/signin; here the ciphertext is
/// encrypted under the RETIRING key and the server configures BOTH keys):
///   Given: Maria enrolled TOTP before rotation; her totp_secret_enc is encrypted under the OLD key
///   And:   EMBYR_ENCRYPTION_KEY is the NEW key and EMBYR_ENCRYPTION_KEY_PREVIOUS is the OLD key
///   When:  Maria signs in with her existing authenticator app code
///   Then:  sign-in succeeds
///
/// @real-io @US-SM-03 @AC-SM-03-02
#[tokio::test]
async fn totp_signin_decrypts_with_previous_key_during_rotation_window() {
    let (_pg, db_url) = start_postgres_container().await;

    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect");
    system_db.migrate().await.expect("migrate");
    let pool = system_db.pool().clone();

    let old_key = hex_to_key(TEST_ENCRYPTION_KEY_PREVIOUS);
    let (_account_id, _user_id, totp_raw, password) =
        seed_totp_user(&pool, "maria.santos@example.com", &old_key).await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS", TEST_ENCRYPTION_KEY_PREVIOUS),
        ],
    );
    assert!(
        server.wait_for_healthy(Duration::from_secs(30)).await,
        "server must start with a dual-key rotation window open"
    );

    let client = reqwest::Client::new();
    let before: HashMap<&str, String> = HashMap::new();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/auth/signin",
            server.admin_port
        ))
        .json(&serde_json::json!({
            "email": "maria.santos@example.com",
            "password": password,
            "totp_code": totp_code_now(&totp_raw),
        }))
        .send()
        .await
        .expect("POST /admin/v1/auth/signin");
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(universe::SIGNIN_STATUS, resp.status().as_u16().to_string());

    let mut expected = HashMap::new();
    expected.insert(universe::SIGNIN_STATUS, set_to("200".to_string()));
    assert_state_delta(&before, &after, &[universe::SIGNIN_STATUS], &expected);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-03-05: both keys fail — genuine authentication failure ───────────

/// A ciphertext encrypted under neither the current nor the previous key
/// returns the existing decrypt-failure outcome — never a false-positive
/// "successful" decrypt (AEAD authentication-tag guarantee).
///
/// Journey (error path):
///   Given: a totp_secret_enc value is well-formed but encrypted under a third, unconfigured key
///   And:   EMBYR_ENCRYPTION_KEY_PREVIOUS is configured
///   When:  the affected user attempts to sign in with a TOTP code
///   Then:  the response is the same decrypt-failure outcome the system returns today
///
/// @error @real-io @US-SM-03 @AC-SM-03-05
#[tokio::test]
async fn totp_signin_fails_when_neither_current_nor_previous_key_decrypts() {
    let (_pg, db_url) = start_postgres_container().await;

    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect");
    system_db.migrate().await.expect("migrate");
    let pool = system_db.pool().clone();

    // Encrypted under a THIRD key, unrelated to either configured key.
    let neither_key = hex_to_key(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let (_account_id, _user_id, totp_raw, password) =
        seed_totp_user(&pool, "unrecoverable@example.com", &neither_key).await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS", TEST_ENCRYPTION_KEY_PREVIOUS),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let before: HashMap<&str, String> = HashMap::new();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/auth/signin",
            server.admin_port
        ))
        .json(&serde_json::json!({
            "email": "unrecoverable@example.com",
            "password": password,
            "totp_code": totp_code_now(&totp_raw),
        }))
        .send()
        .await
        .expect("POST /admin/v1/auth/signin");
    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(universe::SIGNIN_STATUS, resp.status().as_u16().to_string());

    // Existing behavior (ADR-018 §5): AuthenticationFailed maps to the
    // pre-existing 500 INTERNAL_SERVER_ERROR this call site already returns.
    let mut expected = HashMap::new();
    expected.insert(universe::SIGNIN_STATUS, set_to("500".to_string()));
    assert_state_delta(&before, &after, &[universe::SIGNIN_STATUS], &expected);

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-03-05 (Malformed variant): truncated ciphertext ───────────────────

/// A ciphertext truncated below the 12-byte nonce minimum fails cleanly under
/// both keys — the existing `invalid_code()` 401 path, unchanged.
///
/// Journey (error path):
///   Given: a totp_secret_enc value is truncated to 8 bytes (below the 12-byte nonce minimum)
///   And:   EMBYR_ENCRYPTION_KEY_PREVIOUS is configured
///   When:  the affected user attempts to sign in with a TOTP code
///   Then:  the response is the same decrypt-failure outcome the system returns today
///   And:   no plaintext is returned to the caller
///
/// @error @real-io @US-SM-03 @AC-SM-03-05
#[tokio::test]
async fn totp_signin_malformed_ciphertext_below_nonce_minimum_fails_cleanly() {
    let (_pg, db_url) = start_postgres_container().await;

    let system_db = embyr_server::adapters::system_db::SystemDb::new(&db_url)
        .await
        .expect("connect");
    system_db.migrate().await.expect("migrate");
    let pool = system_db.pool().clone();

    let current_key = hex_to_key(TEST_ENCRYPTION_KEY);
    let (_account_id, user_id, totp_raw, password) =
        seed_totp_user(&pool, "corrupted-ciphertext@example.com", &current_key).await;
    corrupt_totp_secret(&pool, user_id).await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
            ("EMBYR_ENCRYPTION_KEY_PREVIOUS", TEST_ENCRYPTION_KEY_PREVIOUS),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/admin/v1/auth/signin",
            server.admin_port
        ))
        .json(&serde_json::json!({
            "email": "corrupted-ciphertext@example.com",
            "password": password,
            "totp_code": totp_code_now(&totp_raw),
        }))
        .send()
        .await
        .expect("POST /admin/v1/auth/signin");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "malformed ciphertext must map to the existing invalid_code() 401 path"
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.to_lowercase().contains("secret"),
        "no plaintext secret material may be returned to the caller: {body}"
    );

    server.sigterm();
    let _ = server.wait_for_exit(Duration::from_secs(15)).await;
}

// ─── AC-SM-03-06: identical current/previous key rejected ───────────────────

/// Startup rejects an identical current and previous encryption key.
///
/// Journey (error path):
///   Given: EMBYR_ENCRYPTION_KEY and EMBYR_ENCRYPTION_KEY_PREVIOUS are set to the same value
///   When:  Sam runs "cargo run -p embyr-server"
///   Then:  the process exits with code 1
///   And:   stderr states that EMBYR_ENCRYPTION_KEY_PREVIOUS must differ from EMBYR_ENCRYPTION_KEY
///
/// @error @US-SM-03 @AC-SM-03-06
#[tokio::test]
async fn startup_rejects_identical_current_and_previous_encryption_key() {
    let mut server = ServerProcess::start_env_only(&[
        (
            "DATABASE_URL",
            "postgres://postgres:postgres@127.0.0.1:5432/embyr",
        ),
        ("EMBYR_ADMIN_KEY", "testkey"),
        ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ("EMBYR_ENCRYPTION_KEY_PREVIOUS", TEST_ENCRYPTION_KEY),
    ]);

    let exit_code = server.wait_for_exit(Duration::from_secs(10)).await;
    let stderr = server.drain_stderr();

    assert_eq!(
        exit_code,
        Some(1),
        "server must exit 1 when EMBYR_ENCRYPTION_KEY_PREVIOUS equals EMBYR_ENCRYPTION_KEY; got {exit_code:?}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("EMBYR_ENCRYPTION_KEY_PREVIOUS") && stderr.contains("differ"),
        "stderr must state that EMBYR_ENCRYPTION_KEY_PREVIOUS must differ from EMBYR_ENCRYPTION_KEY; got: {stderr}"
    );
}

// ─── AC-SM-03-03: single shared decrypt function — direct calls ─────────────

/// `decrypt_with_rotation` tries the current key before the previous key —
/// unit-level ordering check (Rust-level driving port: the function's public
/// signature IS the entry point per port-to-port testing at this scope).
///
/// @in-memory @US-SM-03 @AC-SM-03-03
#[test]
fn decrypt_with_rotation_tries_current_before_previous() {
    let current = hex_to_key(TEST_ENCRYPTION_KEY);
    let previous = hex_to_key(TEST_ENCRYPTION_KEY_PREVIOUS);
    let plaintext = b"totp-secret-plaintext-20-bytes!";

    // Ciphertext encrypted under CURRENT — must decrypt without needing previous.
    let raw: [u8; 20] = plaintext[..20].try_into().unwrap();
    let ciphertext_under_current = encrypt_totp_secret(&current, &raw);

    let result = decrypt_with_rotation(&current, Some(&previous), &ciphertext_under_current);
    assert_eq!(
        result.expect("must decrypt under the current key"),
        raw.to_vec(),
        "decrypt_with_rotation must try current_key first and succeed without falling back"
    );

    // Ciphertext encrypted under PREVIOUS — must fall back and still succeed.
    let ciphertext_under_previous = encrypt_totp_secret(&previous, &raw);
    let result = decrypt_with_rotation(&current, Some(&previous), &ciphertext_under_previous);
    assert_eq!(
        result.expect("must fall back to the previous key"),
        raw.to_vec(),
        "decrypt_with_rotation must fall back to previous_key on current-key AEAD failure"
    );

    // Neither key: AuthenticationFailed.
    let third_key = hex_to_key(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let ciphertext_under_third = encrypt_totp_secret(&third_key, &raw);
    let result = decrypt_with_rotation(&current, Some(&previous), &ciphertext_under_third);
    assert!(matches!(result, Err(RotationDecryptError::AuthenticationFailed)));

    // Malformed: below the 12-byte nonce minimum.
    let result = decrypt_with_rotation(&current, Some(&previous), &[0u8; 4]);
    assert!(matches!(result, Err(RotationDecryptError::Malformed)));
}

/// The shared rotation-aware decrypt helper is correct for ALL THREE
/// ciphertext shapes it must serve (US-SM-03 UAT scenario 6) — TOTP-secret-shaped
/// (20 raw bytes), client-secret-shaped (arbitrary-length string bytes), and
/// DSN-shaped (connection-string bytes) — even though only the TOTP call site
/// is live today.
///
/// @in-memory @US-SM-03 @AC-SM-03-03
#[test]
fn decrypt_with_rotation_works_against_oidc_and_dsn_shaped_ciphertext() {
    let old_key = hex_to_key(TEST_ENCRYPTION_KEY_PREVIOUS);
    let new_key = hex_to_key(TEST_ENCRYPTION_KEY);

    // client_secret-shaped plaintext (OIDC provider's client_secret).
    let client_secret = b"okta-client-secret-9f2c".to_vec();
    let enc_client_secret = encrypt_arbitrary(&old_key, &client_secret);
    let decrypted = decrypt_with_rotation(&new_key, Some(&old_key), &enc_client_secret)
        .expect("client_secret-shaped ciphertext must decrypt via previous-key fallback");
    assert_eq!(decrypted, client_secret);

    // DSN-shaped plaintext (projects.backend_pg_dsn_enc).
    let dsn = b"postgres://user:pass@host:5432/customer_db".to_vec();
    let enc_dsn = encrypt_arbitrary(&old_key, &dsn);
    let decrypted = decrypt_with_rotation(&new_key, Some(&old_key), &enc_dsn)
        .expect("DSN-shaped ciphertext must decrypt via previous-key fallback");
    assert_eq!(decrypted, dsn);
}

/// AES-256-GCM encrypt arbitrary-length `plaintext` under `key`, prefixing the
/// 12-byte random nonce — same on-disk shape as [`encrypt_totp_secret`] but
/// for non-fixed-length plaintexts (client secrets, DSNs).
fn encrypt_arbitrary(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use rand_core::{OsRng, RngCore};

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).expect("valid key");
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).expect("encrypt");
    let mut enc = nonce_bytes.to_vec();
    enc.extend_from_slice(&ciphertext);
    enc
}

//! ADR-083: connection secrets (customer DSN, agent TLS bundle) live in ONE store, encrypted under
//! the server key (`EMBYR_ENCRYPTION_KEY`, current/previous rotation). Legacy rows still hold an
//! ECIES-to-API-key copy; request paths read both, serve per [`resolve`], and convert lazily.

use embyr_core::auth::ecies;
use sqlx::PgPool;

use crate::adapters::encryption::{decrypt_with_rotation, encrypt};

#[derive(Clone, Copy)]
pub struct ServerKeys {
    pub current: [u8; 32],
    pub previous: Option<[u8; 32]>,
}

impl ServerKeys {
    pub fn encrypt(&self, plaintext: &str) -> Vec<u8> {
        encrypt(&self.current, plaintext.as_bytes())
            .expect("AES-GCM encrypt of in-memory plaintext cannot fail")
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Option<String> {
        let bytes = decrypt_with_rotation(&self.current, self.previous.as_ref(), ciphertext).ok()?;
        String::from_utf8(bytes).ok()
    }
}

/// What the request path must do to the server-store column after serving a request.
#[derive(Debug, PartialEq, Eq)]
pub enum Conversion {
    /// Server store already holds the served value.
    None,
    /// Server store empty (or undecryptable) and legacy value served: fill it.
    Insert,
    /// Both stores decrypt but differ: legacy (what traffic used) wins and overwrites.
    Overwrite,
}

/// Pure read-order rule. Server-store only → served as is; legacy only → served, convert;
/// both equal → server; both differ → legacy wins (AC-DSS-05).
pub fn resolve(server: Option<String>, legacy: Option<String>) -> Option<(String, Conversion)> {
    match (server, legacy) {
        (None, None) => None,
        (Some(s), None) => Some((s, Conversion::None)),
        (None, Some(l)) => Some((l, Conversion::Insert)),
        (Some(s), Some(l)) if s == l => Some((s, Conversion::None)),
        (Some(_), Some(l)) => Some((l, Conversion::Overwrite)),
    }
}

/// Which server-store column a secret lives in.
#[derive(Clone, Copy)]
pub enum Secret {
    Dsn,
    AgentBundle,
}

impl Secret {
    fn label(self) -> &'static str {
        match self {
            Secret::Dsn => "dsn",
            Secret::AgentBundle => "bundle",
        }
    }

    fn cas_sql(self) -> &'static str {
        match self {
            Secret::Dsn => {
                "UPDATE projects SET backend_pg_dsn_enc = $1 \
                 WHERE id = $2 AND backend_pg_dsn_enc IS NOT DISTINCT FROM $3"
            }
            Secret::AgentBundle => {
                "UPDATE projects SET agent_tls_bundle_srv_enc = $1 \
                 WHERE id = $2 AND agent_tls_bundle_srv_enc IS NOT DISTINCT FROM $3"
            }
        }
    }
}

/// Decrypt both copies and apply [`resolve`]. `None` when neither store yields a value.
pub fn read_secret(
    keys: &ServerKeys,
    api_key: &str,
    server_enc: Option<&[u8]>,
    legacy_enc: Option<&[u8]>,
) -> Option<(String, Conversion)> {
    let server = server_enc.and_then(|c| keys.decrypt(c));
    let legacy = legacy_enc
        .and_then(|c| ecies::decrypt(api_key.as_bytes(), c).ok())
        .and_then(|b| String::from_utf8(b).ok());
    resolve(server, legacy)
}

/// Best-effort, never blocks or fails the request: compare-and-set the server-store column from
/// the value that was just served (idempotent under races: only the row state we read is replaced).
pub fn spawn_conversion(
    pool: PgPool,
    project_id: String,
    secret: Secret,
    conversion: Conversion,
    old_server_enc: Option<Vec<u8>>,
    plaintext: String,
    keys: ServerKeys,
) {
    if conversion == Conversion::None {
        return;
    }
    if conversion == Conversion::Overwrite {
        tracing::warn!(project_id = %project_id, secret = secret.label(),
            "connection secret: legacy and server-store values differ, legacy wins and overwrites");
        metrics::counter!("embyr_dsn_convert_divergent_total", "secret" => secret.label())
            .increment(1);
    }
    tokio::spawn(async move {
        let enc = keys.encrypt(&plaintext);
        let result = sqlx::query(secret.cas_sql())
            .bind(&enc)
            .bind(&project_id)
            .bind(&old_server_enc)
            .execute(&pool)
            .await;
        if let Err(e) = result {
            tracing::warn!(project_id = %project_id, secret = secret.label(), error = %e,
                "connection secret conversion failed, will retry on a later request");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn read_order_table() {
        assert_eq!(resolve(None, None), None);
        assert_eq!(resolve(s("a"), None), Some(("a".into(), Conversion::None)));
        assert_eq!(resolve(None, s("l")), Some(("l".into(), Conversion::Insert)));
        assert_eq!(resolve(s("x"), s("x")), Some(("x".into(), Conversion::None)));
        assert_eq!(resolve(s("srv"), s("leg")), Some(("leg".into(), Conversion::Overwrite)));
    }

    #[test]
    fn rotation_window_decrypts_with_previous_key() {
        let old = [1u8; 32];
        let new = [2u8; 32];
        let ct = ServerKeys { current: old, previous: None }.encrypt("postgres://x");
        let rotated = ServerKeys { current: new, previous: Some(old) };
        assert_eq!(rotated.decrypt(&ct).as_deref(), Some("postgres://x"));
        assert_eq!(ServerKeys { current: new, previous: None }.decrypt(&ct), None);
    }

    #[test]
    fn undecryptable_server_copy_falls_back_to_legacy() {
        let keys = ServerKeys { current: [3u8; 32], previous: None };
        let api_key = "k-api";
        let legacy = ecies::encrypt(&ecies::derive_public_key(api_key.as_bytes()), b"postgres://l").unwrap();
        let wrong = ServerKeys { current: [9u8; 32], previous: None }.encrypt("postgres://s");
        assert_eq!(
            read_secret(&keys, api_key, Some(&wrong), Some(&legacy)),
            Some(("postgres://l".into(), Conversion::Insert))
        );
    }
}

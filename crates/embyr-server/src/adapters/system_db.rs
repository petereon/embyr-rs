use embyr_core::error::CoreError;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

/// client-auth (ADR-025): a project's registered client-identity verification
/// credential row, as stored in `client_identity_credentials`. Raw key bytes
/// — never hashed, never encrypted (public key material has no
/// confidentiality property to protect).
#[derive(Debug, Clone)]
pub struct ClientIdentityCredentialRow {
    pub public_key_current: Vec<u8>,
    /// `None` when no rotation window is open.
    pub public_key_previous: Option<Vec<u8>>,
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub rotated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Result row of a rotation `UPDATE ... RETURNING` (ADR-025 § Rotation).
/// Deliberately excludes public key material — the rotate response, like
/// registration's, never echoes raw key bytes back (mirrors AC-16-01).
#[derive(Debug, Clone)]
pub struct ClientIdentityCredentialRotationRow {
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub rotated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// client-auth-hosted-identity (ADR-036 Decision 5): the result of folding
/// project-ownership verification and `backend_mode` into one query
/// (mirrors `verify_project_ownership`'s WHERE-clause shape, extended with
/// one column) — used by the session-authenticated `enable_hosted_identity`
/// admin action, which `get_project_for_auth`/`verify_project_ownership`
/// cannot serve alone (former has no ownership check, latter does not
/// surface `backend_mode`).
#[derive(Debug, Clone)]
pub struct ProjectBackendModeRow {
    pub backend_mode: String,
}

/// client-auth-hosted-identity (ADR-036 Decision 2): a project's embyr-owned
/// hosted-identity signing key, as stored in `hosted_identity_signing_keys`
/// (System DB — structurally disjoint from `client_identity_credentials`,
/// Resolution 3). `private_key_enc` is ECIES ciphertext, never plaintext.
#[derive(Debug, Clone)]
pub struct HostedIdentitySigningKeyRow {
    pub public_key: Vec<u8>,
    pub private_key_enc: Vec<u8>,
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules (ADR-028): a project's per-collection access-control rule
/// row, as stored in `access_rules`. `condition_source` is the raw,
/// validated grammar text — NOT a serialized AST (ADR-028 § Store Source,
/// Not AST) — re-parsed via `embyr_core::access_control::parse_condition`
/// on every gated `GetDocument` call.
#[derive(Debug, Clone)]
pub struct AccessRuleRow {
    pub condition_source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules-operations (ADR-035): one `access_rule_history` row —
/// the condition a rule held at some point, who set it, and when.
#[derive(Debug, Clone)]
pub struct AccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules-write-path (ADR-030): a project's per-collection WRITE
/// rule row, as stored in `write_access_rules` — a table entirely
/// independent of `access_rules` (ADR-030 § Decision — Storage Shape).
/// Schema-identical shape to `AccessRuleRow`, deliberately a separate type
/// (not shared), mirroring `write_access_rules`' own independent-table
/// decision at the Rust type level.
#[derive(Debug, Clone)]
pub struct WriteAccessRuleRow {
    pub condition_source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules-operations (ADR-035, Slice 04): one
/// `write_access_rule_history` row — mirrors `AccessRuleHistoryRow` exactly,
/// against the independent write-rule history table (AC-17-169's
/// structural-independence guarantee).
#[derive(Debug, Clone)]
pub struct WriteAccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules-collection-group-rules (ADR-032): a project's per-
/// collection-id COLLECTION-GROUP access-control rule, as stored in
/// `group_access_rules` — a table structurally independent of both
/// `access_rules` (read, exact-path) and `write_access_rules` (write,
/// exact-path). Schema-identical shape to both, deliberately a separate
/// type (mirrors `WriteAccessRuleRow`'s own precedent of not sharing a type
/// with `AccessRuleRow`).
#[derive(Debug, Clone)]
pub struct GroupAccessRuleRow {
    pub condition_source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// security-rules-operations (ADR-035, Slice 05): one
/// `group_access_rule_history` row — mirrors `WriteAccessRuleHistoryRow`
/// exactly, against the independent group-rule history table (AC-17-172's
/// structural-independence guarantee).
#[derive(Debug, Clone)]
pub struct GroupAccessRuleHistoryRow {
    pub id: i64,
    pub condition_source: String,
    pub actor_account_id: uuid::Uuid,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

/// Project row returned for credential verification.
#[derive(Debug)]
pub struct ProjectAuthRow {
    pub id: String,
    pub status: String,
    pub backend_mode: String,
    pub api_key_hash_current: String,
    pub api_key_hash_previous: Option<String>,
    pub ecies_encrypted_dsn: Option<Vec<u8>>,
    /// For agent-mode projects: the gRPC endpoint (host:port) of the agent.
    pub backend_agent_endpoint: Option<String>,
    /// For agent-mode projects: ECIES-encrypted JSON TLS bundle (ca_pem, client_cert_pem, client_key_pem).
    pub agent_tls_bundle_enc: Option<Vec<u8>>,
    /// For aws_secret-mode projects: the ARN of the AWS Secrets Manager secret.
    pub backend_secret_arn: Option<String>,
    /// For gcp_secret-mode projects: the GCP Secret Manager resource name.
    pub backend_secret_gcp: Option<String>,
}

#[derive(Debug)]
pub struct SystemDb {
    pool: PgPool,
}

impl SystemDb {
    /// Connect to system DB. Does NOT run migrations — call migrate() separately.
    ///
    /// Pool acquire timeout is 5 seconds to ensure startup fails fast when the
    /// database is unreachable (rather than the sqlx default of 30 seconds).
    pub async fn new(database_url: &str) -> Result<Self, CoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(database_url)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        Ok(Self { pool })
    }

    /// Run sqlx migrations from the `migrations/` directory (workspace root).
    pub async fn migrate(&self) -> Result<(), CoreError> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))
    }

    /// Fetch project row for authentication.
    pub async fn get_project_for_auth(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectAuthRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT id, status, backend_mode, api_key_hash_current, \
             api_key_hash_previous, ecies_encrypted_dsn, \
             backend_agent_endpoint, agent_tls_bundle_enc, backend_secret_arn, \
             backend_secret_gcp \
             FROM projects WHERE id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(ProjectAuthRow {
            id: r
                .try_get("id")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            status: r
                .try_get("status")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            backend_mode: r
                .try_get("backend_mode")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            api_key_hash_current: r
                .try_get("api_key_hash_current")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            api_key_hash_previous: r
                .try_get("api_key_hash_previous")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            ecies_encrypted_dsn: r
                .try_get::<Option<Vec<u8>>, _>("ecies_encrypted_dsn")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            backend_agent_endpoint: r
                .try_get::<Option<String>, _>("backend_agent_endpoint")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            agent_tls_bundle_enc: r
                .try_get::<Option<Vec<u8>>, _>("agent_tls_bundle_enc")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            backend_secret_arn: r
                .try_get::<Option<String>, _>("backend_secret_arn")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            backend_secret_gcp: r
                .try_get::<Option<String>, _>("backend_secret_gcp")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    /// Expose the raw pool for adapters that share the system DB connection.
    ///
    /// Use sparingly — prefer going through SystemDb's typed query methods.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Verify DB is reachable and expected schema tables exist.
    pub async fn probe(&self) -> Result<(), CoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("system DB unreachable: {e}")))?;

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = 'projects'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("schema check failed: {e}")))?;

        if count == 0 {
            return Err(CoreError::BackendUnavailable(
                "system DB schema not initialized (projects table missing)".into(),
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // client-auth (ADR-025) — client_identity_credentials CRUD.
    // insert_client_identity_credential: implemented (step 01-01).
    // rotate_client_identity_credential: implemented (step 03-01).
    // get_client_identity_credential: implemented (step 04-01).
    // -----------------------------------------------------------------------

    /// Read a project's registered client-identity credential, if any.
    /// `Ok(None)` means no credential has been registered for this project.
    pub async fn get_client_identity_credential(
        &self,
        project_id: &str,
    ) -> Result<Option<ClientIdentityCredentialRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT public_key_current, public_key_previous, algorithm, created_at, rotated_at \
             FROM client_identity_credentials WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(ClientIdentityCredentialRow {
            public_key_current: r
                .try_get("public_key_current")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            public_key_previous: r
                .try_get::<Option<Vec<u8>>, _>("public_key_previous")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            algorithm: r
                .try_get("algorithm")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: r
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            rotated_at: r
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("rotated_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    /// Register a project's first client-identity verification credential
    /// (US-01). Relies on the `PRIMARY KEY` constraint to make a second
    /// registration attempt a Postgres unique-violation (mapped by the
    /// caller to HTTP 409, AC-16-04) — a database-enforced invariant, not an
    /// application-level check that could drift from the schema (ADR-025).
    pub async fn insert_client_identity_credential(
        &self,
        project_id: &str,
        public_key: &[u8; 32],
    ) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO client_identity_credentials (project_id, public_key_current, algorithm) \
             VALUES ($1, $2, 'EdDSA')",
        )
        .bind(project_id)
        .bind(&public_key[..])
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                // PostgreSQL unique_violation = "23505" — the PRIMARY KEY on
                // project_id (AC-16-04: a second registration is rejected,
                // not silently overwritten).
                if db_err.code().as_deref() == Some("23505") {
                    return CoreError::AlreadyExists(format!(
                        "client identity credential already registered for project {project_id}"
                    ));
                }
            }
            CoreError::BackendUnavailable(format!("insert_client_identity_credential failed: {e}"))
        })?;
        Ok(())
    }

    /// Rotate a project's client-identity credential (US-03): shifts
    /// current -> previous, sets the new current key, stamps `rotated_at`.
    /// One rotation generation retained (ADR-025 — not an unbounded
    /// history), identical shape to ADR-018's `admin_key`/`admin_key_previous`.
    ///
    /// `Ok(None)` means no credential was registered for this project (the
    /// `UPDATE` matched zero rows) — the caller maps this to 404, mirroring
    /// `insert_client_identity_credential`'s error-mapping convention.
    pub async fn rotate_client_identity_credential(
        &self,
        project_id: &str,
        new_public_key: &[u8; 32],
    ) -> Result<Option<ClientIdentityCredentialRotationRow>, CoreError> {
        let row_opt = sqlx::query(
            "UPDATE client_identity_credentials \
             SET public_key_previous = public_key_current, public_key_current = $2, rotated_at = now() \
             WHERE project_id = $1 \
             RETURNING algorithm, created_at, rotated_at",
        )
        .bind(project_id)
        .bind(&new_public_key[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("rotate_client_identity_credential failed: {e}"))
        })?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(ClientIdentityCredentialRotationRow {
            algorithm: r
                .try_get("algorithm")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: r
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            rotated_at: r
                .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("rotated_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    // -----------------------------------------------------------------------
    // client-auth-hosted-identity (ADR-036) — hosted_identity_signing_keys.
    // -----------------------------------------------------------------------

    /// Fold project-ownership verification and `backend_mode` into one query
    /// (US-01, ADR-036 Decision 5) for the session-authenticated
    /// `enable_hosted_identity` admin action. `Ok(None)` means the project
    /// does not exist, is deleted, or belongs to a different account —
    /// caller maps this to 404 (mirrors `verify_project_ownership`'s own
    /// "no matching row" -> 404 convention).
    pub async fn get_project_backend_mode(
        &self,
        project_id: &str,
        account_id: Uuid,
    ) -> Result<Option<ProjectBackendModeRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT backend_mode FROM projects \
             WHERE id = $1 AND account_id = $2 AND status != 'deleted'",
        )
        .bind(project_id)
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(ProjectBackendModeRow {
            backend_mode: r
                .try_get("backend_mode")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    /// Enable hosted identity for a project (US-01): stores the
    /// server-generated, ECIES-encrypted signing key. Idempotent UPSERT
    /// (AC-18-02) — `INSERT ... ON CONFLICT (project_id) DO NOTHING
    /// RETURNING` returns zero rows on conflict; the fallback `SELECT` then
    /// reads back the EXISTING row untouched, so a second enablement call
    /// returns byte-identical `public_key`/`private_key_enc` — no
    /// regeneration, no re-encryption.
    pub async fn enable_hosted_identity(
        &self,
        project_id: &str,
        public_key: &[u8; 32],
        private_key_enc: &[u8],
    ) -> Result<HostedIdentitySigningKeyRow, CoreError> {
        let row_opt = sqlx::query(
            "INSERT INTO hosted_identity_signing_keys \
             (project_id, public_key, private_key_enc, algorithm) \
             VALUES ($1, $2, $3, 'EdDSA') \
             ON CONFLICT (project_id) DO NOTHING \
             RETURNING public_key, private_key_enc, algorithm, created_at",
        )
        .bind(project_id)
        .bind(&public_key[..])
        .bind(private_key_enc)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("enable_hosted_identity insert failed: {e}"))
        })?;

        let row = match row_opt {
            Some(r) => r,
            None => sqlx::query(
                "SELECT public_key, private_key_enc, algorithm, created_at \
                 FROM hosted_identity_signing_keys WHERE project_id = $1",
            )
            .bind(project_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                CoreError::BackendUnavailable(format!(
                    "enable_hosted_identity fallback select failed: {e}"
                ))
            })?,
        };

        Ok(HostedIdentitySigningKeyRow {
            public_key: row
                .try_get("public_key")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            private_key_enc: row
                .try_get("private_key_enc")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            algorithm: row
                .try_get("algorithm")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        })
    }

    // -----------------------------------------------------------------------
    // security-rules (ADR-028) — access_rules CRUD.
    // -----------------------------------------------------------------------

    /// Define OR redefine (Resolution 3: idempotent upsert, the SAME action
    /// either way — ADR-028 § Decision, `INSERT ... ON CONFLICT (project_id,
    /// collection_path) DO UPDATE`) the access rule for `(project_id,
    /// collection_path)`. There is no separate `insert_*`/`redefine_*`
    /// pair, unlike `insert_client_identity_credential`/
    /// `rotate_client_identity_credential` above — see ADR-028 § Decision,
    /// "Adapter methods" for why that asymmetry is intentional.
    ///
    /// security-rules-operations (ADR-035 § Decision — Capture Mechanism
    /// Placement): history capture is FUSED into this SAME method, in the
    /// SAME transaction as the existing upsert — not a second adapter call.
    /// The existing upsert statement text above remains byte-for-byte
    /// unchanged; the history `INSERT` is additive, in the identical
    /// transaction, so the rule change and its history entry succeed or
    /// fail together. `actor_account_id` is a compiler-enforced required
    /// parameter — every caller (present and future) must supply it.
    pub async fn upsert_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
        condition_source: &str,
        actor_account_id: uuid::Uuid,
    ) -> Result<(), CoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx begin failed: {e}")))?;

        sqlx::query(
            "INSERT INTO access_rules (project_id, collection_path, condition_source) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (project_id, collection_path) \
             DO UPDATE SET condition_source = EXCLUDED.condition_source, updated_at = now()",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .execute(&mut *tx)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("upsert_access_rule failed: {e}")))?;

        sqlx::query(
            "INSERT INTO access_rule_history \
             (project_id, collection_path, condition_source, actor_account_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .bind(actor_account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("access_rule_history insert failed: {e}"))
        })?;

        tx.commit()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx commit failed: {e}")))?;
        Ok(())
    }

    /// Look up the access rule for `(project_id, collection_path)`.
    /// `Ok(None)` is the mechanism behind the structural no-rule-defined
    /// guardrail (ADR-029 § Structural no-rule-defined guardrail,
    /// AC-17-14/15/16): when `None`, `grpc::handler::handle_get_document`
    /// takes the EXACT unmodified pre-`security-rules` code path —
    /// `embyr_core::access_control::evaluate()` is never called.
    pub async fn get_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
    ) -> Result<Option<AccessRuleRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT condition_source, created_at, updated_at \
             FROM access_rules WHERE project_id = $1 AND collection_path = $2",
        )
        .bind(project_id)
        .bind(collection_path)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("get_access_rule failed: {e}")))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(AccessRuleRow {
            condition_source: r
                .try_get("condition_source")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: r
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            updated_at: r
                .try_get("updated_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    /// Retrieve `(project_id, collection_path)`'s complete history, newest
    /// first (security-rules-operations, US-02, ADR-035). Ordered by `id
    /// DESC` — the authoritative monotonic ordering key (ADR-035 § Decision
    /// — Schema, "not `captured_at` alone"), served by
    /// `idx_access_rule_history_lookup`, no scan. A collection with no rule
    /// ever defined yields an empty `Vec`, never an error (AC-17-161) — a
    /// natural consequence of zero matching rows, not a special case.
    pub async fn get_access_rule_history(
        &self,
        project_id: &str,
        collection_path: &str,
    ) -> Result<Vec<AccessRuleHistoryRow>, CoreError> {
        let rows = sqlx::query(
            "SELECT id, condition_source, actor_account_id, captured_at \
             FROM access_rule_history WHERE project_id = $1 AND collection_path = $2 \
             ORDER BY id DESC",
        )
        .bind(project_id)
        .bind(collection_path)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("get_access_rule_history failed: {e}")))?;

        rows.into_iter()
            .map(|r| {
                Ok(AccessRuleHistoryRow {
                    id: r
                        .try_get("id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    condition_source: r
                        .try_get("condition_source")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    actor_account_id: r
                        .try_get("actor_account_id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    captured_at: r
                        .try_get("captured_at")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // security-rules-write-path (ADR-030) — write_access_rules CRUD.
    // -----------------------------------------------------------------------

    /// Define OR redefine (same idempotent-upsert shape as
    /// `upsert_access_rule` — ADR-030 § Decision — Storage Shape) the WRITE
    /// rule for `(project_id, collection_path)`. Operates against
    /// `write_access_rules` EXCLUSIVELY — no `access_rules` in this
    /// statement's FROM/INTO clause at all, the structural mechanism behind
    /// AC-17-43/AC-17-22's independence guarantee.
    ///
    /// security-rules-operations (ADR-035, Slice 04): history capture is
    /// FUSED into this SAME method, in the SAME transaction as the existing
    /// upsert — mirrors `upsert_access_rule`'s exact Slice-01 shape, applied
    /// to `write_access_rule_history`. `actor_account_id` is a
    /// compiler-enforced required parameter, identical discipline.
    pub async fn upsert_write_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
        condition_source: &str,
        actor_account_id: uuid::Uuid,
    ) -> Result<(), CoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx begin failed: {e}")))?;

        sqlx::query(
            "INSERT INTO write_access_rules (project_id, collection_path, condition_source) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (project_id, collection_path) \
             DO UPDATE SET condition_source = EXCLUDED.condition_source, updated_at = now()",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("upsert_write_access_rule failed: {e}"))
        })?;

        sqlx::query(
            "INSERT INTO write_access_rule_history \
             (project_id, collection_path, condition_source, actor_account_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .bind(actor_account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("write_access_rule_history insert failed: {e}"))
        })?;

        tx.commit()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx commit failed: {e}")))?;
        Ok(())
    }

    /// Look up the write rule for `(project_id, collection_path)`. `Ok(None)`
    /// is this feature's own version of ADR-029's structural
    /// no-rule-defined guardrail (AC-17-42, mirrors `get_access_rule`'s
    /// identical shape exactly): when `None`,
    /// `grpc::handler::handle_create_document` (and Update/Delete, Slices
    /// 03/04) takes the EXACT unmodified pre-`security-rules-write-path`
    /// code path — `embyr_core::access_control::evaluate()` is never called.
    pub async fn get_write_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
    ) -> Result<Option<WriteAccessRuleRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT condition_source, created_at, updated_at \
             FROM write_access_rules WHERE project_id = $1 AND collection_path = $2",
        )
        .bind(project_id)
        .bind(collection_path)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("get_write_access_rule failed: {e}")))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(WriteAccessRuleRow {
            condition_source: r
                .try_get("condition_source")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: r
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            updated_at: r
                .try_get("updated_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }

    /// Retrieve `(project_id, collection_path)`'s complete WRITE-rule
    /// history, newest first (security-rules-operations, Slice 04, ADR-035)
    /// — mirrors `get_access_rule_history` exactly, against
    /// `write_access_rule_history` EXCLUSIVELY (AC-17-169's
    /// structural-independence guarantee: no `access_rule_history` in this
    /// statement's FROM clause at all).
    pub async fn get_write_access_rule_history(
        &self,
        project_id: &str,
        collection_path: &str,
    ) -> Result<Vec<WriteAccessRuleHistoryRow>, CoreError> {
        let rows = sqlx::query(
            "SELECT id, condition_source, actor_account_id, captured_at \
             FROM write_access_rule_history WHERE project_id = $1 AND collection_path = $2 \
             ORDER BY id DESC",
        )
        .bind(project_id)
        .bind(collection_path)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("get_write_access_rule_history failed: {e}"))
        })?;

        rows.into_iter()
            .map(|r| {
                Ok(WriteAccessRuleHistoryRow {
                    id: r
                        .try_get("id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    condition_source: r
                        .try_get("condition_source")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    actor_account_id: r
                        .try_get("actor_account_id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    captured_at: r
                        .try_get("captured_at")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                })
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // security-rules-collection-group-rules (ADR-032) — group_access_rules
    // CRUD.
    // -----------------------------------------------------------------------

    /// Define OR redefine (same idempotent-upsert shape as
    /// `upsert_access_rule`/`upsert_write_access_rule`) the COLLECTION-GROUP
    /// rule for `(project_id, collection_id)`. Operates against
    /// `group_access_rules` EXCLUSIVELY — no `access_rules`/
    /// `write_access_rules` in this statement's FROM/INTO clause at all, the
    /// structural mechanism behind AC-17-79's independence guarantee.
    ///
    /// security-rules-operations (ADR-035, Slice 05): history capture is
    /// FUSED into this SAME method, in the SAME transaction as the existing
    /// upsert — mirrors `upsert_write_access_rule`'s exact Slice-04 shape,
    /// applied to `group_access_rule_history`. `actor_account_id` is a
    /// compiler-enforced required parameter, identical discipline.
    pub async fn upsert_group_access_rule(
        &self,
        project_id: &str,
        collection_id: &str,
        condition_source: &str,
        actor_account_id: uuid::Uuid,
    ) -> Result<(), CoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx begin failed: {e}")))?;

        sqlx::query(
            "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (project_id, collection_id) \
             DO UPDATE SET condition_source = EXCLUDED.condition_source, updated_at = now()",
        )
        .bind(project_id)
        .bind(collection_id)
        .bind(condition_source)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("upsert_group_access_rule failed: {e}"))
        })?;

        sqlx::query(
            "INSERT INTO group_access_rule_history \
             (project_id, collection_id, condition_source, actor_account_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(project_id)
        .bind(collection_id)
        .bind(condition_source)
        .bind(actor_account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("group_access_rule_history insert failed: {e}"))
        })?;

        tx.commit()
            .await
            .map_err(|e| CoreError::BackendUnavailable(format!("tx commit failed: {e}")))?;
        Ok(())
    }

    /// Retrieve `(project_id, collection_id)`'s complete GROUP-rule history,
    /// newest first (security-rules-operations, Slice 05, ADR-035) —
    /// mirrors `get_write_access_rule_history` exactly, against
    /// `group_access_rule_history` EXCLUSIVELY (AC-17-172's
    /// structural-independence guarantee: no `access_rule_history`/
    /// `write_access_rule_history` in this statement's FROM clause at all).
    pub async fn get_group_access_rule_history(
        &self,
        project_id: &str,
        collection_id: &str,
    ) -> Result<Vec<GroupAccessRuleHistoryRow>, CoreError> {
        let rows = sqlx::query(
            "SELECT id, condition_source, actor_account_id, captured_at \
             FROM group_access_rule_history WHERE project_id = $1 AND collection_id = $2 \
             ORDER BY id DESC",
        )
        .bind(project_id)
        .bind(collection_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            CoreError::BackendUnavailable(format!("get_group_access_rule_history failed: {e}"))
        })?;

        rows.into_iter()
            .map(|r| {
                Ok(GroupAccessRuleHistoryRow {
                    id: r
                        .try_get("id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    condition_source: r
                        .try_get("condition_source")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    actor_account_id: r
                        .try_get("actor_account_id")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                    captured_at: r
                        .try_get("captured_at")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
                })
            })
            .collect()
    }

    /// Look up the collection-group rule for `(project_id, collection_id)`.
    /// `Ok(None)` is this feature's own version of ADR-029's structural
    /// no-rule-defined guardrail — but with the OPPOSITE default from
    /// `get_access_rule`/`get_write_access_rule` (ADR-032 Resolution 2:
    /// reject, not "unrestricted"). Consumed by Slice 02+'s
    /// `handle_run_query` composition, not this slice.
    pub async fn get_group_access_rule(
        &self,
        project_id: &str,
        collection_id: &str,
    ) -> Result<Option<GroupAccessRuleRow>, CoreError> {
        let row_opt = sqlx::query(
            "SELECT condition_source, created_at, updated_at \
             FROM group_access_rules WHERE project_id = $1 AND collection_id = $2",
        )
        .bind(project_id)
        .bind(collection_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(format!("get_group_access_rule failed: {e}")))?;

        let Some(r) = row_opt else {
            return Ok(None);
        };

        Ok(Some(GroupAccessRuleRow {
            condition_source: r
                .try_get("condition_source")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            created_at: r
                .try_get("created_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            updated_at: r
                .try_get("updated_at")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ContainerAsync};

    async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
        use testcontainers_modules::testcontainers::ImageExt;
        // Use pg15 — gen_random_uuid() is built-in since pg13; pg11 default image lacks it.
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");
        (container, url)
    }

    #[tokio::test]
    async fn probe_returns_ok_when_db_reachable_and_schema_current() {
        let (_container, url) = start_postgres().await;
        let db = SystemDb::new(&url).await.unwrap();
        db.migrate().await.unwrap();
        assert!(db.probe().await.is_ok());
    }

    #[tokio::test]
    async fn probe_returns_err_when_unreachable() {
        let result = SystemDb::new("postgres://postgres:postgres@127.0.0.1:9999/postgres").await;
        assert!(result.is_err(), "new() must fail for unreachable DB");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unavailable") || msg.contains("connect"),
            "error: {msg}"
        );
    }

    #[tokio::test]
    async fn migrations_apply_all_tables() {
        let (_container, url) = start_postgres().await;
        let db = SystemDb::new(&url).await.unwrap();
        db.migrate().await.unwrap();
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema='public' ORDER BY table_name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(
            tables.contains(&"projects".to_string()),
            "tables: {tables:?}"
        );
        assert!(
            tables.contains(&"daily_project_metrics".to_string()),
            "tables: {tables:?}"
        );
        assert!(
            tables.contains(&"composite_indexes".to_string()),
            "tables: {tables:?}"
        );
        assert!(
            tables.contains(&"client_identity_credentials".to_string()),
            "tables: {tables:?}"
        );
    }

    // -----------------------------------------------------------------------
    // security-rules-collection-group-rules (ADR-032) — group_access_rules
    // adapter tests. `upsert_group_access_rule`/`get_group_access_rule` have
    // no handler-level caller in Slice 01 (that is Slice 02+'s
    // `handle_run_query` composition) — exercised directly here instead.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn group_access_rule_upsert_and_get_round_trip() {
        let (_container, url) = start_postgres().await;
        let db = SystemDb::new(&url).await.unwrap();
        db.migrate().await.unwrap();
        let account_id: uuid::Uuid =
            sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('acc') RETURNING id")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ('proj-1', $1, 'direct_pg', 'hash', 'active', 'proj-1')",
        )
        .bind(account_id)
        .execute(&db.pool)
        .await
        .unwrap();

        // AC-17-91-adjacent: no rule defined -> None (the opposite-default
        // guardrail Slice 04 relies on; proven at the adapter level here).
        assert!(db
            .get_group_access_rule("proj-1", "journal_entries")
            .await
            .unwrap()
            .is_none());

        db.upsert_group_access_rule(
            "proj-1",
            "journal_entries",
            "request.auth.uid == resource.data.owner_id",
            account_id,
        )
        .await
        .unwrap();
        let row = db
            .get_group_access_rule("proj-1", "journal_entries")
            .await
            .unwrap()
            .expect("row must exist after upsert");
        assert_eq!(
            row.condition_source,
            "request.auth.uid == resource.data.owner_id"
        );

        // Redefine: full replace, no merge.
        db.upsert_group_access_rule("proj-1", "journal_entries", "true", account_id)
            .await
            .unwrap();
        let row = db
            .get_group_access_rule("proj-1", "journal_entries")
            .await
            .unwrap()
            .expect("row must exist after redefine");
        assert_eq!(row.condition_source, "true");
    }

    /// ADR-032 § Decision — Schema: `CHECK (collection_id NOT LIKE '%/%')`
    /// is a second, DB-level defense-in-depth layer alongside the admin
    /// handler's own `validate_bare_collection_id` 400 — proven here by a
    /// raw SQL bypass of the adapter/handler entirely.
    #[tokio::test]
    async fn group_access_rules_check_constraint_rejects_slash_containing_collection_id() {
        let (_container, url) = start_postgres().await;
        let db = SystemDb::new(&url).await.unwrap();
        db.migrate().await.unwrap();
        let account_id: uuid::Uuid =
            sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('acc') RETURNING id")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
             VALUES ('proj-1', $1, 'direct_pg', 'hash', 'active', 'proj-1')",
        )
        .bind(account_id)
        .execute(&db.pool)
        .await
        .unwrap();

        let result = sqlx::query(
            "INSERT INTO group_access_rules (project_id, collection_id, condition_source) \
             VALUES ('proj-1', 'expeditions/journal_entries', 'true')",
        )
        .execute(&db.pool)
        .await;

        let err = result.expect_err("CHECK constraint must reject a '/'-containing collection_id");
        if let sqlx::Error::Database(db_err) = &err {
            // PostgreSQL check_violation = "23514".
            assert_eq!(db_err.code().as_deref(), Some("23514"), "err: {db_err}");
        } else {
            panic!("expected a database CHECK-constraint error, got: {err}");
        }
    }
}

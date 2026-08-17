use embyr_core::error::CoreError;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

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
}

use embyr_core::error::CoreError;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

/// Project row returned for credential verification.
#[derive(Debug)]
pub struct ProjectAuthRow {
    pub id: String,
    pub status: String,
    pub backend_mode: String,
    pub api_key_hash_current: String,
    pub api_key_hash_previous: Option<String>,
    pub ecies_encrypted_dsn: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct SystemDb {
    pool: PgPool,
}

impl SystemDb {
    /// Connect to system DB. Does NOT run migrations — call migrate() separately.
    pub async fn new(database_url: &str) -> Result<Self, CoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
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
             api_key_hash_previous, ecies_encrypted_dsn \
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
            .map_err(|e| {
                CoreError::BackendUnavailable(format!("system DB unreachable: {e}"))
            })?;

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
    async fn migrations_apply_all_three_tables() {
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
    }
}

/// Postgres LISTEN/NOTIFY adapter for real-time document change fan-out.
///
/// Step 05-02: one `PostgresNotifyListener` per project, started on first Listen
/// stream. Uses `sqlx::postgres::PgListener` on a dedicated connection (not pool).
use std::sync::Arc;

use sqlx::{postgres::PgListener, PgPool};

use embyr_core::domain::{
    document::DocumentPath,
    project::ProjectId,
};

use crate::realtime::listen_registry::{ListenEvent, ListenRegistry};

// notify_channel is now provided by embyr-pg-storage.
pub use embyr_pg_storage::notify_listener::notify_channel;

/// Background LISTEN/NOTIFY task handle.
///
/// Dropping this struct cancels the background task via the JoinHandle abort.
pub struct PostgresNotifyListener {
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for PostgresNotifyListener {
    fn drop(&mut self) {
        self._task.abort();
    }
}

impl PostgresNotifyListener {
    /// Start a background task that LISTENs on the project's channel and fans
    /// out events to all registered subscribers in the registry.
    ///
    /// `dsn` — the customer DSN (same as used by `PostgresBackendAdapter`).
    /// `project_id` — used to compute the channel name.
    /// `registry` — shared fan-out registry.
    /// `backend_pool` — pool for fetching the document after NOTIFY.
    pub async fn start(
        dsn: &str,
        project_id: &str,
        registry: Arc<ListenRegistry>,
        backend_pool: PgPool,
    ) -> Result<Self, embyr_core::error::CoreError> {
        let channel = notify_channel(project_id);
        let project_id = project_id.to_string();

        let mut pg_listener = PgListener::connect(dsn)
            .await
            .map_err(|e| embyr_core::error::CoreError::BackendUnavailable(e.to_string()))?;
        pg_listener
            .listen(&channel)
            .await
            .map_err(|e| embyr_core::error::CoreError::BackendUnavailable(e.to_string()))?;

        let task = tokio::spawn(async move {
            loop {
                match pg_listener.recv().await {
                    Ok(notification) => {
                        let payload = notification.payload().to_string();
                        // payload format: "collection_path/document_id"
                        let event = fetch_event(&backend_pool, &project_id, &payload).await;
                        registry.fan_out(&channel, event).await;
                    }
                    Err(e) => {
                        tracing::warn!("PgListener error on channel {channel}: {e}");
                        break;
                    }
                }
            }
        });

        Ok(Self { _task: task })
    }
}

/// Fetch the document for a NOTIFY payload and produce a `ListenEvent`.
///
/// Payload format: `{collection_path}/{document_id}`.
/// If the document is not found (deleted), returns `ListenEvent::Removed`.
async fn fetch_event(pool: &PgPool, project_id: &str, payload: &str) -> ListenEvent {
    // Split payload into collection_path and document_id.
    // The document_id is the last segment; collection_path is everything before.
    let (collection_path, document_id) = match payload.rsplit_once('/') {
        Some((c, d)) => (c.to_string(), d.to_string()),
        None => {
            // Malformed payload — emit Removed with a best-effort path.
            let pid = ProjectId::new(project_id).unwrap_or_else(|_| ProjectId(project_id.to_string()));
            return ListenEvent::Removed(DocumentPath {
                project_id: pid,
                collection_path: String::new(),
                document_id: payload.to_string(),
            });
        }
    };

    let pid = ProjectId::new(project_id).unwrap_or_else(|_| ProjectId(project_id.to_string()));

    let doc_path = DocumentPath {
        project_id: pid,
        collection_path: collection_path.clone(),
        document_id: document_id.clone(),
    };

    // Query the document from the pool.
    use sqlx::Row;
    let row_opt = sqlx::query(
        "SELECT fields, version, create_time, update_time \
         FROM documents \
         WHERE project_id = $1 \
           AND collection_path = $2 \
           AND document_id = $3 \
           AND NOT deleted",
    )
    .bind(project_id)
    .bind(&collection_path)
    .bind(&document_id)
    .fetch_optional(pool)
    .await;

    match row_opt {
        Ok(Some(row)) => {
            use chrono::{DateTime, Utc};
            let fields_json: serde_json::Value = row
                .try_get("fields")
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            let version: i64 = row.try_get("version").unwrap_or(0);
            let create_time: DateTime<Utc> =
                row.try_get("create_time").unwrap_or_else(|_| Utc::now());
            let update_time: DateTime<Utc> =
                row.try_get("update_time").unwrap_or_else(|_| Utc::now());

            let fields = crate::encoding::field_value::json_to_fields(&fields_json)
                .unwrap_or_default();

            ListenEvent::Changed(embyr_core::domain::document::FirestoreDocument {
                path: doc_path,
                fields,
                create_time: (
                    create_time.timestamp(),
                    create_time.timestamp_subsec_nanos() as i32,
                ),
                update_time: (
                    update_time.timestamp(),
                    update_time.timestamp_subsec_nanos() as i32,
                ),
                version,
            })
        }
        _ => ListenEvent::Removed(doc_path),
    }
}

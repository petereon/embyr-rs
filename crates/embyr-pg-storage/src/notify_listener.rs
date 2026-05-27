use sqlx::postgres::PgListener;
use tokio::sync::mpsc;
use embyr_core::error::CoreError;

/// Compute the Postgres NOTIFY channel name for a project.
///
/// Format: `dc_<16 lowercase hex chars>` — BLAKE3 hash of project_id, first 8 bytes.
/// Total length: 19 chars (well within Postgres 63-char identifier limit).
pub fn notify_channel(project_id: &str) -> String {
    let hash = blake3::hash(project_id.as_bytes());
    let bytes = hash.as_bytes();
    let hex: String = bytes[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("dc_{hex}")
}

/// Generic Postgres NOTIFY listener. Sends raw payload strings to a channel.
///
/// The payload format is `{collection_path}/{document_id}`.
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
    /// Start a background task that LISTENs on the project's notify channel
    /// and forwards raw payload strings to the provided sender.
    ///
    /// `dsn` — the customer DSN (same as used by `PostgresBackendAdapter`).
    /// `project_id` — used to compute the channel name.
    /// `tx` — sender half of an mpsc channel; receiver gets raw payload strings.
    pub async fn start(
        dsn: &str,
        project_id: &str,
        tx: mpsc::Sender<String>,
    ) -> Result<Self, CoreError> {
        let channel = notify_channel(project_id);
        let mut listener = PgListener::connect(dsn)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        listener
            .listen(&channel)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        let task = tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(n) => {
                        if tx.send(n.payload().to_string()).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        tracing::warn!("PgListener error on {channel}: {e}");
                        break;
                    }
                }
            }
        });
        Ok(Self { _task: task })
    }
}

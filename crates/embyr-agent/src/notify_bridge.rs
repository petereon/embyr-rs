//! AgentNotifyBridge — translates Postgres NOTIFY payloads into DocChange events.
//!
//! Wraps sqlx PgListener. Exposes `subscribe()` returning a capacity-64 mpsc
//! Receiver<DocChange>. Overflow detection: when the channel is full, the next
//! DocChange delivered carries kind=RESET to signal resync.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use sqlx::PgPool;
use tokio::sync::mpsc;

use embyr_proto::agent::{DocChange, DocChangeKind};
use embyr_pg_storage::notify_listener::notify_channel;

/// Bridges Postgres NOTIFY events to a DocChange mpsc channel.
///
/// Call `subscribe()` to get a `Receiver<DocChange>` that yields change events
/// for the project's notify channel. Channel capacity is 64. When the channel
/// fills, the overflow flag is set and the next DocChange delivered has
/// `kind = RESET`, signalling the consumer to resync.
pub struct AgentNotifyBridge {
    pool: PgPool,
    project_id: String,
}

impl AgentNotifyBridge {
    /// Construct a new bridge for the given project.
    pub fn new(pool: PgPool, project_id: String) -> Self {
        Self { pool, project_id }
    }

    /// Subscribe to real-time DocChange events for the project.
    ///
    /// Returns a `Receiver<DocChange>` with capacity 64. Spawns a background
    /// task that listens on the Postgres NOTIFY channel and translates each raw
    /// payload into a DocChange proto message.
    ///
    /// Overflow policy: if the mpsc channel is full when a notification arrives,
    /// the overflow flag is set. On the next notification, a `kind=RESET`
    /// DocChange is sent first (clearing the flag), followed by the real change.
    pub async fn subscribe(
        &self,
    ) -> Result<mpsc::Receiver<DocChange>, Box<dyn std::error::Error + Send + Sync>> {
        let (tx, rx) = mpsc::channel::<DocChange>(64);
        let pool = self.pool.clone();
        let project_id = self.project_id.clone();
        let overflow = Arc::new(AtomicBool::new(false));

        // LISTEN must be registered before subscribe() returns: a NOTIFY sent
        // by a caller right after subscribe() (e.g. immediately following a
        // write) is lost forever if the session isn't listening yet. Mirrors
        // the connect-then-listen-before-spawn pattern in
        // embyr-pg-storage::notify_listener::PostgresNotifyListener::start.
        let channel_name = notify_channel(&project_id);
        let mut pg_listener = sqlx::postgres::PgListener::connect_with(&pool).await?;
        pg_listener.listen(&channel_name).await?;

        tokio::spawn(async move {
            loop {
                match pg_listener.recv().await {
                    Ok(notification) => {
                        // If overflow was set, deliver a RESET first.
                        if overflow.swap(false, Ordering::SeqCst) {
                            let reset = DocChange {
                                kind: DocChangeKind::Reset as i32,
                                ..Default::default()
                            };
                            if tx.send(reset).await.is_err() {
                                break;
                            }
                        }

                        // Translate raw payload into a DocChange.
                        let change =
                            translate_payload(notification.payload(), &project_id, &pool).await;

                        // try_send: non-blocking; if channel full, set overflow.
                        if tx.try_send(change).is_err() {
                            overflow.store(true, Ordering::SeqCst);
                        }
                    }
                    Err(e) => {
                        tracing::error!("AgentNotifyBridge: PgListener error: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

/// Translate a raw NOTIFY payload string into a DocChange.
///
/// Payload format: `{collection_path}/{document_id}` (see backend_adapter.rs).
/// Looks up the document from Postgres to obtain generation and fields.
/// Returns UPSERT if found, DELETE if absent.
async fn translate_payload(payload: &str, project_id: &str, pool: &PgPool) -> DocChange {
    let (collection_path, document_id) = match payload.rsplit_once('/') {
        Some((c, d)) => (c.to_string(), d.to_string()),
        None => {
            // Malformed payload — treat as a RESET.
            return DocChange {
                kind: DocChangeKind::Reset as i32,
                ..Default::default()
            };
        }
    };

    let document_name = format!(
        "projects/{project_id}/databases/(default)/documents/{collection_path}/{document_id}"
    );

    // Query the document to determine kind, generation, and fields. A delete
    // is a soft delete that keeps `fields` (so the server can evaluate a read
    // rule on the old document): select the row whether or not it is deleted.
    use sqlx::Row;
    let row_opt = sqlx::query(
        "SELECT fields, version, deleted \
         FROM documents \
         WHERE project_id = $1 \
           AND collection_path = $2 \
           AND document_id = $3",
    )
    .bind(project_id)
    .bind(&collection_path)
    .bind(&document_id)
    .fetch_optional(pool)
    .await;

    match row_opt {
        Ok(Some(row)) => {
            let version: i64 = row.try_get("version").unwrap_or(0);
            let deleted: bool = row.try_get("deleted").unwrap_or(false);
            let fields_json: serde_json::Value = row
                .try_get("fields")
                .unwrap_or(serde_json::Value::Object(Default::default()));
            doc_change_for_row(document_name, version, deleted, &fields_json)
        }
        _ => {
            // Row absent (purged) or query error — DELETE with unknown old fields.
            DocChange {
                kind: DocChangeKind::Delete as i32,
                document_name,
                ..Default::default()
            }
        }
    }
}

/// The DocChange for a document row: UPSERT with its fields, or, for a
/// soft-deleted row, DELETE carrying the pre-delete fields as `old_fields`.
fn doc_change_for_row(
    document_name: String,
    version: i64,
    deleted: bool,
    fields_json: &serde_json::Value,
) -> DocChange {
    let proto_fields = json_fields_to_proto(fields_json);
    if deleted {
        DocChange {
            kind: DocChangeKind::Delete as i32,
            document_name,
            old_fields: proto_fields,
            ..Default::default()
        }
    } else {
        DocChange {
            kind: DocChangeKind::Upsert as i32,
            document_name,
            generation: version,
            fields: proto_fields,
            ..Default::default()
        }
    }
}

/// Convert a stored `fields` JSON object to a proto `map<string, Value>`.
///
/// The `documents.fields` column holds the storage crate's discriminated-union
/// encoding (`{"t":"S","v":"x"}`), so decode it with the storage crate's own
/// decoder (the one `get_document` uses) and translate the domain values. A
/// shape that does not decode yields an EMPTY map (unknown), never a guess:
/// the server treats empty `old_fields` as "unknown" and fails closed.
fn json_fields_to_proto(
    fields_json: &serde_json::Value,
) -> std::collections::HashMap<String, embyr_proto::agent::Value> {
    embyr_pg_storage::encoding::field_value::json_to_fields(fields_json)
        .unwrap_or_default()
        .iter()
        .map(|(k, v)| (k.clone(), crate::encoding::field_value_to_proto(v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_row_is_an_upsert_with_fields_and_generation() {
        let c = doc_change_for_row("d".into(), 4, false, &serde_json::json!({"n": {"t": "I", "v": 1}}));
        assert_eq!(c.kind, DocChangeKind::Upsert as i32);
        assert_eq!(c.generation, 4);
        assert_eq!(c.fields.len(), 1);
        assert!(c.old_fields.is_empty());
    }

    #[test]
    fn soft_deleted_row_is_a_delete_carrying_its_pre_delete_fields() {
        // Stored shape: the storage crate's discriminated union, not plain JSON.
        let stored = serde_json::json!({"owner": {"t": "S", "v": "u1"}});
        let c = doc_change_for_row("d".into(), 5, true, &stored);
        assert_eq!(c.kind, DocChangeKind::Delete as i32);
        assert!(c.fields.is_empty(), "fields stays empty for a delete");
        let owner = c.old_fields.get("owner").expect("owner");
        assert_eq!(
            owner.value_type,
            Some(embyr_proto::agent::value::ValueType::StringValue("u1".into())),
            "decoded to a string value, not a nested map of the storage tags"
        );
    }

    #[test]
    fn undecodable_stored_fields_are_unknown_not_guessed() {
        let c = doc_change_for_row("d".into(), 5, true, &serde_json::json!({"owner": "plain"}));
        assert!(c.old_fields.is_empty());
    }
}

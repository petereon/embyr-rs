/// Postgres LISTEN/NOTIFY adapter for real-time document change fan-out.
///
/// Step 05-02: one `PostgresNotifyListener` per project, started on first Listen
/// stream. Uses `sqlx::postgres::PgListener` on a dedicated connection (not pool).
///
/// realtime-listener-reconnect (ADR-071, amended during DELIVER — see
/// `reconnect_pg_listener` doc comment below): the background task never
/// `break`s on a `recv()` error. It applies a capped exponential backoff,
/// then re-establishes the listener with a fresh connection and re-issues
/// `LISTEN`, rather than assuming `sqlx`'s own internal auto-reconnect always
/// recovers the existing binding.
use std::sync::Arc;

use sqlx::{postgres::PgListener, PgPool};

use embyr_core::domain::{
    document::DocumentPath,
    project::ProjectId,
};

use crate::realtime::listen_registry::{ListenEvent, ListenRegistry};

// notify_channel is now provided by embyr-pg-storage.
pub use embyr_pg_storage::notify_listener::notify_channel;

const RECONNECT_INITIAL_BACKOFF_SECS: u32 = 1;
const RECONNECT_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(30);

/// Consecutive-failure count (1-indexed) at which a listener's sustained
/// failure becomes operator-visible (~15s of backoff already elapsed:
/// 1+2+4+8s before the 5th failure).
const RECONNECT_ALERT_THRESHOLD: u32 = 5;

/// Hard wall-clock cap on a single reconnect attempt (`reconnect_pg_listener`).
/// Guards against a `connect()` call itself hanging indefinitely against a
/// target in a "blackhole" state (SYN packets silently dropped rather than
/// actively refused — empirically observed during DELIVER against a
/// stopped-but-not-yet-restarted `testcontainers` Postgres instance, where
/// `PgPoolOptions::connect`'s own initial connection is NOT bounded by
/// `acquire_timeout`). Without this, a single stuck attempt could starve
/// this loop's own backoff schedule of control far longer than any
/// configured backoff interval.
const RECONNECT_ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Exponential backoff for `PostgresNotifyListener`'s reconnect loop: 1s, 2s,
/// 4s, 8s, 16s, then capped at 30s. `consecutive_failures` is 1-indexed (the
/// value immediately after the Nth consecutive `recv()` error).
///
/// ponytail: no jitter — each project's `PgListener` targets a dedicated,
/// per-tenant Postgres instance (AD-08), so one project's retry schedule is
/// never correlated with another's. Add per-project jitter only if a future
/// backend_mode ever shares one Postgres instance across multiple projects'
/// listeners.
fn reconnect_backoff(consecutive_failures: u32) -> std::time::Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5); // 2^5s = 32s, capped at 30s below
    std::time::Duration::from_secs(u64::from(RECONNECT_INITIAL_BACKOFF_SECS) << shift)
        .min(RECONNECT_MAX_BACKOFF)
}

/// Establish a fresh dedicated (AD-08) `PgListener` connection and re-issue
/// `LISTEN` for `channel`.
///
/// ADR-071 amendment (DELIVER, empirically forced): ADR-071 originally
/// assumed `sqlx::postgres::PgListener`'s own internal auto-reconnect (see
/// its doc comment) was sufficient and that no new connection should ever be
/// created on a reconnect cycle. Reading `sqlx-postgres-0.8.6`'s
/// `try_recv()` source directly during DELIVER shows that internal
/// auto-reconnect only clears its stale connection and retries for FOUR
/// specific IO error kinds: `ConnectionAborted`, `UnexpectedEof`,
/// `TimedOut`, `BrokenPipe`. Any other error kind — empirically confirmed
/// during DELIVER via a real Postgres container kill/restart cycle to
/// reliably surface `io::ErrorKind::ConnectionReset` (a forcibly-terminated
/// connection, e.g. a killed Postgres backend process) — is forwarded
/// as-is, and `PgListener` never clears its own dead connection handle. Its
/// own `recv()`/`try_recv()` would then retry the SAME broken connection
/// forever, never recovering, for that error class. Building a fresh
/// connection here (short `acquire_timeout` so a still-unreachable target
/// fails fast, keeping OUR OWN `reconnect_backoff` schedule — not sqlx's
/// default 30s pool-acquire timeout — in control of retry cadence) makes
/// recovery independent of which IO error kind a given outage happens to
/// surface as. Still exactly one dedicated connection (AD-08) alive at any
/// time — the old, broken `PgListener` is dropped (see call site) before
/// this replacement is constructed, never accumulating.
async fn reconnect_pg_listener(dsn: &str, channel: &str) -> Result<PgListener, sqlx::Error> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(dsn)
        .await?;
    let mut pg_listener = PgListener::connect_with(&pool).await?;
    pg_listener.listen(channel).await?;
    Ok(pg_listener)
}

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
        let dsn = dsn.to_string();

        let mut pg_listener = reconnect_pg_listener(&dsn, &channel)
            .await
            .map_err(|e| embyr_core::error::CoreError::BackendUnavailable(e.to_string()))?;

        let task = tokio::spawn(async move {
            let mut consecutive_failures: u32 = 0;
            loop {
                match pg_listener.recv().await {
                    Ok(notification) => {
                        if consecutive_failures >= RECONNECT_ALERT_THRESHOLD {
                            tracing::info!(
                                project_id = %project_id,
                                channel = %channel,
                                attempts = consecutive_failures,
                                "postgres_notify_listener_recovered"
                            );
                            metrics::gauge!(
                                "embyr_pg_notify_listener_reconnecting",
                                "project_id" => project_id.clone()
                            )
                            .set(0.0);
                        }
                        consecutive_failures = 0;

                        let payload = notification.payload().to_string();
                        // payload format: "collection_path/document_id"
                        let event = fetch_event(&backend_pool, &project_id, &payload).await;
                        registry.fan_out(&channel, event).await;
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        metrics::counter!(
                            "embyr_pg_notify_listener_reconnect_attempts_total",
                            "project_id" => project_id.clone()
                        )
                        .increment(1);

                        if consecutive_failures == RECONNECT_ALERT_THRESHOLD {
                            tracing::error!(
                                project_id = %project_id,
                                channel = %channel,
                                attempts = consecutive_failures,
                                error = %e,
                                "postgres_notify_listener_sustained_failure"
                            );
                            metrics::gauge!(
                                "embyr_pg_notify_listener_reconnecting",
                                "project_id" => project_id.clone()
                            )
                            .set(1.0);
                        } else {
                            tracing::warn!(
                                project_id = %project_id,
                                channel = %channel,
                                attempt = consecutive_failures,
                                error = %e,
                                "postgres_notify_listener_recv_error"
                            );
                        }

                        // Force a fresh connection (see reconnect_pg_listener's
                        // own doc comment for why sqlx's own internal
                        // auto-reconnect cannot be relied on for every error
                        // kind) — attempted IMMEDIATELY, before any backoff
                        // sleep. Postgres NOTIFY is fire-and-forget with no
                        // replay for a not-yet-subscribed listener: sleeping
                        // BEFORE the first reconnect attempt after a failure
                        // would risk re-issuing LISTEN only after a write
                        // that occurred while connectivity was already
                        // restored, permanently missing that write's
                        // notification (empirically confirmed during
                        // DELIVER).
                        //
                        // Amendment (orchestrator, empirically forced): for
                        // any error kind OTHER than the 4 `sqlx` itself
                        // clears (see `reconnect_pg_listener`'s own doc
                        // comment), `PgListener` never clears its own
                        // `self.connection` — so `pg_listener` here is
                        // POISONED after this branch's `recv()` error, not
                        // merely "the old one." Looping back to
                        // `pg_listener.recv().await` on it again (as an
                        // earlier version of this function did on a FAILED
                        // reconnect attempt) reuses that same poisoned
                        // connection handle, which was empirically observed
                        // to hang indefinitely rather than error again
                        // (confirmed via a real Postgres container
                        // kill/restart reproduction against this exact
                        // build). So: retry `reconnect_pg_listener` in this
                        // inner loop — with backoff between FAILED attempts,
                        // never before the first (AC-RLR-03 bounds the
                        // attempt rate; a successful reconnect never waits)
                        // — until it succeeds, and only then resume the
                        // outer `.recv()` loop on the fresh listener. The
                        // outer loop's `pg_listener` binding is never read
                        // again while in this state.
                        loop {
                            match tokio::time::timeout(
                                RECONNECT_ATTEMPT_TIMEOUT,
                                reconnect_pg_listener(&dsn, &channel),
                            )
                            .await
                            {
                                Ok(Ok(fresh)) => {
                                    pg_listener = fresh;
                                    break;
                                }
                                Ok(Err(reconnect_err)) => {
                                    tracing::warn!(
                                        project_id = %project_id,
                                        channel = %channel,
                                        error = %reconnect_err,
                                        "postgres_notify_listener_reconnect_attempt_failed"
                                    );
                                }
                                Err(_elapsed) => {
                                    tracing::warn!(
                                        project_id = %project_id,
                                        channel = %channel,
                                        "postgres_notify_listener_reconnect_attempt_timed_out"
                                    );
                                }
                            }
                            consecutive_failures += 1;
                            metrics::counter!(
                                "embyr_pg_notify_listener_reconnect_attempts_total",
                                "project_id" => project_id.clone()
                            )
                            .increment(1);
                            if consecutive_failures == RECONNECT_ALERT_THRESHOLD {
                                tracing::error!(
                                    project_id = %project_id,
                                    channel = %channel,
                                    attempts = consecutive_failures,
                                    "postgres_notify_listener_sustained_failure"
                                );
                                metrics::gauge!(
                                    "embyr_pg_notify_listener_reconnecting",
                                    "project_id" => project_id.clone()
                                )
                                .set(1.0);
                            }
                            tokio::time::sleep(reconnect_backoff(consecutive_failures)).await;
                        }
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
///
/// security-rules-realtime (ADR-033 § Decision — Delete Non-Leakage, US-05):
/// the query is widened to also match soft-deleted rows and select the
/// `deleted` column, branching in Rust — a soft-deleted row still carries
/// its pre-deletion `fields`, needed by `evaluate()` (see
/// `listen_handler.rs`'s `Removed` arm) to decide delete-event delivery
/// without leaking existence. Zero additional round-trip: this query
/// already ran on every delete NOTIFY before this change.
async fn fetch_event(pool: &PgPool, project_id: &str, payload: &str) -> ListenEvent {
    // Split payload into collection_path and document_id.
    // The document_id is the last segment; collection_path is everything before.
    let (collection_path, document_id) = match payload.rsplit_once('/') {
        Some((c, d)) => (c.to_string(), d.to_string()),
        None => {
            // Malformed payload — emit Removed with a best-effort path.
            let pid = ProjectId::new(project_id).unwrap_or_else(|_| ProjectId(project_id.to_string()));
            return ListenEvent::Removed {
                path: DocumentPath {
                    project_id: pid,
                    collection_path: String::new(),
                    document_id: payload.to_string(),
                },
                fields: std::collections::BTreeMap::new(),
            };
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
        "SELECT fields, version, create_time, update_time, deleted \
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
            use chrono::{DateTime, Utc};
            let fields_json: serde_json::Value = row
                .try_get("fields")
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
            let deleted: bool = row.try_get("deleted").unwrap_or(false);
            let fields = crate::encoding::field_value::json_to_fields(&fields_json)
                .unwrap_or_default();

            if deleted {
                return ListenEvent::Removed { path: doc_path, fields };
            }

            let version: i64 = row.try_get("version").unwrap_or(0);
            let create_time: DateTime<Utc> =
                row.try_get("create_time").unwrap_or_else(|_| Utc::now());
            let update_time: DateTime<Utc> =
                row.try_get("update_time").unwrap_or_else(|_| Utc::now());

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
        // Row genuinely absent — defensive, should not occur for a
        // NOTIFY-triggered event (documents never issue a hard DELETE).
        _ => ListenEvent::Removed { path: doc_path, fields: std::collections::BTreeMap::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::{reconnect_backoff, RECONNECT_MAX_BACKOFF};
    use proptest::prelude::*;
    use std::time::Duration;

    proptest! {
        /// Behavior 1 (Invariant): backoff never exceeds the 30s cap, for any
        /// consecutive-failure count including 0 and pathologically large values.
        #[test]
        fn never_exceeds_the_cap(consecutive_failures in any::<u32>()) {
            prop_assert!(reconnect_backoff(consecutive_failures) <= RECONNECT_MAX_BACKOFF);
        }

        /// Behavior 2 (Invariant): backoff is monotonically non-decreasing as
        /// consecutive_failures increases — later failures never wait less
        /// than earlier ones (a prerequisite for "capped exponential", not
        /// merely "bounded").
        #[test]
        fn monotonically_non_decreasing(consecutive_failures in 1u32..1000) {
            let earlier = reconnect_backoff(consecutive_failures);
            let later = reconnect_backoff(consecutive_failures + 1);
            prop_assert!(later >= earlier);
        }
    }

    /// Behavior 3 (Generalizing example, Hebert ch.3): the documented curve's
    /// first five values are exactly 1s, 2s, 4s, 8s, 16s, then capped at 30s
    /// from the 6th failure on — pinning ADR-071's own numeric contract.
    #[test]
    fn matches_documented_curve() {
        let expected = [1, 2, 4, 8, 16, 30, 30];
        for (i, secs) in expected.iter().enumerate() {
            let consecutive_failures = (i + 1) as u32;
            assert_eq!(
                reconnect_backoff(consecutive_failures),
                Duration::from_secs(*secs),
                "consecutive_failures={consecutive_failures}"
            );
        }
    }
}

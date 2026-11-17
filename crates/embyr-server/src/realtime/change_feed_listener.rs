//! Per-project change source for backends the server cannot `LISTEN` on
//! directly (`backend_mode=agent`): consumes the adapter's own
//! `ChangeFeed` and fans events out through the same `ListenRegistry`
//! channel the direct-Postgres `PostgresNotifyListener` uses, so Listen
//! sessions cannot tell the two sources apart.
//!
//! Reconnect mirrors `postgres_notify_listener.rs`: capped exponential
//! backoff (shared `reconnect_backoff`), first attempt immediate, per-attempt
//! timeout. Unlike Postgres NOTIFY, a re-opened feed has no memory of events
//! sent while it was down, so every successful reconnect RESETs the channel's
//! subscribers (they resync from a fresh snapshot).

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use embyr_core::{
    domain::{document::DocumentPath, project::ProjectId},
    error::CoreError,
    storage::change_feed::{ChangeEvent, ChangeFeed},
};

use crate::{
    adapters::{
        credential_cache::SharedBackendAdapter,
        postgres_notify_listener::{notify_channel, reconnect_backoff},
    },
    realtime::listen_registry::{ListenEvent, ListenRegistry},
};

const RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// Same threshold as `postgres_notify_listener`: ~15s of backoff elapsed.
const RECONNECT_ALERT_THRESHOLD: u32 = 5;

/// Background task handle; dropping it aborts the task.
pub struct ChangeFeedListener {
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for ChangeFeedListener {
    fn drop(&mut self) {
        self._task.abort();
    }
}

impl ChangeFeedListener {
    /// Open the adapter's change feed and start fanning its events out.
    /// `Ok(None)`: the adapter has no feed (use the direct Postgres listener).
    /// `Err`: the feed could not be opened (Listen should fail, as it does
    /// when the direct listener cannot connect).
    pub async fn start(
        adapter: SharedBackendAdapter,
        project_id: &str,
        registry: Arc<ListenRegistry>,
    ) -> Result<Option<Self>, CoreError> {
        let pid = ProjectId::new(project_id).map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
        let Some(feed) = adapter.open_change_feed(&pid).await? else {
            return Ok(None);
        };
        let task = tokio::spawn(run(adapter, pid, registry, feed));
        Ok(Some(Self { _task: task }))
    }
}

async fn run(
    adapter: SharedBackendAdapter,
    pid: ProjectId,
    registry: Arc<ListenRegistry>,
    mut feed: Box<dyn ChangeFeed>,
) {
    let project_id = pid.as_str().to_string();
    let channel = notify_channel(&project_id);
    loop {
        match feed.next().await {
            Ok(ChangeEvent::Changed(path)) => {
                let event = match adapter.get_document(&path, None).await {
                    Ok(Some(doc)) => ListenEvent::Changed(doc),
                    // Gone by the time we looked: a delete (no pre-delete fields available).
                    Ok(None) => removed(path),
                    Err(e) => {
                        // Unknown state of this document: resync rather than guess.
                        tracing::warn!(project_id = %project_id, error = %e, "change_feed_fetch_failed");
                        registry.reset(&channel).await;
                        continue;
                    }
                };
                registry.fan_out(&channel, event).await;
            }
            Ok(ChangeEvent::Deleted { path, old_fields }) => {
                registry.fan_out(&channel, ListenEvent::Removed { path, fields: old_fields }).await
            }
            Ok(ChangeEvent::Reset) => registry.reset(&channel).await,
            Err(e) => {
                tracing::warn!(project_id = %project_id, error = %e, "change_feed_recv_error");
                feed = reconnect(&adapter, &pid).await;
                registry.reset(&channel).await;
            }
        }
    }
}

// A delete whose pre-delete fields are unknown (`Changed` that re-fetched to
// nothing, or an agent that predates `DocChange.old_fields`) carries empty
// `fields`: delivered only where the collection's rule does not depend on the
// deleted document's data; rule-gated deletes are withheld (fail closed).
fn removed(path: DocumentPath) -> ListenEvent {
    ListenEvent::Removed { path, fields: BTreeMap::new() }
}

/// Retry `open_change_feed` until it succeeds (first attempt immediate).
async fn reconnect(adapter: &SharedBackendAdapter, pid: &ProjectId) -> Box<dyn ChangeFeed> {
    let project_id = pid.as_str();
    let mut attempt: u32 = 0;
    loop {
        if attempt > 0 {
            tokio::time::sleep(reconnect_backoff(attempt)).await;
        }
        attempt += 1;
        // Same metric names as the Postgres listener so existing alerts cover agent mode.
        metrics::counter!(
            "embyr_pg_notify_listener_reconnect_attempts_total",
            "project_id" => project_id.to_string()
        )
        .increment(1);
        match tokio::time::timeout(RECONNECT_ATTEMPT_TIMEOUT, adapter.open_change_feed(pid)).await {
            Ok(Ok(Some(feed))) => {
                if attempt > RECONNECT_ALERT_THRESHOLD {
                    tracing::info!(project_id, attempts = attempt, "change_feed_recovered");
                }
                metrics::gauge!(
                    "embyr_pg_notify_listener_reconnecting",
                    "project_id" => project_id.to_string()
                )
                .set(0.0);
                return feed;
            }
            Ok(Ok(None)) => tracing::warn!(project_id, "change_feed_reconnect_unsupported"),
            Ok(Err(e)) => tracing::warn!(project_id, error = %e, "change_feed_reconnect_attempt_failed"),
            Err(_) => tracing::warn!(project_id, "change_feed_reconnect_attempt_timed_out"),
        }
        if attempt == RECONNECT_ALERT_THRESHOLD {
            tracing::error!(project_id, attempts = attempt, "change_feed_sustained_failure");
            metrics::gauge!(
                "embyr_pg_notify_listener_reconnecting",
                "project_id" => project_id.to_string()
            )
            .set(1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use async_trait::async_trait;
    use embyr_core::{
        domain::{
            document::{CollectionPath, FirestoreDocument, WriteResult},
            field_value::FieldValue,
            query::StructuredQuery,
            transaction::{TransactionId, TransactionOptions},
        },
        storage::backend_adapter::{BackendAdapter, Write, WritePrecondition},
    };

    use super::*;

    type Script = Result<Vec<Result<ChangeEvent, CoreError>>, CoreError>;

    /// Hand-written fake Subscribe source: each `open_change_feed` call pops
    /// the next scripted open result; a feed yields its scripted events, then
    /// stays quiet (like a healthy idle stream).
    #[derive(Default)]
    struct FakeAdapter {
        opens: Mutex<VecDeque<Script>>,
        open_count: Mutex<u32>,
        docs: Mutex<BTreeMap<String, FirestoreDocument>>,
    }

    struct ScriptedFeed(VecDeque<Result<ChangeEvent, CoreError>>);

    #[async_trait]
    impl ChangeFeed for ScriptedFeed {
        async fn next(&mut self) -> Result<ChangeEvent, CoreError> {
            match self.0.pop_front() {
                Some(r) => r,
                None => std::future::pending().await,
            }
        }
    }

    fn down() -> CoreError {
        CoreError::BackendUnavailable("down".into())
    }

    #[async_trait]
    impl BackendAdapter for FakeAdapter {
        async fn get_document(
            &self,
            path: &DocumentPath,
            _t: Option<&TransactionId>,
        ) -> Result<Option<FirestoreDocument>, CoreError> {
            Ok(self.docs.lock().unwrap().get(&path.document_id).cloned())
        }
        async fn open_change_feed(
            &self,
            _p: &ProjectId,
        ) -> Result<Option<Box<dyn ChangeFeed>>, CoreError> {
            *self.open_count.lock().unwrap() += 1;
            match self.opens.lock().unwrap().pop_front() {
                Some(Ok(events)) => Ok(Some(Box::new(ScriptedFeed(events.into())))),
                Some(Err(e)) => Err(e),
                None => Err(down()),
            }
        }
        async fn create_document(&self, _: &DocumentPath, _: BTreeMap<String, FieldValue>) -> Result<WriteResult, CoreError> { unimplemented!() }
        async fn update_document(&self, _: &DocumentPath, _: BTreeMap<String, FieldValue>, _: Option<WritePrecondition>) -> Result<WriteResult, CoreError> { unimplemented!() }
        async fn delete_document(&self, _: &DocumentPath, _: Option<WritePrecondition>) -> Result<(), CoreError> { unimplemented!() }
        async fn run_query(&self, _: &CollectionPath, _: &StructuredQuery, _: Option<&TransactionId>) -> Result<Vec<FirestoreDocument>, CoreError> { unimplemented!() }
        async fn begin_transaction(&self, _: &ProjectId, _: TransactionOptions) -> Result<TransactionId, CoreError> { unimplemented!() }
        async fn commit_transaction(&self, _: &ProjectId, _: &TransactionId, _: Vec<Write>) -> Result<Vec<WriteResult>, CoreError> { unimplemented!() }
        async fn rollback_transaction(&self, _: &ProjectId, _: &TransactionId) -> Result<(), CoreError> { unimplemented!() }
        async fn probe(&self) -> Result<(), CoreError> { Ok(()) }
    }

    fn path(id: &str) -> DocumentPath {
        DocumentPath {
            project_id: ProjectId::new("proj").unwrap(),
            collection_path: "orders".into(),
            document_id: id.into(),
        }
    }

    fn doc(id: &str) -> FirestoreDocument {
        FirestoreDocument {
            path: path(id),
            fields: BTreeMap::from([("n".to_string(), FieldValue::Integer(1))]),
            create_time: (1, 0),
            update_time: (2, 0),
            version: 3,
        }
    }

    fn fake(opens: Vec<Script>, docs: &[&str]) -> Arc<FakeAdapter> {
        let f = FakeAdapter::default();
        *f.opens.lock().unwrap() = opens.into();
        for d in docs {
            f.docs.lock().unwrap().insert((*d).into(), doc(d));
        }
        Arc::new(f)
    }

    async fn next(rx: &mut tokio::sync::mpsc::Receiver<ListenEvent>) -> ListenEvent {
        tokio::time::timeout(Duration::from_secs(120), rx.recv()).await.unwrap().unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn feed_events_fan_out_to_registered_subscribers() {
        let adapter = fake(
            vec![Ok(vec![
                Ok(ChangeEvent::Changed(path("a"))),
                Ok(ChangeEvent::Changed(path("gone"))),
                Ok(ChangeEvent::Deleted { path: path("b"), old_fields: BTreeMap::new() }),
                Ok(ChangeEvent::Deleted {
                    path: path("c"),
                    old_fields: BTreeMap::from([("n".to_string(), FieldValue::Integer(7))]),
                }),
            ])],
            &["a"],
        );
        let registry = ListenRegistry::new();
        let mut sub = registry.register(&notify_channel("proj")).await;
        let _l = ChangeFeedListener::start(adapter, "proj", Arc::clone(&registry)).await.unwrap().unwrap();

        match next(&mut sub.event_rx).await {
            ListenEvent::Changed(d) => assert_eq!((d.path.document_id.as_str(), d.version), ("a", 3)),
            e => panic!("expected Changed, got {e:?}"),
        }
        // `gone` and `b`: pre-delete fields unknown. `c`: the agent supplied them.
        for (want, old_n) in [("gone", None), ("b", None), ("c", Some(7))] {
            match next(&mut sub.event_rx).await {
                ListenEvent::Removed { path, fields } => {
                    assert_eq!(path.document_id, want);
                    assert_eq!(fields.get("n"), old_n.map(FieldValue::Integer).as_ref());
                }
                e => panic!("expected Removed, got {e:?}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn broken_feed_reconnects_with_backoff_then_resets_and_resumes() {
        let adapter = fake(
            vec![
                Ok(vec![Ok(ChangeEvent::Changed(path("a"))), Err(down())]),
                Err(down()), // first reconnect attempt fails
                Ok(vec![Ok(ChangeEvent::Changed(path("b")))]),
            ],
            &["a", "b"],
        );
        let registry = ListenRegistry::new();
        let mut sub = registry.register(&notify_channel("proj")).await;
        let _l = ChangeFeedListener::start(adapter.clone(), "proj", Arc::clone(&registry)).await.unwrap().unwrap();

        assert!(matches!(next(&mut sub.event_rx).await, ListenEvent::Changed(d) if d.path.document_id == "a"));
        // Missed-events resync signal after the reconnect succeeds.
        tokio::time::timeout(Duration::from_secs(120), sub.reset_notify.notified()).await.unwrap();
        assert!(matches!(next(&mut sub.event_rx).await, ListenEvent::Changed(d) if d.path.document_id == "b"));
        assert_eq!(*adapter.open_count.lock().unwrap(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn feed_reset_event_resets_subscribers() {
        let adapter = fake(vec![Ok(vec![Ok(ChangeEvent::Reset)])], &[]);
        let registry = ListenRegistry::new();
        let sub = registry.register(&notify_channel("proj")).await;
        let _l = ChangeFeedListener::start(adapter, "proj", Arc::clone(&registry)).await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(120), sub.reset_notify.notified()).await.unwrap();
    }

    #[tokio::test]
    async fn start_fails_when_feed_cannot_be_opened() {
        let adapter = fake(vec![Err(down())], &[]);
        let r = ChangeFeedListener::start(adapter, "proj", ListenRegistry::new()).await;
        assert!(r.is_err());
    }
}

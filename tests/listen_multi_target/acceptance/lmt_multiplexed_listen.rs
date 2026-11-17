//! listen-multi-target (go-live finding P2) — one Listen stream carries many
//! targets: Query targets added/removed at any point, Documents targets
//! (`onSnapshot(doc)`), server-assigned target ids, and Query targets that
//! honour orderBy / limit / offset.
//!
//! Driving port: gRPC data port (:8080) `Listen`, real Postgres NOTIFY
//! fan-out, real gRPC. No test waits on a short wall-clock window: every
//! expectation is "the next message on the stream" with a generous timeout —
//! events on one stream are strictly ordered, so "X is never delivered" is
//! proven by the NEXT event being something else.

use std::{collections::HashMap, sync::Arc, time::Duration};

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    structured_query::{CollectionSelector, Direction, FieldReference, Order},
    target::{self, query_target},
    target_change::TargetChangeType,
    value::ValueType,
    CreateDocumentRequest, DeleteDocumentRequest, Document, ListenRequest, ListenResponse,
    StructuredQuery, Target, UpdateDocumentRequest, Value,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server_with_keepalive};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use tokio_stream::StreamExt;

// ── environment ─────────────────────────────────────────────────────────────

struct TestEnv {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    project_id: String,
    api_key: String,
    server: embyr_server::TestServer,
}

async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("Failed to start Postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("host port");
    (container, format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
}

async fn setup(api_key: &str, project_id: &str) -> TestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer").run(&cust_pool).await.unwrap();

    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    let sys_pool = sqlx::PgPool::connect(&sys_url).await.unwrap();
    sqlx::query(
        "INSERT INTO projects \
         (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ($1, 'active', 'direct_pg', $2, $3)",
    )
    .bind(project_id)
    .bind(&api_key_hash)
    .bind(&encrypted_dsn)
    .execute(&sys_pool)
    .await
    .unwrap();

    // Long keepalive: nothing here may depend on it.
    let server = start_test_server_with_keepalive(system_db, Duration::from_secs(30)).await;
    TestEnv {
        _sys_container,
        _cust_container,
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server,
    }
}

impl TestEnv {
    fn client(&self) -> FirestoreClient<tonic::transport::Channel> {
        FirestoreClient::new(
            tonic::transport::Channel::from_shared(format!("http://{}", self.server.grpc_addr))
                .unwrap()
                .connect_lazy(),
        )
    }

    fn authed<T>(&self, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request
            .metadata_mut()
            .insert("authorization", format!("bearer {}", self.api_key).parse().unwrap());
        request
    }

    fn docs_root(&self) -> String {
        format!("projects/{}/databases/(default)/documents", self.project_id)
    }

    fn doc_name(&self, collection: &str, id: &str) -> String {
        format!("{}/{collection}/{id}", self.docs_root())
    }

    async fn create(&self, collection: &str, id: &str, fields: HashMap<String, Value>) {
        self.client()
            .create_document(self.authed(CreateDocumentRequest {
                parent: self.docs_root(),
                collection_id: collection.to_string(),
                document_id: id.to_string(),
                document: Some(Document { fields, ..Default::default() }),
                ..Default::default()
            }))
            .await
            .expect("create_document");
    }

    async fn update(&self, collection: &str, id: &str, fields: HashMap<String, Value>) {
        self.client()
            .update_document(self.authed(UpdateDocumentRequest {
                document: Some(Document {
                    name: self.doc_name(collection, id),
                    fields,
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await
            .expect("update_document");
    }

    async fn delete(&self, collection: &str, id: &str) {
        self.client()
            .delete_document(self.authed(DeleteDocumentRequest {
                name: self.doc_name(collection, id),
                ..Default::default()
            }))
            .await
            .expect("delete_document");
    }

    /// Open a Listen stream whose request side stays open, so later
    /// AddTarget / RemoveTarget messages can be sent through the returned sender.
    async fn listen(
        &self,
        first: ListenRequest,
    ) -> (tokio::sync::mpsc::Sender<ListenRequest>, tonic::Streaming<ListenResponse>) {
        let (request_tx, request_rx) = tokio::sync::mpsc::channel(8);
        request_tx.send(first).await.unwrap();
        let stream = self
            .client()
            .listen(self.authed(tokio_stream::wrappers::ReceiverStream::new(request_rx)))
            .await
            .expect("listen should succeed")
            .into_inner();
        (request_tx, stream)
    }

    fn add(&self, target: Target) -> ListenRequest {
        ListenRequest {
            database: format!("projects/{}/databases/(default)", self.project_id),
            target_change: Some(listen_request::TargetChange::AddTarget(target)),
            ..Default::default()
        }
    }

    fn remove(&self, target_id: i32) -> ListenRequest {
        ListenRequest {
            database: format!("projects/{}/databases/(default)", self.project_id),
            target_change: Some(listen_request::TargetChange::RemoveTarget(target_id)),
            ..Default::default()
        }
    }

    fn collection_target(&self, target_id: i32, collection: &str) -> Target {
        self.query_target(target_id, collection, vec![], None, 0)
    }

    fn query_target(
        &self,
        target_id: i32,
        collection: &str,
        order_by: Vec<(&str, Direction)>,
        limit: Option<i32>,
        offset: i32,
    ) -> Target {
        Target {
            target_id,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: self.docs_root(),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection.to_string(),
                        all_descendants: false,
                    }],
                    order_by: order_by
                        .into_iter()
                        .map(|(field, direction)| Order {
                            field: Some(FieldReference { field_path: field.to_string() }),
                            direction: direction as i32,
                        })
                        .collect(),
                    limit,
                    offset,
                    ..Default::default()
                })),
            })),
            ..Default::default()
        }
    }

    fn documents_target(&self, target_id: i32, names: &[String]) -> Target {
        Target {
            target_id,
            target_type: Some(target::TargetType::Documents(target::DocumentsTarget {
                documents: names.to_vec(),
            })),
            ..Default::default()
        }
    }
}

fn int(n: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(n)) }
}

fn score(n: i64) -> HashMap<String, Value> {
    HashMap::from([("score".to_string(), int(n))])
}

// ── observed stream events ──────────────────────────────────────────────────

/// What a client can observe, with document names shortened to
/// `collection/id`. NO_CHANGE keep-alives are skipped.
#[derive(Debug, PartialEq, Eq)]
enum Ev {
    Added(Vec<i32>),
    Current(Vec<i32>),
    /// TargetChange(REMOVE) with the cause's status code, if any.
    TargetRemoved { ids: Vec<i32>, cause_code: Option<i32> },
    Changed { doc: String, targets: Vec<i32> },
    Deleted { doc: String, removed_from: Vec<i32> },
    /// DocumentRemove: still exists, no longer in the target's result.
    Left { doc: String, removed_from: Vec<i32> },
}

fn short(name: &str) -> String {
    name.split("/documents/").nth(1).unwrap_or(name).to_string()
}

async fn next_event(stream: &mut tonic::Streaming<ListenResponse>) -> Ev {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .expect("timed out waiting for the next Listen message")
            .expect("stream ended while waiting for the next Listen message")
            .expect("stream error while waiting for the next Listen message");
        match message.response_type.expect("response_type set") {
            listen_response::ResponseType::TargetChange(tc) => {
                match TargetChangeType::try_from(tc.target_change_type).unwrap() {
                    TargetChangeType::NoChange => continue,
                    TargetChangeType::Add => return Ev::Added(tc.target_ids),
                    TargetChangeType::Current => return Ev::Current(tc.target_ids),
                    TargetChangeType::Remove => {
                        return Ev::TargetRemoved {
                            ids: tc.target_ids,
                            cause_code: tc.cause.map(|c| c.code),
                        }
                    }
                    TargetChangeType::Reset => panic!("unexpected RESET"),
                }
            }
            listen_response::ResponseType::DocumentChange(dc) => {
                return Ev::Changed {
                    doc: short(&dc.document.expect("document").name),
                    targets: dc.target_ids,
                }
            }
            listen_response::ResponseType::DocumentDelete(dd) => {
                return Ev::Deleted { doc: short(&dd.document), removed_from: dd.removed_target_ids }
            }
            listen_response::ResponseType::DocumentRemove(dr) => {
                return Ev::Left { doc: short(&dr.document), removed_from: dr.removed_target_ids }
            }
            listen_response::ResponseType::Filter(_) => continue,
        }
    }
}

fn changed(doc: &str, target: i32) -> Ev {
    Ev::Changed { doc: doc.to_string(), targets: vec![target] }
}

// ── scenarios ───────────────────────────────────────────────────────────────

/// Two listeners multiplexed on ONE stream (what the web SDK does): each
/// target gets its own ADD / snapshot / CURRENT, changes carry only the
/// matching target id, and RemoveTarget stops delivery for exactly that target.
///
/// @driving_port @real-io
#[tokio::test]
async fn second_target_added_later_gets_its_own_snapshot_and_changes_and_remove_stops_it() {
    let env = setup("test-sk-lmt-multi-01", "lmt-multi-project-01").await;
    env.create("alpha", "a1", HashMap::new()).await;
    env.create("beta", "b1", HashMap::new()).await;

    let (request_tx, mut stream) = env.listen(env.add(env.collection_target(1, "alpha"))).await;
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![1]));
    assert_eq!(next_event(&mut stream).await, changed("alpha/a1", 1));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![1]));

    // A second AddTarget, well after the stream started.
    request_tx.send(env.add(env.collection_target(2, "beta"))).await.unwrap();
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![2]));
    assert_eq!(next_event(&mut stream).await, changed("beta/b1", 2));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![2]));

    env.create("alpha", "a2", HashMap::new()).await;
    assert_eq!(next_event(&mut stream).await, changed("alpha/a2", 1));
    env.create("beta", "b2", HashMap::new()).await;
    assert_eq!(next_event(&mut stream).await, changed("beta/b2", 2));

    request_tx.send(env.remove(1)).await.unwrap();
    assert_eq!(
        next_event(&mut stream).await,
        Ev::TargetRemoved { ids: vec![1], cause_code: None }
    );

    // a3 must never be delivered: the next event is b3's, for target 2 only.
    env.create("alpha", "a3", HashMap::new()).await;
    env.create("beta", "b3", HashMap::new()).await;
    assert_eq!(next_event(&mut stream).await, changed("beta/b3", 2));
}

/// `target_id` 0 means the server assigns one (never reusing a removed id),
/// and a target that fails to add AFTER the first message is reported as
/// TargetChange(REMOVE, cause) without killing the stream.
///
/// @driving_port @real-io
#[tokio::test]
async fn server_assigns_ids_for_zero_and_a_failed_add_removes_only_that_target() {
    let env = setup("test-sk-lmt-multi-02", "lmt-multi-project-02").await;

    let (request_tx, mut stream) = env.listen(env.add(env.collection_target(0, "alpha"))).await;
    let Ev::Added(first) = next_event(&mut stream).await else { panic!("expected ADD") };
    assert_eq!(first.len(), 1);
    assert!(first[0] > 0, "server-assigned id must be positive, got {first:?}");
    assert_eq!(next_event(&mut stream).await, Ev::Current(first.clone()));

    request_tx.send(env.add(env.collection_target(0, "beta"))).await.unwrap();
    let Ev::Added(second) = next_event(&mut stream).await else { panic!("expected ADD") };
    assert_eq!(next_event(&mut stream).await, Ev::Current(second.clone()));
    assert_ne!(first, second, "two targets must not share an id");

    request_tx.send(env.remove(first[0])).await.unwrap();
    assert_eq!(
        next_event(&mut stream).await,
        Ev::TargetRemoved { ids: first.clone(), cause_code: None }
    );
    request_tx.send(env.add(env.collection_target(0, "gamma"))).await.unwrap();
    let Ev::Added(third) = next_event(&mut stream).await else { panic!("expected ADD") };
    assert_eq!(next_event(&mut stream).await, Ev::Current(third.clone()));
    assert!(third != first && third != second, "removed ids are not reused: {first:?} {second:?} {third:?}");

    // A Documents target naming no documents fails on its own...
    request_tx.send(env.add(env.documents_target(50, &[]))).await.unwrap();
    assert_eq!(
        next_event(&mut stream).await,
        Ev::TargetRemoved { ids: vec![50], cause_code: Some(tonic::Code::InvalidArgument as i32) }
    );
    // ...and the stream (and the other targets) live on.
    env.create("beta", "b1", HashMap::new()).await;
    assert_eq!(next_event(&mut stream).await, Ev::Changed { doc: "beta/b1".into(), targets: second });
}

/// `onSnapshot(doc(...))`: a Documents target as the FIRST message. Existing
/// documents arrive as the snapshot (a missing one is simply absent), then
/// only changes to exactly the named documents.
///
/// @driving_port @real-io
#[tokio::test]
async fn documents_target_delivers_named_documents_and_only_their_changes() {
    let env = setup("test-sk-lmt-multi-03", "lmt-multi-project-03").await;
    env.create("users", "u1", HashMap::new()).await;

    let names = [env.doc_name("users", "u1"), env.doc_name("users", "ghost")];
    let (_request_tx, mut stream) = env.listen(env.add(env.documents_target(7, &names))).await;
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![7]));
    assert_eq!(next_event(&mut stream).await, changed("users/u1", 7));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![7]));

    // u2 is in the same collection but not named: the next event is u1's.
    env.create("users", "u2", HashMap::new()).await;
    env.update("users", "u1", score(1)).await;
    assert_eq!(next_event(&mut stream).await, changed("users/u1", 7));

    env.delete("users", "u1").await;
    assert_eq!(
        next_event(&mut stream).await,
        Ev::Deleted { doc: "users/u1".into(), removed_from: vec![7] }
    );

    // A named document that did not exist at subscribe time shows up once created.
    env.create("users", "ghost", HashMap::new()).await;
    assert_eq!(next_event(&mut stream).await, changed("users/ghost", 7));
}

/// A limit query is ordered in its snapshot, and its window stays correct as
/// documents enter and leave it.
///
/// @driving_port @real-io
#[tokio::test]
async fn limit_query_snapshot_is_ordered_and_window_follows_changes() {
    let env = setup("test-sk-lmt-multi-04", "lmt-multi-project-04").await;
    for (id, s) in [("s10", 10), ("s20", 20), ("s30", 30)] {
        env.create("scores", id, score(s)).await;
    }

    let target = env.query_target(1, "scores", vec![("score", Direction::Descending)], Some(2), 0);
    let (_request_tx, mut stream) = env.listen(env.add(target)).await;
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![1]));
    assert_eq!(next_event(&mut stream).await, changed("scores/s30", 1));
    assert_eq!(next_event(&mut stream).await, changed("scores/s20", 1));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![1]));

    // 25 enters the top-2 and pushes 20 out.
    env.create("scores", "s25", score(25)).await;
    assert_eq!(next_event(&mut stream).await, changed("scores/s25", 1));
    assert_eq!(
        next_event(&mut stream).await,
        Ev::Left { doc: "scores/s20".into(), removed_from: vec![1] }
    );

    // A document that would not make the window changes nothing visible; the
    // events seen next are the ones caused by the following delete. NOTIFY
    // delivery is asynchronous, so the s15 notification may be handled before
    // or after the delete committed: s30 leaves as DocumentDelete (delete seen
    // first) or DocumentRemove (window recomputed after it), and in either
    // order s20 is pulled back in. Compare as a set of normalized events.
    // (Two-digit scores on purpose: storage orders field values as text.)
    env.create("scores", "s15", score(15)).await;
    env.delete("scores", "s30").await;
    let mut next_two = vec![next_event(&mut stream).await, next_event(&mut stream).await]
        .into_iter()
        .map(|ev| match ev {
            Ev::Deleted { doc, removed_from } | Ev::Left { doc, removed_from } => {
                format!("gone {doc} {removed_from:?}")
            }
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>();
    next_two.sort();
    let mut expected = vec![format!("{:?}", changed("scores/s20", 1)), "gone scores/s30 [1]".to_string()];
    expected.sort();
    assert_eq!(next_two, expected);
}

/// An offset window shifts when a document ahead of it is deleted, even though
/// that document was never in the window.
///
/// @driving_port @real-io
#[tokio::test]
async fn offset_window_shifts_when_an_earlier_document_is_deleted() {
    let env = setup("test-sk-lmt-multi-05", "lmt-multi-project-05").await;
    for (id, s) in [("s10", 10), ("s20", 20), ("s30", 30)] {
        env.create("scores", id, score(s)).await;
    }

    let target = env.query_target(1, "scores", vec![("score", Direction::Ascending)], Some(1), 1);
    let (_request_tx, mut stream) = env.listen(env.add(target)).await;
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![1]));
    assert_eq!(next_event(&mut stream).await, changed("scores/s20", 1));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![1]));

    env.delete("scores", "s10").await;
    assert_eq!(next_event(&mut stream).await, changed("scores/s30", 1));
    assert_eq!(
        next_event(&mut stream).await,
        Ev::Left { doc: "scores/s20".into(), removed_from: vec![1] }
    );
}

// ── cursors and resume (listen-resume-and-agent-deletes) ────────────────────

/// Give a Query target `startAt(start)` / `endBefore(end)` cursors (single
/// field, matching the single-field orderBy the storage layer supports).
fn with_cursors(mut target: Target, start: i64, end: i64) -> Target {
    let Some(target::TargetType::Query(qt)) = &mut target.target_type else { panic!("query target") };
    let Some(query_target::QueryType::StructuredQuery(sq)) = &mut qt.query_type else {
        panic!("structured query")
    };
    sq.start_at = Some(embyr_proto::firestore::Cursor { values: vec![int(start)], before: true });
    sq.end_at = Some(embyr_proto::firestore::Cursor { values: vec![int(end)], before: true });
    target
}

/// The messages a client sees until CURRENT, as short tags: `ADD`, `RESET`,
/// `doc:<collection/id>`, `FILTER:<count>`. NO_CHANGE keep-alives are skipped.
async fn until_current(stream: &mut tonic::Streaming<ListenResponse>) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        match message.response_type.expect("response_type set") {
            listen_response::ResponseType::TargetChange(tc) => {
                match TargetChangeType::try_from(tc.target_change_type).unwrap() {
                    TargetChangeType::Add => seen.push("ADD".to_string()),
                    TargetChangeType::Reset => seen.push("RESET".to_string()),
                    TargetChangeType::Current => return seen,
                    _ => {}
                }
            }
            listen_response::ResponseType::DocumentChange(dc) => {
                seen.push(format!("doc:{}", short(&dc.document.expect("document").name)));
            }
            listen_response::ResponseType::Filter(f) => seen.push(format!("FILTER:{}", f.count)),
            other => panic!("unexpected message before CURRENT: {other:?}"),
        }
    }
}

fn with_resume_token(mut target: Target, ts_seconds: i64) -> Target {
    // Token layout (see resume_token.rs): 8-byte big-endian unix seconds.
    target.resume_type = Some(target::ResumeType::ResumeToken(ts_seconds.to_be_bytes().to_vec()));
    target
}

/// startAt / endBefore cursors on a Listen query bound the initial snapshot
/// AND every later change: a document outside the bounds is never delivered.
///
/// @driving_port @real-io
#[tokio::test]
async fn cursor_bounds_apply_to_the_snapshot_and_to_later_changes() {
    let env = setup("test-sk-lmt-multi-06", "lmt-multi-project-06").await;
    for (id, s) in [("s10", 10), ("s20", 20), ("s30", 30), ("s40", 40)] {
        env.create("scores", id, score(s)).await;
    }

    let target =
        with_cursors(env.query_target(1, "scores", vec![("score", Direction::Ascending)], None, 0), 20, 40);
    let (_request_tx, mut stream) = env.listen(env.add(target)).await;
    assert_eq!(next_event(&mut stream).await, Ev::Added(vec![1]));
    assert_eq!(next_event(&mut stream).await, changed("scores/s20", 1));
    assert_eq!(next_event(&mut stream).await, changed("scores/s30", 1));
    assert_eq!(next_event(&mut stream).await, Ev::Current(vec![1]));

    // s05 and s45 are outside [20, 40): the next event is s35's.
    env.create("scores", "s05", score(5)).await;
    env.create("scores", "s45", score(45)).await;
    env.create("scores", "s35", score(35)).await;
    assert_eq!(next_event(&mut stream).await, changed("scores/s35", 1));
}

/// A resume on a windowed target cannot be answered with a delta: the server
/// says RESET (client drops its cached documents for the target) and then
/// sends the full result, instead of silently resending over a stale cache.
///
/// @driving_port @real-io
#[tokio::test]
async fn resume_token_on_a_windowed_target_resets_then_sends_the_full_window() {
    let env = setup("test-sk-lmt-multi-07", "lmt-multi-project-07").await;
    for (id, s) in [("s10", 10), ("s20", 20), ("s30", 30)] {
        env.create("scores", id, score(s)).await;
    }

    let one_minute_ago = chrono::Utc::now().timestamp() - 60;
    let target = with_resume_token(
        env.query_target(1, "scores", vec![("score", Direction::Ascending)], Some(2), 0),
        one_minute_ago,
    );
    let (_request_tx, mut stream) = env.listen(env.add(target)).await;
    assert_eq!(
        until_current(&mut stream).await,
        ["ADD", "RESET", "doc:scores/s10", "doc:scores/s20"]
    );
}

/// A resume on a whole-collection target sends only what changed since the
/// token, then an ExistenceFilter with the TRUE match count so the client can
/// detect a deletion that happened while it was away. (A token newer than
/// every document stands for "nothing changed since": no documents, count 2.)
///
/// @driving_port @real-io
#[tokio::test]
async fn resume_token_on_a_collection_target_sends_delta_then_existence_filter() {
    let env = setup("test-sk-lmt-multi-08", "lmt-multi-project-08").await;
    env.create("things", "t1", HashMap::new()).await;
    env.create("things", "t2", HashMap::new()).await;

    let in_an_hour = chrono::Utc::now().timestamp() + 3600;
    let target = with_resume_token(env.collection_target(1, "things"), in_an_hour);
    let (_request_tx, mut stream) = env.listen(env.add(target)).await;
    assert_eq!(until_current(&mut stream).await, ["ADD", "FILTER:2"]);
}

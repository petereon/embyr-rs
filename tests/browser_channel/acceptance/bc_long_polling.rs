//! browser-channel (go-live finding P3) — the Firestore web SDK's long-polling
//! fallback (BrowserChannel / WebChannel) over real HTTP on :8081:
//! `onSnapshot` on a collection (Listen channel) and `setDoc` (Write channel).
//!
//! Driving port: REST port (:8081) `/google.firestore.v1.Firestore/{Listen,Write}/channel`,
//! speaking the wire protocol firebase-js-sdk's `webchannel_connection.ts` speaks
//! (handshake POST with `$httpHeaders`, form-encoded `req0___data__` messages,
//! `<len>\n<json>` chunks on a streaming GET back channel). State is changed and
//! verified through the native gRPC port. Every expectation is "the next
//! message on the channel" with a generous timeout.

use std::{collections::HashMap, sync::Arc, time::Duration};

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, value::ValueType, CreateDocumentRequest, Document,
    GetDocumentRequest, Value,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server_with_keepalive};
use serde_json::{json, Value as J};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

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

async fn setup() -> TestEnv {
    let (project_id, api_key) = ("bc-test-project", "test-sk-browser-channel-abc");
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
    fn docs_root(&self) -> String {
        format!("projects/{}/databases/(default)/documents", self.project_id)
    }

    fn database(&self) -> String {
        format!("projects/{}/databases/(default)", self.project_id)
    }

    fn grpc(&self) -> FirestoreClient<tonic::transport::Channel> {
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

    async fn create(&self, collection: &str, id: &str, score: i64) {
        let fields = HashMap::from([(
            "score".to_string(),
            Value { value_type: Some(ValueType::IntegerValue(score)) },
        )]);
        self.grpc()
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

    fn channel_url(&self, rpc: &str) -> String {
        format!("http://{}/google.firestore.v1.Firestore/{rpc}/channel", self.server.rest_addr)
    }
}

// ── a minimal WebChannel client ─────────────────────────────────────────────

fn urlenc(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Pop one `<len>\n<payload>` chunk off the front of `buf` (len = UTF-16
/// units of payload, as closure's `getNextChunk_` reads it).
fn take_chunk(buf: &mut String) -> Option<String> {
    let nl = buf.find('\n')?;
    let len: usize = buf[..nl].trim().parse().expect("chunk length");
    let rest = &buf[nl + 1..];
    let units: Vec<u16> = rest.encode_utf16().collect();
    if units.len() < len {
        return None;
    }
    let payload = String::from_utf16(&units[..len]).unwrap();
    let consumed = nl + 1 + payload.len();
    buf.drain(..consumed);
    Some(payload.trim().to_string())
}

struct Channel {
    http: reqwest::Client,
    url: String,
    sid: String,
    gsessionid: String,
    next_ofs: u64,
    rid: u64,
    last_aid: u64,
    back: Option<reqwest::Response>,
    buf: String,
    inbox: std::collections::VecDeque<J>,
}

impl Channel {
    /// Handshake the way the SDK does: credentials in `$httpHeaders`, zero
    /// messages in the first POST (it waits for the `c` array before sending).
    async fn open(env: &TestEnv, rpc: &str, bearer: &str) -> Channel {
        let http = reqwest::Client::builder().http1_only().build().unwrap();
        let headers = urlenc(&format!("Authorization: Bearer {bearer}\r\nX-Goog-Api-Client: test\r\n"));
        let url = env.channel_url(rpc);
        let resp = http
            .post(format!(
                "{url}?VER=8&database={}&RID=1&CVER=22&X-HTTP-Session-Id=gsessionid&%24httpHeaders={headers}",
                urlenc(&env.database())
            ))
            .header("content-type", "application/x-www-form-urlencoded")
            .body("count=0&ofs=0")
            .send()
            .await
            .expect("handshake POST");
        assert_eq!(resp.status(), 200, "handshake must succeed");
        let gsessionid = resp
            .headers()
            .get("x-http-session-id")
            .expect("X-HTTP-Session-Id header")
            .to_str()
            .unwrap()
            .to_string();
        let mut body = resp.text().await.unwrap();
        let c: J = serde_json::from_str(&take_chunk(&mut body).expect("c chunk")).unwrap();
        assert_eq!(c[0][0], 0, "handshake array id is 0");
        assert_eq!(c[0][1][0], "c");
        Channel {
            http,
            url,
            sid: c[0][1][1].as_str().unwrap().to_string(),
            gsessionid,
            next_ofs: 0,
            rid: 1,
            last_aid: 0,
            back: None,
            buf: String::new(),
            inbox: Default::default(),
        }
    }

    fn query(&self) -> String {
        format!("VER=8&gsessionid={}&SID={}", self.gsessionid, self.sid)
    }

    async fn send(&mut self, msg: J) {
        self.rid += 1;
        let body = format!(
            "count=1&ofs={}&req0___data__={}",
            self.next_ofs,
            urlenc(&msg.to_string())
        );
        self.next_ofs += 1;
        let resp = self
            .http
            .post(format!("{}?{}&RID={}&AID={}", self.url, self.query(), self.rid, self.last_aid))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .expect("forward POST");
        assert_eq!(resp.status(), 200);
        let mut text = resp.text().await.unwrap();
        let ack: J = serde_json::from_str(&take_chunk(&mut text).expect("forward ack")).unwrap();
        assert_eq!(ack.as_array().map(Vec::len), Some(3), "forward response is [bc, lastId, bytes]");
    }

    /// Next application message from the streaming back channel (opening it
    /// if needed); `noop` and NO_CHANGE keep-alives are skipped.
    async fn next(&mut self) -> J {
        loop {
            if let Some(m) = self.inbox.pop_front() {
                if m["targetChange"]["targetChangeType"] == "NO_CHANGE" {
                    continue;
                }
                return m;
            }
            if let Some(chunk) = take_chunk(&mut self.buf) {
                let arrays: Vec<J> = serde_json::from_str(&chunk).unwrap();
                for a in arrays {
                    self.last_aid = a[0].as_u64().unwrap();
                    for m in a[1].as_array().unwrap() {
                        if m != "noop" {
                            self.inbox.push_back(m.clone());
                        }
                    }
                }
                continue;
            }
            if self.back.is_none() {
                let resp = self
                    .http
                    .get(format!(
                        "{}?{}&RID=rpc&AID={}&CI=0&TYPE=xmlhttp",
                        self.url,
                        self.query(),
                        self.last_aid
                    ))
                    .send()
                    .await
                    .expect("back channel GET");
                assert_eq!(resp.status(), 200);
                self.back = Some(resp);
            }
            let more = tokio::time::timeout(Duration::from_secs(20), self.back.as_mut().unwrap().chunk())
                .await
                .expect("timed out waiting for a back-channel message")
                .expect("back channel read");
            match more {
                Some(bytes) => self.buf.push_str(std::str::from_utf8(&bytes).unwrap()),
                None => self.back = None, // server ended this response: re-poll with the ack
            }
        }
    }

    /// One long-poll round trip (`CI=1`) acking up to `aid`; the raw arrays.
    async fn long_poll(&self, aid: u64) -> Vec<J> {
        let mut body = self
            .http
            .get(format!("{}?{}&RID=rpc&AID={aid}&CI=1&TYPE=xmlhttp", self.url, self.query()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let mut out = vec![];
        while let Some(c) = take_chunk(&mut body) {
            out.extend(serde_json::from_str::<Vec<J>>(&c).unwrap());
        }
        out
    }

    async fn terminate(&mut self) {
        self.rid += 1;
        self.http
            .post(format!("{}?{}&RID={}&TYPE=terminate", self.url, self.query(), self.rid))
            .body("count=1&ofs=0&req0_type=terminate")
            .send()
            .await
            .unwrap();
    }
}

fn add_collection_target(env: &TestEnv, target_id: i32, collection: &str) -> J {
    json!({
        "database": env.database(),
        "addTarget": {
            "query": {
                "parent": env.docs_root(),
                "structuredQuery": {"from": [{"collectionId": collection}]},
            },
            "targetId": target_id,
        }
    })
}

fn short(name: &J) -> String {
    let n = name.as_str().unwrap();
    n.split("/documents/").nth(1).unwrap_or(n).to_string()
}

// ── onSnapshot over long polling ────────────────────────────────────────────

/// Given an existing document in a collection
/// When  a Listen channel adds a collection query target
/// Then  the back channel delivers ADD, the existing document, then CURRENT
/// And   a document created afterwards arrives as a live documentChange.
#[tokio::test]
async fn bc01_on_snapshot_delivers_initial_snapshot_then_live_changes() {
    let env = setup().await;
    env.create("bc_items", "a", 1).await;

    let mut ch = Channel::open(&env, "Listen", &env.api_key).await;
    ch.send(add_collection_target(&env, 2, "bc_items")).await;

    let m = ch.next().await;
    assert_eq!(m["targetChange"]["targetChangeType"], "ADD");
    assert_eq!(m["targetChange"]["targetIds"], json!([2]));

    let m = ch.next().await;
    assert_eq!(short(&m["documentChange"]["document"]["name"]), "bc_items/a");
    assert_eq!(m["documentChange"]["document"]["fields"]["score"]["integerValue"], "1");
    assert_eq!(m["documentChange"]["targetIds"], json!([2]));

    let m = ch.next().await;
    assert_eq!(m["targetChange"]["targetChangeType"], "CURRENT");

    env.create("bc_items", "b", 2).await;
    let m = ch.next().await;
    assert_eq!(short(&m["documentChange"]["document"]["name"]), "bc_items/b");
    assert_eq!(m["documentChange"]["document"]["fields"]["score"]["integerValue"], "2");
}

/// Un-acked arrays are replayed to a fresh back channel; acked ones are not.
#[tokio::test]
async fn bc02_back_channel_replays_unacked_arrays_after_reconnect() {
    let env = setup().await;
    env.create("bc_items", "a", 1).await;
    let mut ch = Channel::open(&env, "Listen", &env.api_key).await;
    ch.send(add_collection_target(&env, 2, "bc_items")).await;
    // Let ADD, the document and CURRENT (arrays 1..=3) all be queued.
    for _ in 0..3 {
        ch.next().await;
    }
    ch.back = None; // simulate a dropped back-channel connection

    let replay = ch.long_poll(0).await;
    assert_eq!(replay[0][0], 1, "AID=0 replays from the first array");
    assert_eq!(replay[0][1][0]["targetChange"]["targetChangeType"], "ADD");

    // Acked 1..=3: only newer arrays come back — here, the next live change.
    env.create("bc_items", "b", 2).await;
    let after_ack = ch.long_poll(3).await;
    assert_eq!(after_ack[0][0], 4);
    assert_eq!(
        short(&after_ack[0][1][0]["documentChange"]["document"]["name"]),
        "bc_items/b"
    );
}

// ── setDoc over long polling ────────────────────────────────────────────────

/// When the Write channel handshakes and then sends an update with the issued
/// streamToken
/// Then  the response carries a writeResult and the document is readable via gRPC.
#[tokio::test]
async fn bc03_set_doc_over_write_channel_persists_the_document() {
    let env = setup().await;
    let mut ch = Channel::open(&env, "Write", &env.api_key).await;

    // Stream handshake: the empty WriteRequest {database}.
    ch.send(json!({"database": env.database()})).await;
    let hs = ch.next().await;
    let token = hs["streamToken"].as_str().expect("handshake streamToken").to_string();
    assert!(!token.is_empty());
    assert!(hs["writeResults"].as_array().is_none_or(Vec::is_empty));

    let name = format!("{}/bc_written/doc1", env.docs_root());
    ch.send(json!({
        "streamToken": token,
        "writes": [{"update": {"name": name, "fields": {"title": {"stringValue": "hello"}}}}],
    }))
    .await;
    let resp = ch.next().await;
    assert_eq!(resp["writeResults"].as_array().unwrap().len(), 1);
    assert!(resp["writeResults"][0]["updateTime"].is_string());
    assert_ne!(resp["streamToken"].as_str().unwrap(), token, "token rotates per response");

    let doc = env
        .grpc()
        .get_document(env.authed(GetDocumentRequest { name, ..Default::default() }))
        .await
        .expect("document written over BrowserChannel is readable over gRPC")
        .into_inner();
    assert!(matches!(
        doc.fields["title"].value_type,
        Some(ValueType::StringValue(ref s)) if s == "hello"
    ));
}

/// A write made over the Write channel shows up on a Listen channel.
#[tokio::test]
async fn bc04_write_channel_change_reaches_listen_channel() {
    let env = setup().await;
    let mut listen = Channel::open(&env, "Listen", &env.api_key).await;
    listen.send(add_collection_target(&env, 2, "bc_cross")).await;
    assert_eq!(listen.next().await["targetChange"]["targetChangeType"], "ADD");
    assert_eq!(listen.next().await["targetChange"]["targetChangeType"], "CURRENT");

    let mut write = Channel::open(&env, "Write", &env.api_key).await;
    write.send(json!({"database": env.database()})).await;
    let token = write.next().await["streamToken"].as_str().unwrap().to_string();
    let name = format!("{}/bc_cross/x", env.docs_root());
    write
        .send(json!({"streamToken": token, "writes": [{"update": {"name": name,
            "fields": {"score": {"integerValue": "7"}}}}]}))
        .await;
    write.next().await;

    let m = listen.next().await;
    assert_eq!(short(&m["documentChange"]["document"]["name"]), "bc_cross/x");
    assert_eq!(m["documentChange"]["document"]["fields"]["score"]["integerValue"], "7");
}

// ── errors and lifecycle ────────────────────────────────────────────────────

/// A handshake with no credentials is refused outright.
#[tokio::test]
async fn bc05_handshake_without_authorization_is_401() {
    let env = setup().await;
    let resp = reqwest::Client::new()
        .post(format!("{}?VER=8&RID=1", env.channel_url("Listen")))
        .body("count=0&ofs=0")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

/// Same authentication as gRPC: a wrong API key surfaces as a WebChannel
/// stream error message (`{"error": {"status": ...}}`), which the SDK maps to
/// a FirestoreError.
#[tokio::test]
async fn bc06_wrong_api_key_is_reported_as_stream_error() {
    let env = setup().await;
    let mut ch = Channel::open(&env, "Listen", "not-the-key").await;
    ch.send(add_collection_target(&env, 2, "bc_items")).await;
    let m = ch.next().await;
    assert_eq!(m["error"]["status"], "UNAUTHENTICATED", "got {m}");
}

/// terminate ends the session; later requests get 400 "Unknown SID".
#[tokio::test]
async fn bc07_terminate_ends_the_session() {
    let env = setup().await;
    let mut ch = Channel::open(&env, "Listen", &env.api_key).await;
    ch.terminate().await;
    let resp = reqwest::Client::new()
        .get(format!(
            "{}?{}&RID=rpc&AID=0&CI=1&TYPE=xmlhttp",
            ch.url,
            ch.query()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(resp.text().await.unwrap(), "Unknown SID");
}

/// A GET without SID (and not a test-channel probe) is a 400.
#[tokio::test]
async fn bc08_back_channel_request_without_sid_is_400() {
    let env = setup().await;
    let resp = reqwest::Client::new().get(env.channel_url("Listen")).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

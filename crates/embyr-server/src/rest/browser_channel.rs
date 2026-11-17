//! BrowserChannel (Closure WebChannel) server for the Firestore web SDK's
//! long-polling transport, on :8081.
//!
//! Routes: `/google.firestore.v1.Firestore/{Listen|Write}/channel`.
//! Design + what is NOT supported: docs/feature/browser-channel/feature-delta.md.
//!
//! This module is protocol only (sessions, framing, acks, expiry). It speaks
//! proto3-JSON in both directions to a [`ChannelBackend`]; the real backend
//! ([`FirestoreService`]) calls the SAME `handle_listen` / `handle_write` the
//! native gRPC path uses, so auth, rate limiting and security rules are not
//! re-implemented here.

use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    future::Future,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::Value as J;
use tokio::sync::{mpsc, watch};
use tokio_stream::{wrappers::ReceiverStream, StreamExt as _};
use tonic::{metadata::MetadataValue, Request, Status};

use crate::{
    encoding::proto_json::{
        listen_request_from_json, listen_response_json, status_error_json,
        write_request_from_json, write_response_json,
    },
    grpc::handler::FirestoreService,
};

/// Only these request headers are forwarded to the gRPC handlers (same ones
/// the gRPC-Web path reads).
const FORWARDED_HEADERS: [&str; 2] = ["authorization", "x-embyr-client-identity"];
/// Sessions that exist but never get a back channel / forward traffic.
const MAX_SESSIONS: usize = 10_000;
/// Un-acked arrays kept per session before the session is dropped.
const MAX_BUFFERED_ARRAYS: usize = 4096;

// ── Backend port ────────────────────────────────────────────────────────────

/// Parse + enqueue one client message; `Err` ends the session.
pub type SendFn = Arc<dyn Fn(&J) -> Result<(), String> + Send + Sync>;

/// What one channel session talks to: JSON in via `send`, JSON out via `recv`.
/// Dropping both ends the underlying stream.
pub struct Wire {
    /// Parse + enqueue one client message. `Err` ends the session with
    /// INVALID_ARGUMENT.
    pub send: SendFn,
    pub recv: mpsc::Receiver<Result<J, Status>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Listen,
    Write,
}

#[async_trait]
pub trait ChannelBackend: Send + Sync + 'static {
    /// Start a stream; must return immediately (the first client message
    /// arrives later). `headers` are the already-filtered, lowercase-named
    /// credentials of the handshake request.
    async fn open(&self, kind: Kind, headers: Vec<(String, String)>) -> Wire;
}

#[async_trait]
impl ChannelBackend for FirestoreService {
    async fn open(&self, kind: Kind, headers: Vec<(String, String)>) -> Wire {
        let svc = self.clone();
        match kind {
            Kind::Listen => wire(
                headers,
                listen_request_from_json,
                listen_response_json,
                move |r| async move { svc.handle_listen(r).await },
            ),
            Kind::Write => wire(
                headers,
                write_request_from_json,
                write_response_json,
                move |r| async move { svc.handle_write(r).await },
            ),
        }
    }
}

/// Glue one gRPC-style handler to a [`Wire`]: client JSON is parsed into a
/// request stream, the handler's response stream is rendered back to JSON.
fn wire<Rq, Rs, Fut>(
    headers: Vec<(String, String)>,
    parse: fn(&J) -> Result<Rq, String>,
    render: fn(&Rs) -> J,
    call: impl FnOnce(Request<ReceiverStream<Result<Rq, Status>>>) -> Fut,
) -> Wire
where
    Rq: Send + 'static,
    Rs: Send + 'static,
    Fut: Future<Output = Result<tonic::Response<tonic::codegen::BoxStream<Rs>>, Status>>
        + Send
        + 'static,
{
    let (req_tx, req_rx) = mpsc::channel::<Result<Rq, Status>>(64);
    let (out_tx, out_rx) = mpsc::channel::<Result<J, Status>>(64);
    let mut request = Request::new(ReceiverStream::new(req_rx));
    for (k, v) in &headers {
        if let (Ok(k), Ok(v)) = (
            tonic::metadata::MetadataKey::from_bytes(k.as_bytes()),
            MetadataValue::try_from(v.as_str()),
        ) {
            request.metadata_mut().insert(k, v);
        }
    }
    let fut = call(request);
    tokio::spawn(async move {
        let mut stream = match fut.await {
            Ok(resp) => resp.into_inner(),
            Err(status) => {
                let _ = out_tx.send(Err(status)).await;
                return;
            }
        };
        loop {
            tokio::select! {
                _ = out_tx.closed() => return,
                item = stream.next() => match item {
                    Some(Ok(m)) => {
                        if out_tx.send(Ok(render(&m))).await.is_err() { return; }
                    }
                    Some(Err(status)) => { let _ = out_tx.send(Err(status)).await; return; }
                    None => return,
                },
            }
        }
    });
    Wire {
        send: Arc::new(move |j| {
            let msg = parse(j)?;
            req_tx.try_send(Ok(msg)).map_err(|e| format!("request stream unavailable: {e}"))
        }),
        recv: out_rx,
    }
}

// ── Pure protocol helpers ───────────────────────────────────────────────────

/// One WebChannel chunk: `<len>\n<payload>\n` where `len` counts the payload
/// plus its trailing newline in UTF-16 code units (what JS `substr` sees).
pub fn chunk(payload: &str) -> String {
    let body = format!("{payload}\n");
    format!("{}\n{}", body.encode_utf16().count(), body)
}

pub struct ForwardBody {
    /// Absolute index of the first message in this request (`ofs`).
    pub ofs: u64,
    /// Raw JSON text of each `req{i}___data__` entry, in order.
    pub messages: Vec<String>,
}

/// Parse `count=N&ofs=N&req0___data__=<json>&...` (the `sendRawJson` form the
/// Firestore SDK uses). Key/value map form (`req0_key=value`) is rejected.
pub fn parse_forward_body(body: &[u8]) -> Result<ForwardBody, String> {
    let pairs: HashMap<String, String> = form_urlencoded::parse(body).into_owned().collect();
    let num = |k: &str| -> Result<u64, String> {
        pairs.get(k).map_or(Ok(0), |v| v.parse().map_err(|_| format!("bad {k}")))
    };
    let count = num("count")?;
    let mut messages = Vec::new();
    for i in 0..count {
        let key = format!("req{i}___data__");
        messages.push(
            pairs.get(&key).cloned().ok_or_else(|| format!("missing {key} (sendRawJson only)"))?,
        );
    }
    Ok(ForwardBody { ofs: num("ofs")?, messages })
}

/// Parse the `$httpHeaders` query value: `Name: value\r\nName: value\r\n`.
pub fn parse_http_headers(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

/// Credentials of a handshake: real headers, overridden by `$httpHeaders`
/// (the SDK sends them there to avoid a CORS preflight).
fn credentials(headers: &HeaderMap, overwrite: Option<&String>) -> Vec<(String, String)> {
    let mut out: HashMap<String, String> = FORWARDED_HEADERS
        .iter()
        .filter_map(|n| Some((n.to_string(), headers.get(*n)?.to_str().ok()?.to_string())))
        .collect();
    for (k, v) in overwrite.map(|s| parse_http_headers(s)).unwrap_or_default() {
        if FORWARDED_HEADERS.contains(&k.as_str()) {
            out.insert(k, v);
        }
    }
    out.into_iter().collect()
}

// ── Sessions ────────────────────────────────────────────────────────────────

pub struct Timing {
    /// Back channel sends a `noop` array after this much silence.
    pub keepalive: Duration,
    /// A streaming back-channel response is ended (client re-polls) after
    /// this long; bounds graceful shutdown.
    pub stream_max: Duration,
    /// A session with no back channel and no forward traffic this long is dropped.
    pub session_idle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(25),
            stream_max: Duration::from_secs(60),
            session_idle: Duration::from_secs(120),
        }
    }
}

struct Inner {
    /// Id of the next array to send; array 0 is the handshake `c` array.
    next_array_id: u64,
    /// Arrays sent or pending, not yet acked: `(id, "[id,[payload]]")`.
    buf: VecDeque<(u64, String)>,
    closed: bool,
    /// Bumped per back-channel GET; an older GET sees the change and ends.
    bc_gen: u64,
    attached: bool,
    /// Index of the next forward message expected (dedups retransmits).
    next_msg: u64,
    last_seen: Instant,
}

struct Session {
    send: SendFn,
    inner: Mutex<Inner>,
    changed: watch::Sender<u64>,
    pump: OnceLock<tokio::task::AbortHandle>,
}

impl Session {
    fn new(send: SendFn) -> Self {
        Self {
            send,
            inner: Mutex::new(Inner {
                next_array_id: 1,
                buf: VecDeque::new(),
                closed: false,
                bc_gen: 0,
                attached: false,
                next_msg: 0,
                last_seen: Instant::now(),
            }),
            changed: watch::channel(0).0,
            pump: OnceLock::new(),
        }
    }

    fn wake(&self) {
        self.changed.send_modify(|v| *v += 1);
    }

    /// Queue `payload` (JSON text) as the next array.
    fn push(&self, payload: &str) {
        {
            let mut i = self.inner.lock().unwrap();
            if i.closed {
                return;
            }
            let id = i.next_array_id;
            i.next_array_id += 1;
            i.buf.push_back((id, format!("[{id},[{payload}]]")));
            if i.buf.len() > MAX_BUFFERED_ARRAYS {
                // Client is not acking: drop the session; its next request
                // gets "Unknown SID" and the SDK reconnects.
                i.closed = true;
                i.buf.clear();
            }
        }
        self.wake();
    }

    /// End the stream: tell the client, then no more arrays.
    fn close(&self) {
        self.push("\"stop\"");
        self.inner.lock().unwrap().closed = true;
        self.wake();
    }

    fn ack(&self, upto: u64) {
        self.inner.lock().unwrap().buf.retain(|(id, _)| *id > upto);
    }

    fn touch(&self) {
        self.inner.lock().unwrap().last_seen = Instant::now();
    }

    fn fail(&self, msg: &str) {
        self.push(&status_error_json(&Status::invalid_argument(msg)).to_string());
        self.close();
    }

    /// Hand forward-channel messages to the backend, skipping retransmits.
    fn deliver(&self, fb: ForwardBody) {
        for (n, text) in fb.messages.iter().enumerate() {
            let idx = fb.ofs + n as u64;
            {
                let mut i = self.inner.lock().unwrap();
                if idx < i.next_msg {
                    continue;
                }
                i.next_msg = idx + 1;
            }
            let sent = serde_json::from_str::<J>(text)
                .map_err(|e| format!("bad message JSON: {e}"))
                .and_then(|j| (self.send)(&j));
            if let Err(e) = sent {
                self.fail(&e);
                return;
            }
        }
    }

    fn abort(&self) {
        if let Some(h) = self.pump.get() {
            h.abort();
        }
    }
}

/// Moves backend output into the session buffer until the stream ends.
async fn pump(sess: Arc<Session>, mut rx: mpsc::Receiver<Result<J, Status>>) {
    while let Some(item) = rx.recv().await {
        match item {
            Ok(j) => sess.push(&j.to_string()),
            Err(status) => {
                sess.push(&status_error_json(&status).to_string());
                break;
            }
        }
    }
    sess.close();
}

type Sessions = Mutex<HashMap<String, Arc<Session>>>;

fn sweep(sessions: &Sessions, idle: Duration) {
    sessions.lock().unwrap().retain(|_, s| {
        let i = s.inner.lock().unwrap();
        // A closed session lingers for `idle` too, so the client can still
        // read its final error + `stop` before getting "Unknown SID".
        let dead = !i.attached && i.last_seen.elapsed() > idle;
        drop(i);
        if dead {
            s.abort();
        }
        !dead
    });
}

#[derive(Clone)]
pub struct BrowserChannelState {
    backend: Arc<dyn ChannelBackend>,
    sessions: Arc<Sessions>,
    timing: Arc<Timing>,
}

impl BrowserChannelState {
    /// Must be called inside a tokio runtime (starts the idle sweeper).
    pub fn new(backend: Arc<dyn ChannelBackend>) -> Self {
        Self::with_timing(backend, Timing::default())
    }

    pub fn with_timing(backend: Arc<dyn ChannelBackend>, timing: Timing) -> Self {
        let sessions: Arc<Sessions> = Arc::default();
        let weak: Weak<Sessions> = Arc::downgrade(&sessions);
        let (idle, every) = (timing.session_idle, (timing.session_idle / 4).max(Duration::from_millis(10)));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match weak.upgrade() {
                    Some(s) => sweep(&s, idle),
                    None => return,
                }
            }
        });
        Self { backend, sessions, timing: Arc::new(timing) }
    }

    fn get(&self, sid: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(sid).cloned()
    }

    #[cfg(test)]
    fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

// ── HTTP ────────────────────────────────────────────────────────────────────

pub fn router(state: BrowserChannelState) -> Router {
    Router::new()
        .route("/google.firestore.v1.Firestore/:rpc/channel", get(channel_get).post(channel_post))
        .with_state(state)
}

fn kind_of(rpc: &str) -> Option<Kind> {
    match rpc {
        "Listen" => Some(Kind::Listen),
        "Write" => Some(Kind::Write),
        _ => None,
    }
}

fn plain(status: StatusCode, body: impl Into<Body>) -> Response {
    let mut r = Response::new(body.into());
    *r.status_mut() = status;
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, max-age=0, must-revalidate"),
    );
    r
}

fn unknown_sid() -> Response {
    plain(StatusCode::BAD_REQUEST, "Unknown SID")
}

fn ack_of(q: &HashMap<String, String>) -> u64 {
    q.get("AID").and_then(|a| a.parse().ok()).unwrap_or(0)
}

async fn channel_post(
    State(st): State<BrowserChannelState>,
    Path(rpc): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(kind) = kind_of(&rpc) else { return StatusCode::NOT_FOUND.into_response() };
    match q.get("SID") {
        None => handshake(&st, kind, &q, &headers, &body).await,
        Some(sid) => forward(&st, sid, &q, &body),
    }
}

/// First forward request: open the stream, issue SID + gsessionid, and answer
/// with the `c` control array (`["c", SID, hostPrefix, version, serverVersion, ...]`).
async fn handshake(
    st: &BrowserChannelState,
    kind: Kind,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let creds = credentials(headers, q.get("$httpHeaders"));
    if !creds.iter().any(|(k, _)| k == "authorization") {
        return plain(StatusCode::UNAUTHORIZED, "missing authorization");
    }
    let first = match parse_forward_body(body) {
        Ok(fb) => fb,
        Err(e) => return plain(StatusCode::BAD_REQUEST, e),
    };
    if st.sessions.lock().unwrap().len() >= MAX_SESSIONS {
        return plain(StatusCode::SERVICE_UNAVAILABLE, "too many sessions");
    }
    let wire = st.backend.open(kind, creds).await;
    let sess = Arc::new(Session::new(wire.send));
    let _ = sess.pump.set(tokio::spawn(pump(Arc::clone(&sess), wire.recv)).abort_handle());
    sess.deliver(first);

    let sid = uuid::Uuid::new_v4().simple().to_string();
    let gsessionid = uuid::Uuid::new_v4().simple().to_string();
    st.sessions.lock().unwrap().insert(sid.clone(), sess);

    let mut r = plain(StatusCode::OK, chunk(&format!("[[0,[\"c\",\"{sid}\",\"\",8,12,30000]]]")));
    r.headers_mut().insert(
        HeaderName::from_static("x-http-session-id"),
        HeaderValue::from_str(&gsessionid).expect("hex is a valid header value"),
    );
    r
}

/// Later forward requests: ack, dedup, deliver, or terminate.
fn forward(st: &BrowserChannelState, sid: &str, q: &HashMap<String, String>, body: &[u8]) -> Response {
    let Some(sess) = st.get(sid) else { return unknown_sid() };
    sess.touch();
    sess.ack(ack_of(q));
    if q.get("TYPE").map(String::as_str) == Some("terminate") {
        if let Some(s) = st.sessions.lock().unwrap().remove(sid) {
            s.abort();
        }
        return plain(StatusCode::OK, "");
    }
    match parse_forward_body(body) {
        Ok(fb) => sess.deliver(fb),
        Err(e) => return plain(StatusCode::BAD_REQUEST, e),
    }
    let i = sess.inner.lock().unwrap();
    plain(StatusCode::OK, chunk(&format!("[{},{},0]", u8::from(i.attached), i.next_array_id - 1)))
}

async fn channel_get(
    State(st): State<BrowserChannelState>,
    Path(rpc): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if kind_of(&rpc).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(sid) = q.get("SID") else {
        // Test-channel phase 1 (`MODE=init`): no proxy-detection hosts.
        // ponytail: phase 2 (buffering-proxy probe) not implemented.
        return if q.get("MODE").map(String::as_str) == Some("init") {
            plain(StatusCode::OK, "[\"\",\"\"]")
        } else {
            plain(StatusCode::BAD_REQUEST, "missing SID")
        };
    };
    let Some(sess) = st.get(sid) else { return unknown_sid() };

    // CI=1: client disabled streaming (long polling). Otherwise stream.
    let long_poll = q.get("CI").map(String::as_str) == Some("1");
    let my_gen = {
        let mut i = sess.inner.lock().unwrap();
        i.buf.retain(|(id, _)| *id > ack_of(&q));
        i.bc_gen += 1;
        i.attached = true;
        i.last_seen = Instant::now();
        i.bc_gen
    };
    sess.wake(); // ends a superseded back channel
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    tokio::spawn(serve_back(sess, my_gen, ack_of(&q), long_poll, Arc::clone(&st.timing), tx));
    let mut r = plain(StatusCode::OK, Body::from_stream(ReceiverStream::new(rx)));
    r.headers_mut()
        .insert(HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff"));
    r
}

/// Writes un-acked arrays to one back-channel response as they appear.
async fn serve_back(
    sess: Arc<Session>,
    my_gen: u64,
    ack: u64,
    long_poll: bool,
    timing: Arc<Timing>,
    tx: mpsc::Sender<Result<Bytes, Infallible>>,
) {
    let started = Instant::now();
    let mut sent = ack;
    let mut changes = sess.changed.subscribe();
    loop {
        let (arrays, closed, superseded) = {
            let i = sess.inner.lock().unwrap();
            let new: Vec<(u64, String)> =
                i.buf.iter().filter(|(id, _)| *id > sent).cloned().collect();
            (new, i.closed, i.bc_gen != my_gen)
        };
        if superseded {
            return; // a newer GET owns `attached`
        }
        if let Some((last, _)) = arrays.last() {
            sent = *last;
            let payload = format!(
                "[{}]",
                arrays.iter().map(|(_, a)| a.as_str()).collect::<Vec<_>>().join(",")
            );
            if tx.send(Ok(Bytes::from(chunk(&payload)))).await.is_err() {
                break; // client went away
            }
            if long_poll {
                break;
            }
            continue;
        }
        if closed || (!long_poll && started.elapsed() > timing.stream_max) {
            break;
        }
        tokio::select! {
            _ = changes.changed() => {}
            _ = tx.closed() => break,
            _ = tokio::time::sleep(timing.keepalive) => sess.push("\"noop\""),
        }
    }
    let mut i = sess.inner.lock().unwrap();
    if i.bc_gen == my_gen {
        i.attached = false;
        i.last_seen = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    /// Echo backend: every client message comes back as `{"echo": msg}`;
    /// `{"fail": true}` ends the stream with PERMISSION_DENIED.
    struct Echo;

    #[async_trait]
    impl ChannelBackend for Echo {
        async fn open(&self, _kind: Kind, headers: Vec<(String, String)>) -> Wire {
            let (tx, rx) = mpsc::channel(64);
            let authed = headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer k");
            Wire {
                send: Arc::new(move |j| {
                    if !authed {
                        let _ = tx.try_send(Err(Status::unauthenticated("bad key")));
                    } else if j.get("fail").is_some() {
                        let _ = tx.try_send(Err(Status::permission_denied("nope")));
                    } else {
                        let _ = tx.try_send(Ok(serde_json::json!({ "echo": j })));
                    }
                    Ok(())
                }),
                recv: rx,
            }
        }
    }

    fn app(timing: Timing) -> (BrowserChannelState, Router) {
        let st = BrowserChannelState::with_timing(Arc::new(Echo), timing);
        (st.clone(), router(st))
    }

    fn fast() -> Timing {
        Timing {
            keepalive: Duration::from_millis(60),
            stream_max: Duration::from_secs(5),
            session_idle: Duration::from_secs(60),
        }
    }

    const LISTEN: &str = "/google.firestore.v1.Firestore/Listen/channel";

    fn form(ofs: u64, msgs: &[&str]) -> String {
        let mut s = format!("count={}&ofs={ofs}", msgs.len());
        for (i, m) in msgs.iter().enumerate() {
            s += &format!("&req{i}___data__={}", urlenc(m));
        }
        s
    }

    fn urlenc(s: &str) -> String {
        form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }

    async fn call(app: &Router, method: &str, uri: &str, auth: bool, body: String) -> Response {
        let mut b = axum::http::Request::builder().method(method).uri(uri);
        if auth {
            b = b.header("authorization", "Bearer k");
        }
        app.clone().oneshot(b.body(Body::from(body)).unwrap()).await.unwrap()
    }

    async fn text_of(r: Response) -> String {
        String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    /// Split a body into chunks the way closure's `getNextChunk_` does.
    fn chunks(mut s: &str) -> Vec<String> {
        let mut out = vec![];
        while !s.is_empty() {
            let (len, rest) = s.split_once('\n').unwrap();
            let units: Vec<u16> = rest.encode_utf16().collect();
            let n: usize = len.trim().parse().unwrap();
            out.push(String::from_utf16(&units[..n]).unwrap().trim().to_string());
            s = &rest[String::from_utf16(&units[..n]).unwrap().len()..];
        }
        out
    }

    async fn open_session(app: &Router) -> String {
        let r = call(app, "POST", &format!("{LISTEN}?VER=8&RID=1&CVER=22"), true, form(0, &[])).await;
        assert_eq!(r.status(), 200);
        assert!(r.headers().contains_key("x-http-session-id"));
        let body = text_of(r).await;
        let c = &chunks(&body)[0];
        let v: J = serde_json::from_str(c).unwrap();
        assert_eq!(v[0][0], 0);
        assert_eq!(v[0][1][0], "c");
        v[0][1][1].as_str().unwrap().to_string()
    }

    async fn poll(app: &Router, sid: &str, aid: u64) -> Vec<J> {
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=rpc&AID={aid}&CI=1&TYPE=xmlhttp");
        let body = text_of(call(app, "GET", &uri, false, String::new()).await).await;
        chunks(&body).iter().flat_map(|c| serde_json::from_str::<Vec<J>>(c).unwrap()).collect()
    }

    async fn send(app: &Router, sid: &str, ofs: u64, msgs: &[&str]) -> String {
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=2&AID=0");
        text_of(call(app, "POST", &uri, false, form(ofs, msgs)).await).await
    }

    #[test]
    fn chunk_counts_utf16_units_and_trailing_newline() {
        assert_eq!(chunk("[1,0,0]"), "8\n[1,0,0]\n");
        // U+1F600 is 2 UTF-16 units; "é" is 1.
        let c = chunk("[\"é😀\"]");
        assert!(c.starts_with("8\n"), "{c}");
        assert_eq!(chunks(&format!("{}{}", chunk("[1]"), chunk("[2,3]"))), vec!["[1]", "[2,3]"]);
    }

    #[test]
    fn forward_body_parses_count_ofs_and_raw_json() {
        let fb = parse_forward_body(form(5, &[r#"{"a":"b&c"}"#, "[1]"]).as_bytes()).unwrap();
        assert_eq!(fb.ofs, 5);
        assert_eq!(fb.messages, vec![r#"{"a":"b&c"}"#, "[1]"]);
        assert!(parse_forward_body(b"count=1&ofs=0&req0_x=y").is_err());
        assert_eq!(parse_forward_body(b"count=0&ofs=0").unwrap().messages.len(), 0);
    }

    #[test]
    fn http_headers_param_parses_crlf_lines() {
        let h = parse_http_headers("Authorization: Bearer abc\r\nX-Goog-Api-Client: gl-js/ fire/1\r\n");
        assert_eq!(h[0], ("authorization".into(), "Bearer abc".into()));
        assert_eq!(h[1].0, "x-goog-api-client");
    }

    #[tokio::test]
    async fn handshake_issues_sid_and_requires_authorization() {
        let (_, app) = app(fast());
        let r = call(&app, "POST", &format!("{LISTEN}?VER=8"), false, form(0, &[])).await;
        assert_eq!(r.status(), 401);
        assert!(!open_session(&app).await.is_empty());
    }

    #[tokio::test]
    async fn authorization_may_arrive_via_httpheaders_param() {
        let (_, app) = app(fast());
        let hdr = urlenc("Authorization: Bearer k\r\n");
        let uri = format!("{LISTEN}?VER=8&%24httpHeaders={hdr}");
        let r = call(&app, "POST", &uri, false, form(0, &[])).await;
        assert_eq!(r.status(), 200);
    }

    #[tokio::test]
    async fn forward_message_comes_back_on_long_poll_with_array_ids() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        let resp = send(&app, &sid, 0, &[r#"{"x":1}"#]).await;
        assert!(chunks(&resp)[0].starts_with("[0,"), "no back channel attached yet: {resp}");
        let arrays = poll(&app, &sid, 0).await;
        assert_eq!(arrays, vec![serde_json::json!([1, [{"echo": {"x": 1}}]])]);
    }

    #[tokio::test]
    async fn unacked_arrays_are_resent_and_acked_ones_are_not() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &[r#"{"n":1}"#]).await;
        assert_eq!(poll(&app, &sid, 0).await.len(), 1);
        // Not acked (AID=0): the same array is delivered again.
        assert_eq!(poll(&app, &sid, 0).await[0][0], 1);
        // Acked (AID=1): nothing but a keepalive noop (a new array id).
        let a = poll(&app, &sid, 1).await;
        assert_eq!(a, vec![serde_json::json!([2, ["noop"]])]);
    }

    #[tokio::test]
    async fn retransmitted_forward_message_is_delivered_once() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &[r#"{"n":1}"#]).await;
        send(&app, &sid, 0, &[r#"{"n":1}"#]).await; // retransmit
        send(&app, &sid, 1, &[r#"{"n":2}"#]).await;
        tokio::time::sleep(Duration::from_millis(30)).await; // let the pump queue both echoes
        let a = poll(&app, &sid, 0).await;
        let echoes: Vec<_> = a.iter().map(|x| x[1][0]["echo"]["n"].clone()).collect();
        assert_eq!(echoes, vec![1, 2]);
    }

    #[tokio::test]
    async fn forward_response_reports_back_channel_and_last_array_id() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &[r#"{"n":1}"#]).await;
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=rpc&AID=0&CI=0&TYPE=xmlhttp");
        let stream = call(&app, "GET", &uri, false, String::new()).await; // attach back channel
        tokio::time::sleep(Duration::from_millis(20)).await;
        let resp = send(&app, &sid, 1, &[r#"{"n":2}"#]).await;
        // `[backChannelPresent, lastArrayId, outstandingBytes]`; the id depends on pump timing.
        assert!(chunks(&resp)[0].starts_with("[1,") && chunks(&resp)[0].ends_with(",0]"), "{resp}");
        drop(stream);
    }

    #[tokio::test]
    async fn streaming_back_channel_delivers_arrays_as_they_arrive() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=rpc&AID=0&CI=0&TYPE=xmlhttp");
        let mut body = call(&app, "GET", &uri, false, String::new()).await.into_body();
        send(&app, &sid, 0, &[r#"{"n":1}"#]).await;
        let f = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.unwrap().unwrap().unwrap();
        let first = String::from_utf8(f.into_data().unwrap().to_vec()).unwrap();
        assert!(chunks(&first)[0].contains(r#""echo":{"n":1}"#), "{first}");
        send(&app, &sid, 1, &[r#"{"n":2}"#]).await;
        let f = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.unwrap().unwrap().unwrap();
        let second = String::from_utf8(f.into_data().unwrap().to_vec()).unwrap();
        assert!(second.contains(r#""n":2"#), "same response stayed open: {second}");
    }

    #[tokio::test]
    async fn newer_back_channel_supersedes_the_older_one() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=rpc&AID=0&CI=0&TYPE=xmlhttp");
        let old = call(&app, "GET", &uri, false, String::new()).await;
        let _new = call(&app, "GET", &uri, false, String::new()).await;
        // The old response ends (empty body) instead of competing for arrays.
        let ended = tokio::time::timeout(Duration::from_secs(2), old.into_body().collect()).await;
        assert!(ended.is_ok());
    }

    #[tokio::test]
    async fn backend_error_is_delivered_as_error_message_then_stop() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &[r#"{"fail":true}"#]).await;
        tokio::time::sleep(Duration::from_millis(30)).await; // error + stop queued
        let a = poll(&app, &sid, 0).await;
        assert_eq!(a[0][1][0]["error"]["status"], "PERMISSION_DENIED");
        assert_eq!(a[1][1][0], "stop");
    }

    #[tokio::test]
    async fn bad_credentials_surface_on_the_back_channel() {
        let (_, app) = app(fast());
        let hdr = urlenc("Authorization: Bearer wrong\r\n");
        let uri = format!("{LISTEN}?VER=8&%24httpHeaders={hdr}");
        let r = call(&app, "POST", &uri, false, form(0, &[r#"{"database":"d"}"#])).await;
        let sid = {
            let v: J = serde_json::from_str(&chunks(&text_of(r).await)[0]).unwrap();
            v[0][1][1].as_str().unwrap().to_string()
        };
        let a = poll(&app, &sid, 0).await;
        assert_eq!(a[0][1][0]["error"]["status"], "UNAUTHENTICATED");
    }

    #[tokio::test]
    async fn malformed_message_json_ends_session_with_invalid_argument() {
        let (_, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &["{not json"]).await;
        assert_eq!(poll(&app, &sid, 0).await[0][1][0]["error"]["status"], "INVALID_ARGUMENT");
    }

    #[tokio::test]
    async fn terminate_removes_session() {
        let (st, app) = app(fast());
        let sid = open_session(&app).await;
        assert_eq!(st.session_count(), 1);
        let uri = format!("{LISTEN}?VER=8&SID={sid}&RID=9&TYPE=terminate");
        assert_eq!(call(&app, "POST", &uri, false, "count=0&ofs=0".into()).await.status(), 200);
        assert_eq!(st.session_count(), 0);
        let uri = format!("{LISTEN}?SID={sid}&RID=rpc&AID=0&CI=1&TYPE=xmlhttp");
        assert_eq!(call(&app, "GET", &uri, false, String::new()).await.status(), 400);
    }

    #[tokio::test]
    async fn unknown_sid_missing_sid_and_unknown_rpc() {
        let (_, app) = app(fast());
        let uri = format!("{LISTEN}?SID=nope&RID=rpc&AID=0&CI=1&TYPE=xmlhttp");
        let r = call(&app, "GET", &uri, false, String::new()).await;
        assert_eq!(r.status(), 400);
        assert_eq!(text_of(r).await, "Unknown SID");
        assert_eq!(call(&app, "GET", LISTEN, false, String::new()).await.status(), 400);
        let uri = "/google.firestore.v1.Firestore/Nope/channel?VER=8";
        assert_eq!(call(&app, "POST", uri, true, form(0, &[])).await.status(), 404);
    }

    #[tokio::test]
    async fn idle_sessions_are_swept_but_attached_ones_survive() {
        let (st, app) = app(fast());
        let idle = open_session(&app).await;
        let live = open_session(&app).await;
        let uri = format!("{LISTEN}?SID={live}&RID=rpc&AID=0&CI=0&TYPE=xmlhttp");
        let _bc = call(&app, "GET", &uri, false, String::new()).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        sweep(&st.sessions, Duration::ZERO);
        assert!(st.get(&idle).is_none());
        assert!(st.get(&live).is_some());
    }

    #[tokio::test]
    async fn closed_session_lingers_then_is_swept() {
        let (st, app) = app(fast());
        let sid = open_session(&app).await;
        send(&app, &sid, 0, &[r#"{"fail":true}"#]).await;
        poll(&app, &sid, 0).await; // reads error + stop, back channel detaches
        tokio::time::sleep(Duration::from_millis(30)).await;
        sweep(&st.sessions, Duration::from_secs(60));
        assert!(st.get(&sid).is_some(), "drained-but-recent session must linger for the client");
        sweep(&st.sessions, Duration::ZERO);
        assert!(st.get(&sid).is_none());
    }

    #[tokio::test]
    async fn test_channel_phase_one_answers() {
        let (_, app) = app(fast());
        let r = call(&app, "GET", &format!("{LISTEN}?VER=8&MODE=init"), false, String::new()).await;
        assert_eq!(text_of(r).await, "[\"\",\"\"]");
    }
}

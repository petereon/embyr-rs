# browser-channel — feature delta

Closes go-live finding **P3** (`docs/product/go-live-assessment-2026-11-13.md`): `rest/browser_channel.rs` was a stub (`/channel`, opaque bytes), so the Firestore web SDK's long-polling fallback got no real-time. Scope: make `onSnapshot` (collection query) and `setDoc` work over BrowserChannel. Wire reference: Closure WebChannel as used by firebase-js-sdk `webchannel_connection.ts`.

## Design

**Routes** (:8081, axum, behind the existing CORS / 2 MiB body-limit layers): `GET|POST /google.firestore.v1.Firestore/{Listen|Write}/channel`. Any other rpc segment is 404. The old `/channel` stub is removed.

**Layering.** `rest/browser_channel.rs` is protocol only: sessions, framing, acks, expiry. It exchanges proto3-JSON with a `ChannelBackend` port (`open(kind, credentials) -> Wire{send, recv}`). `FirestoreService` implements it by calling the same `handle_listen` / `handle_write` the native gRPC path runs, now generic over the request stream (`tonic::Streaming` or a channel-backed stream). Authentication, rate limiting, suspension, client identity, security rules, multi-target Listen and the Write token loop are therefore the gRPC code, not a fork. `encoding/proto_json.rs` holds the hand-written JSON<->prost subset (no new dependency except `form_urlencoded`, already in the tree).

**Protocol.**
- *Handshake*: `POST ...?VER=8&RID&CVER&X-HTTP-Session-Id=gsessionid[&$httpHeaders=...]`, body `count=N&ofs=N&req0___data__=<json>`. Credentials (`authorization`, `x-embyr-client-identity`) come from real headers, overridden by the `$httpHeaders` param (`Name: value\r\n...`), which is how the SDK avoids a CORS preflight. No `authorization` -> 401. Response: `<len>\n[[0,["c",SID,"",8,12,30000]]]\n` plus `X-HTTP-Session-Id: <gsessionid>` (exposed via CORS). The session is created before any message exists (the SDK handshakes with `count=0`); the backend task waits for the first message.
- *Framing*: `<len>\n<payload>\n`, `len` = UTF-16 units of payload + newline. Back-channel payload is `[[arrayId,[msg,...]],...]`, `msg` = ListenResponse / WriteResponse JSON. Arrays are numbered from 1 per session.
- *Forward channel*: `POST ...&SID=&RID=&AID=`; messages are deduplicated by absolute index (`ofs+i`) so SDK retransmits are applied once. Response `[backChannelPresent, lastArrayId, 0]`.
- *Back channel*: `GET ...&SID=&RID=rpc&AID=&CI=&TYPE=xmlhttp`. `CI=0` streams (chunks flushed as arrays appear, response ends after 60 s so graceful shutdown is bounded); `CI=1` long-polls (one response, then end). `noop` array after 25 s of silence. A newer GET supersedes an older one.
- *Acks*: `AID=n` (on either GET or POST) drops arrays <= n; arrays > n are replayed to a new back channel. Buffer cap 4096 un-acked arrays, then the session is dropped.
- *Errors*: a gRPC status from the backend is sent as `{"error":{"code","message","status"}}` (what the SDK maps to `FirestoreError`) followed by a `stop` array; a malformed client message does the same with INVALID_ARGUMENT.
- *Terminate*: `TYPE=terminate` removes the session. Unknown/expired SID -> `400 Unknown SID`.
- *Expiry*: a session with no attached back channel and no forward traffic for 120 s is swept (also bounds unauthenticated handshakes together with a 10 000-session cap); the sweep aborts the pump, which drops the gRPC stream.

## NOT supported

- Key/value map messages (`req0_key=value`); only `sendRawJson` form (`req{i}___data__`, what Firestore uses).
- Test channel phase 2 (buffering-proxy probe used by `experimentalAutoDetectLongPolling`); phase 1 (`MODE=init`) answers `["",""]` unverified.
- `TYPE=html` (iframe) back channel; `$req`/POST-over-GET; compression.
- JSON conversion covers: Listen add/remove target (query: from/where/orderBy/limit/offset/cursors/select; documents), Write update/delete + mask + precondition + all field transforms. Not converted: the legacy `Write.transform` operation. `geoPointValue` converts but the domain model rejects it, as on gRPC.
- Per-call gRPC metrics (`record_grpc_call`) are not emitted for BrowserChannel sessions.
- Session state is in-process: no sharing across instances, no resume after a server restart (the SDK reconnects and re-lists; Listen resume tokens work as on gRPC). Needs sticky routing if run behind multiple instances.
- Not verified against a real firebase-js-sdk client (see PR).

# ADR-004: gRPC Framework Selection

## Status

Accepted

## Context

embyr-rs must implement the server side of the Google Firestore gRPC API surface (`google.firestore.v1.Firestore` service). The Firestore API uses all four gRPC streaming patterns:

| gRPC pattern | Firestore RPC | Stream lifetime |
|-------------|---------------|----------------|
| Unary | `GetDocument`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `BeginTransaction`, `Commit`, `Rollback`, `RunAggregationQuery` | Single request/response |
| Server-streaming | `RunQuery`, `BatchGetDocuments` | Short-lived (query result set) |
| Client-streaming | `Write` (batched write stream) | Medium-lived (SDK-controlled) |
| Bidirectional streaming | `Listen` | Long-lived (entire SDK session) |

The `Listen` RPC is the architecturally most demanding: it is a long-lived bidirectional stream where the server pushes `ListenResponse` messages to the client whenever a watched document changes. The server must be able to maintain thousands of concurrent `Listen` streams, each as a separate async task, and inject messages from the Postgres NOTIFY fan-out registry without blocking the client-reading side of the stream.

Additionally, the REST port (8081) must support:
- **gRPC-Web**: browser clients that cannot speak raw HTTP/2 gRPC need gRPC-Web framing (HTTP/1.1 with content-type `application/grpc-web+proto`). The server must transcode between gRPC-Web and native gRPC.
- **grpc-gateway REST/JSON**: `RunQuery`, `BatchGet`, `RunAggregationQuery` have REST transcoding bindings from the Firestore proto `google.api.http` options.

### Quality Attribute Priorities

1. **Protocol fidelity**: The framework must faithfully implement the gRPC spec (HTTP/2 framing, trailer handling, status codes, flow control). Any deviation silently breaks Firebase SDK state machines.
2. **Streaming correctness**: `Listen` stream must handle slow consumers without deadlocking, must support graceful server-initiated close, and must interoperate with Tokio channels for the NOTIFY fan-out.
3. **Interceptor / middleware support**: Auth middleware, rate limiting, and tracing spans must apply to all RPC handlers without per-handler boilerplate.
4. **gRPC-Web and REST transcoding**: Both must be achievable within the same server binary without a separate proxy process (AC-13d, SD-01).

## Decision

**Use `tonic` 0.12.x for the gRPC server, with `tonic-web` for gRPC-Web transcoding.**

Rationale by requirement:

**Streaming**: Tonic's `Streaming<T>` (inbound) and `mpsc::channel` + `ReceiverStream` (outbound) pattern provides a clean interface for bidirectional streaming. The Listen handler pattern is:

```
// Pseudocode — not implementation
async fn listen(request: Request<Streaming<ListenRequest>>) -> Result<Response<impl Stream<Item=...>>> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    // Register tx in ListenRegistry; spawn task to consume ListenRequest stream.
    // Return ReceiverStream(rx) as the server's outbound stream.
}
```

The `tx` sender is handed to the Listen registry. When the registry receives a `DocChange`, it sends a `ListenResponse` through `tx`. The gRPC stream delivers it to the client. This is the standard Tonic bidirectional streaming pattern.

**Interceptors**: Tonic 0.12 provides `tonic::service::interceptor` for unary request interception and the `tower::Layer` / `tower::Service` model for middleware that wraps the entire service. The auth interceptor is a `tower::Layer` applied to the `FirestoreServer` at composition time. This ensures it runs for every RPC method including streaming methods.

**gRPC-Web**: `tonic-web` (included in the Tonic 0.12 release family) provides a `GrpcWebLayer` that handles gRPC-Web framing. It is applied as a `tower::Layer` to the Axum router handling the REST port. CORS headers are added by a separate `tower-http::cors::CorsLayer`. No external proxy required.

**REST/JSON transcoding (grpc-gateway)**: The `prost-wkt` and `prost-wkt-types` crates provide JSON serialization of proto messages. The grpc-gateway REST mux is implemented as a thin Axum handler that deserializes the JSON request body into the proto type, invokes the gRPC handler directly (bypassing TCP), and serializes the proto response to JSON. This avoids a separate `grpc-gateway` process.

**Proto codegen**: `tonic-build` in `embyr-proto/build.rs` compiles the `.proto` files using `prost` as the underlying protobuf encoder. The `.proto` files for the Firestore API are sourced from the `googleapis` repository and vendored into the codebase under `proto/`.

**Tonic version rationale**: Tonic 0.12.x is the current stable release (2025). It targets Tokio 1.x (ADR-003) and prost 0.13.x. Tonic 0.11 and earlier had a known limitation with bidirectional streaming backpressure that was resolved in 0.12.

## Alternatives Considered

### Alternative A: `grpcio` (gRPC-rs, C core binding)

`grpcio` is a Rust binding to the official gRPC C core library. It provides a Rust API over the battle-tested C implementation used by the official gRPC SDKs.

**Rejected because:**
- `grpcio` is not async-native for Tokio. It uses its own event loop (`grpc::Environment`). Integrating it with a Tokio-based application requires bridging between the grpcio event loop and the Tokio runtime, creating two concurrency models within the same process.
- The C core dependency introduces a C linkage requirement (libgrpc.so or static link). This complicates static binary compilation for `embyr-agent`, which requires a self-contained binary.
- Tonic's bidirectional streaming API is significantly more ergonomic in Rust async/await style than grpcio's callback-based or future-based API.
- The `grpcio` crate is maintained by PingCAP (TiKV project). While production-proven, its maintenance priority is TiDB/TiKV-specific workloads; Tokio integration is not a first-class concern.
- `tonic-web` and the REST transcoding layer (via `axum` + `tower`) are not available for `grpcio`. The gRPC-Web requirement would require an external proxy (nginx, envoy) — violating AC-13d.

### Alternative B: `h2` + hand-rolled gRPC framing

Use the `h2` crate (HTTP/2 implementation) directly and implement gRPC framing (length-prefixed messages, trailers, status codes) manually. This avoids all framework abstractions.

**Rejected because:**
- gRPC framing is non-trivial: 5-byte length-prefix, compression flag, message encoding, trailer-only responses for error status codes, and flow control semantics. The Firestore proto surface has 15+ RPC methods; implementing all framing edge cases correctly for all streaming patterns represents weeks of work with no correctness advantage over `tonic` (which has been validated against the official gRPC compliance tests).
- Tonic's proto codegen (`tonic-build`) eliminates the boilerplate of matching request/response types to method names, routing inbound frames to handlers, and encoding/decoding proto messages. Hand-rolling this is pure implementation risk with no architectural benefit.
- Protocol fidelity is the #1 quality attribute. Any deviation in gRPC framing (e.g., incorrect trailer handling for `ABORTED` status on OCC conflict) silently breaks the Firebase SDK's retry logic. Tonic's compliance with gRPC spec is production-validated.

### Alternative C: `tower-grpc` (predecessor to `tonic`)

`tower-grpc` was the predecessor library before `tonic` was created. It was built on the Tower middleware model.

**Rejected because:**
- `tower-grpc` is unmaintained. The project was superseded by `tonic` in 2019. Last commit: 2019. No Tokio 1.x support.

### Alternative D: Implement gRPC-Web via an external proxy (Envoy / nginx)

Keep the gRPC server (Tonic, port 8080) gRPC-only, and add an Envoy sidecar to transcode gRPC-Web on port 8081.

**Rejected because:**
- AC-13d explicitly states: "No separate server process required for browser transports." Envoy is a separate server process.
- The BrowserChannel protocol (Firebase JS SDK's long-poll transport) is not gRPC-Web — it is a completely different HTTP-based protocol. Envoy cannot proxy BrowserChannel; a custom handler is required regardless. If a custom handler is required anyway, the gRPC-Web bridge in the same process via `tonic-web` adds negligible complexity.
- Envoy as a sidecar creates a new operational dependency (Envoy version management, config synchronization with embyr behavior, additional failure mode: Envoy crash drops browser clients while native gRPC clients continue). This contradicts the Operational Simplicity quality attribute (rank 6 in the System Architecture section).
- The sticky routing requirement for BrowserChannel is implemented at the load balancer, not at the sidecar. Envoy as a sidecar does not help with sticky routing.

## Consequences

### Positive

- Single framework for all four gRPC streaming patterns, gRPC-Web, and REST transcoding — no external processes.
- Tonic's `tower::Layer` model enables auth, rate limiting, and tracing middleware to be composed uniformly for all RPC methods and all transports (native gRPC on port 8080, gRPC-Web on port 8081 via the bridge).
- `tonic-build` proto codegen is deterministic and reproducible. Proto files are vendored; no network access required during build.
- Tonic is the most-used Rust gRPC implementation in production (used by Databricks, Cloudflare, and others). Protocol compliance is production-validated.
- The `ReceiverStream` + `mpsc::channel` pattern for server-streaming/bidirectional streaming is idiomatic Tokio and well-documented. The Listen handler pattern maps directly to this.

### Trade-offs and Costs

- **Tonic's interceptor API for bidirectional streaming is limited.** Tonic `interceptor` (the simple API) only intercepts unary requests — it cannot inspect or modify streaming request/response bodies. The auth interceptor must use the `tower::Layer` API (which wraps the entire service, including streaming) rather than the simpler `interceptor` function. This is a known limitation documented in Tonic 0.12; the workaround (Tower layer) is the standard approach.
- **gRPC-Web does not support bidirectional streaming.** The gRPC-Web spec allows server-streaming but not client-streaming or bidirectional streaming (browsers cannot initiate bidirectional HTTP/1.1 streams). The Firebase JS SDK works around this via BrowserChannel for `Listen` and `Write` streams. `tonic-web` is only used for unary and server-streaming RPCs from the browser; BrowserChannel is the browser's bidirectional stream transport. This split is handled by the REST port routing decision tree (ADR-001).
- **Proto vendoring requires manual sync** when the Firestore proto surface changes. The `proto/` directory must be updated when Google releases new Firestore API versions. This is acceptable: embyr implements a fixed protocol surface (google.firestore.v1); Google's API versioning provides stability windows.
- **`prost` (Tonic's proto encoder) does not support the `Any` type's full JSON name resolution** required for some `google.api.http` transcoding edge cases. If a REST transcoding route requires `Any` type resolution, a custom handler is needed. Audit of the Firestore REST surface shows this affects only the `ListDocuments` method, which is not in scope for the current feature set.

## References

- Tonic 0.12 documentation: https://docs.rs/tonic/0.12.0/tonic/
- tonic-web documentation: https://docs.rs/tonic-web/0.12.0/tonic_web/
- gRPC specification: https://grpc.io/docs/what-is-grpc/introduction/
- gRPC-Web specification: https://github.com/grpc/grpc-web
- Firestore gRPC proto: https://github.com/googleapis/googleapis/tree/master/google/firestore/v1
- `docs/product/architecture/adr-003-async-runtime.md` — Tokio runtime (prerequisite)
- `docs/product/architecture/adr-001-process-topology.md` — single binary requirement (AC-13d)
- `docs/product/architecture/brief.md` §§ Application Architecture (driving ports, technology choices)

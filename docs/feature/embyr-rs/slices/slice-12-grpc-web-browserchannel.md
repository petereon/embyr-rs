# Slice 12 — gRPC-Web + BrowserChannel Transport

**Goal**: Browser-based Firebase apps (web SDK) work against embyr using gRPC-Web and BrowserChannel transports.

## IN scope
- gRPC-Web framing layer (HTTP/1.1 compatible, `application/grpc-web+proto` content-type)
- gRPC-Web trailers encoding (packed into body, not HTTP trailers)
- BrowserChannel (Firebase-specific long-poll): `POST /google.firestore.v1.Firestore/Listen` new-session + resume
- `${browser_channel_sid}`: 24-hex random, issued on new-session; required on all subsequent BC requests
- All existing operations (read, write, query, listen) available via both transports
- REST/grpc-gateway: JSON transcoding for all RPCs (for SDK compatibility)

## OUT scope
- mTLS on data transports (operator concern; handled at reverse proxy level)

## Learning Hypothesis
Disproves: "Browser transports require a separate server process or cgo dependency."
Confirms if: the same Rust binary serves gRPC-Web and BrowserChannel on the same port as pure gRPC, with no additional process or native library.

## Acceptance Criteria
- Firebase JS web SDK connects via gRPC-Web and successfully reads/writes documents
- `onSnapshot` works via BrowserChannel (receives initial snapshot + live changes)
- `browser_channel_sid` required after session establishment (requests without it return 400)
- Trailers are embedded in gRPC-Web response body (not HTTP trailers)

## Dependencies
S01, S02, S06, S07 (all core operations must work first)

## Effort estimate
≤1 day

## Pre-slice SPIKE
Validate that `h2` + `tower` middleware can demux gRPC-Web from pure gRPC on the same port; identify if a separate HTTP/1.1 listener is needed.

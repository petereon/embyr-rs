# Slice 01 — gRPC Server + GetDocument + direct_pg

**Goal**: Firebase SDK points at embyr and successfully reads a document from a real Postgres DB.

## IN scope
- Tonic gRPC server serving `google.firestore.v1.Firestore`
- `GetDocument` RPC handler
- Static-key `Authorization: Bearer` authentication (Argon2id hash check)
- `direct_pg` backend adapter: Postgres connection pool, `SELECT` by path
- Firestore proto encoding/decoding (Document, Value, Timestamp)
- `/healthz` HTTP health endpoint

## OUT scope
- Writes, queries, streaming
- Admin API (project hardcoded in config for this slice)
- gRPC-Web, REST, BrowserChannel transports
- Rate limiting, metrics

## Learning Hypothesis
Disproves: "Firestore proto encoding is too complex to implement correctly without a transcription layer."
Confirms if: Firebase JS SDK reads a document and `snapshot.data()` equals the DB row.

## Acceptance Criteria
- `getDoc(doc(db, "users", "alice"))` returns a document with correct fields
- Wrong Bearer token returns `UNAUTHENTICATED`
- Missing document returns `NOT_FOUND`
- `/healthz` returns 200 while Postgres is up

## Dependencies
None (foundation slice)

## Effort estimate
≤1 day (reference: gRPC server + single handler + Postgres adapter)

## Pre-slice SPIKE
None required; Tonic + sqlx are well-understood.

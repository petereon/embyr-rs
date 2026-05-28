# Story Map — embyr-rs

> Feature: embyr-rs full implementation
> Wave: DISCUSS / Phase 2.5
> Updated: 2026-05-23

---

## Activity Backbone

| A1: Provision & Configure | A2: Authenticate | A3: Write Docs | A4: Read Docs | A5: Query | A6: Stream Changes | A7: Transactions | A8: DB Connectivity | A9: Lifecycle | A10: Metering |
|---|---|---|---|---|---|---|---|---|---|
| Admin create project (direct_pg) | Validate Bearer token (static_key) | CreateDocument | GetDocument | RunQuery (simple) | Listen (initial snapshot) | BeginTransaction | direct_pg backend | Suspend project | Usage metrics table |
| Admin create project (aws_secret) | Argon2id key hash check | UpdateDocument | GetDocument with transaction | RunQuery (composite + order) | Listen (live NOTIFY push) | Commit | aws_secret backend | Delete project | Ingress/egress tracking |
| Admin create project (gcp_secret) | Project status check (suspended → reject) | DeleteDocument | BatchGetDocuments | RunQuery (collection group) | Listen (resume token + delta) | Rollback | gcp_secret backend | Sweeper | CPU tracking |
| Admin create project (agent) | OAuth2 token validation | Write stream (batched) | | RunQuery (cursor pagination) | gRPC-Web transport | | agent backend (mTLS) | | |
| Admin PATCH project | Credential cache (BLAKE3) | gRPC-Web writes | | BrowserChannel query | BrowserChannel listen | | | | |
| Admin suspend/delete project | ECIES key derivation (direct_pg) | | | | | | | | |
| DB migrations on create | Rate limit check | | | | | | | | |

---

## Walking Skeleton

**Minimum slice** that delivers end-to-end value (proves the system works at all):

> gRPC server accepts a `GetDocument` call from the Firebase SDK → authenticates with Argon2id key check → routes to a direct_pg backend → executes a Postgres query → returns a Firestore-formatted response.

This slice proves: transport, auth, storage adapter, protocol encoding. Everything else extends one of these four layers.

---

## Slice Summary (15 slices)

| # | Name | Activity | Learning Hypothesis | Effort |
|---|------|----------|---------------------|--------|
| S01 | gRPC server + GetDocument + direct_pg | A2 + A4 + A8 | Disproves: "Firestore proto encoding is too complex to implement correctly" | ≤1 day |
| S02 | CreateDocument + UpdateDocument | A3 | Disproves: "Write OCC via version column is unreliable under concurrent writes" | ≤1 day |
| S03 | DeleteDocument + tombstones | A3 | Disproves: "Tombstone-based delta delivery is leaky (deletes not propagated)" | ≤1 day |
| S04 | RunQuery (simple filter) | A5 | Disproves: "SQL translation of Firestore filter operators is unsound" | ≤1 day |
| S05 | RunQuery (composite + ordering + indexes) | A5 | Disproves: "Index enforcement can be deferred without breaking query semantics" | ≤1 day |
| S06 | Listen initial snapshot | A6 | Disproves: "Bidirectional gRPC streaming is impractical in Rust with Tonic" | ≤1 day |
| S07 | Listen live changes (Postgres NOTIFY) | A6 | Disproves: "NOTIFY fan-out latency exceeds 2s for typical write rates" | ≤1 day |
| S08 | Listen resume token + delta delivery | A6 | Disproves: "Resume tokens after reconnect lead to duplicate or missed documents" | ≤1 day |
| S09 | BeginTransaction + Commit + Rollback | A7 | Disproves: "OCC transaction abort rate is too high to be usable by SDK" | ≤1 day |
| S10 | Admin API (create/get/patch/delete project, direct_pg) | A1 + A9 | Disproves: "Admin project provisioning with DB migrations is a multi-day integration" | ≤1 day |
| S11 | Project suspension + enforcement | A9 | Disproves: "SUSPENDED state check adds unacceptable latency per request" | ≤1 day |
| S12 | gRPC-Web + BrowserChannel transport | A3 + A4 + A6 | Disproves: "Browser transports require a separate server process" | ≤1 day |
| S13 | embyr agent (mTLS gRPC backend) | A8 | Disproves: "Agent mode TLS cert management is too operationally complex to ship" | ≤1 day |
| S14 | Cloud secret backends (AWS + GCP) | A8 | Disproves: "Cloud IAM credential fetch adds >500ms per request" | ≤1 day |
| S15 | Rate limiting + usage metrics | A10 | Disproves: "Per-project token-bucket rate limiting distorts tail latency" | ≤1 day |

---

## Prioritization rationale

See `prioritization.md` for full slice execution order.

High-uncertainty first:
- S07 (NOTIFY latency) and S08 (resume tokens) have most uncertainty — schedule early so failures cost less.
- S13 (agent mTLS) is novel infrastructure — test the cert management story while other slices are still in progress.

Dependency chain:
- S01 is the foundation; all other slices depend on it.
- S02 must precede S03 (tombstones depend on writes working).
- S04 must precede S05 (composite queries build on simple ones).
- S06 must precede S07 and S08 (live changes build on initial snapshot).
- S10 must precede S11, S13, S14 (project lifecycle before backends).

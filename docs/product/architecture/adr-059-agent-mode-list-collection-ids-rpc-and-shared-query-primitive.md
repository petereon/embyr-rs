# ADR-059: Agent-Mode `ListCollectionIds` — New Unary RPC on `storage_agent.proto`, Zero SQL Duplication (Reuses `PostgresBackendAdapter::list_collection_ids` Directly), `AgentBackendAdapter`'s Own Page-Flattening Loop

## Status

Accepted

## Numbering Note

Originally drafted as ADR-056; renumbered to ADR-059 after a collision was
found with two concurrently-running sibling DESIGN waves
(`agent-mode-field-transforms` claimed `adr-056`/`adr-057`,
`agent-mode-transaction-purge` claimed `adr-058`) — per this session's own
standing numbering rule. `adr-056-agent-mode-list-collection-ids-rpc-and-shared-query-primitive.md`
is left in place as a short pointer to this file, not deleted (tooling
available to this agent cannot delete files).

## Context

`firestore-list-rpcs` (ADR-051 § Decision 3) deferred agent-mode
`ListCollectionIds` because `proto/embyr/agent/v1/storage_agent.proto`
declared no such RPC at all — a hard proto-surface wall, not a
latency/threshold question. That ADR named its own rough shape for a future
follow-up: "one new unary RPC declaration ... mirroring
`RunAggregationQuery`'s own unary agent-side shape ... plus a
`PostgresBackendAdapter`-identical SQL handler inside the agent binary ...
and one new `AgentBackendAdapter::list_collection_ids` override proxying
it" — explicitly assuming the agent binary would need its OWN copy of the
`split_part`-based SQL.

This feature (`agent-mode-list-collection-ids`) builds that follow-up. DESIGN
re-verified ADR-051's own assumption against current ground truth and found
it does not hold: `crates/embyr-agent/Cargo.toml` already depends on
`embyr-pg-storage` (workspace dependency, unconditional — not a dev-only or
optional dependency), and `StorageAgentService` (`crates/embyr-agent/src/server.rs`)
already holds `storage: Arc<PostgresBackendAdapter>` as a field, used
directly by every existing handler (`get_document`, `run_query`,
`run_aggregation_query`, `list_documents`, ...). `PostgresBackendAdapter::list_collection_ids`
(`crates/embyr-pg-storage/src/backend_adapter.rs:844-895`) is the exact same
struct/method the non-agent path already calls. **No SQL duplication is
needed at all** — the agent's own new handler calls
`self.storage.list_collection_ids(...)` directly, the identical method,
identical `split_part`/`DISTINCT`/`LIMIT`/`OFFSET` query, identical
correctness guarantees (root/nested collapsing, sibling-parent scoping,
`DISTINCT`-by-construction). This is a genuine refinement of ADR-051's own
prediction, not a deviation from it — it confirms the DISCUSS-stage
hypothesis named in this feature's own commissioning brief ("this might be
closer to wiring a new RPC onto an already-existing query method than
writing new SQL") was correct, and materially shrinks the follow-up's actual
size versus ADR-051's own estimate.

The one genuinely new design problem ADR-051's rough sketch did not address:
`BackendAdapter::list_collection_ids`'s own driven-port signature is
`(parent: &CollectionPath, limit: i32, offset: i32) -> Result<Vec<String>, CoreError>`
— a single call, no page token. The NEW agent wire RPC, mirroring the
client-facing `ListCollectionIdsRequest`/`Response` shape (§ Decision 1),
is itself paginated (`page_size`/`page_token` → `next_page_token`), and the
agent's own handler defensively clamps `page_size` to ≤100 per call
(matching `list_documents`'s own existing clamp in the same file). A caller
can request `limit` values above 100 — including `i32::MAX`, the exact value
`handle_list_documents`'s own `collection_id`-empty fan-out already passes
today (`crates/embyr-server/src/grpc/handler.rs:1503`, unpaginated
"enumerate every child" call) for every `backend_mode` including `agent`.
`AgentBackendAdapter::list_collection_ids` must therefore flatten the agent's
own ≤100-per-call pagination into a single `Vec<String>` satisfying an
arbitrary `limit`, transparently to its own caller. This is the one
genuinely new mechanism this ADR designs (§ Decision 3).

## Decision 1 — Proto: New Unary RPC, Field Shape Mirrors the Client-Facing Messages Exactly

```protobuf
// storage_agent.proto — inserted immediately after ListDocuments, before Subscribe
// (unary-before-streaming, matching this file's own existing ordering).

service StorageAgent {
  // ... existing RPCs unchanged ...

  // Lists documents in a collection with pagination.
  rpc ListDocuments(ListDocumentsRequest) returns (ListDocumentsResponse);

  // Lists the distinct child collection IDs immediately under a parent document.
  rpc ListCollectionIds(ListCollectionIdsRequest) returns (ListCollectionIdsResponse);

  // Subscribes to real-time changes for a collection.
  rpc Subscribe(SubscribeRequest) returns (stream DocChange);
}

// A request to list the distinct child collection IDs under a parent.
message ListCollectionIdsRequest {
  string parent = 1;
  int32 page_size = 2;
  string page_token = 3;
}

// The response to ListCollectionIds.
message ListCollectionIdsResponse {
  repeated string collection_ids = 1;
  string next_page_token = 2;
}
```

Field numbers mirror `google/firestore/v1/firestore.proto`'s own
`ListCollectionIdsRequest`/`ListCollectionIdsResponse` exactly
(`parent=1, page_size=2, page_token=3` / `collection_ids=1,
next_page_token=2`) — the same field-numbering-mirroring discipline
ADR-050/051 already established for `ListDocuments`. `parent` here is a
DOCUMENT-or-root path (identical semantic to `ListDocumentsRequest.parent`),
not a collection path — a parent document's immediate child collections are
what this RPC enumerates, matching `docs/SPEC.md`'s own client-facing
contract.

## Decision 2 — Agent Handler: Zero New SQL, Direct Call Through the Existing `Arc<PostgresBackendAdapter>` Field

```rust
// crates/embyr-agent/src/server.rs — inside `impl StorageAgent for StorageAgentService`

async fn list_collection_ids(
    &self,
    request: Request<ListCollectionIdsRequest>,
) -> Result<Response<ListCollectionIdsResponse>, Status> {
    let req = request.into_inner();
    let project_id_str = parse_project_id_from_parent(&req.parent)?;
    let prefix = parent_prefix(&req.parent)?;
    let page_size = if req.page_size <= 0 { 100i32 } else { req.page_size.min(100) };
    let offset = decode_page_token(&req.page_token)?;
    let pid = ProjectId::new(&project_id_str)
        .map_err(|e| Status::invalid_argument(format!("{e}")))?;
    let parent = CollectionPath { project_id: pid, collection_path: prefix };

    // Same struct, same method, same SQL as the non-agent path
    // (ADR-051 § Decision 2) — self.storage IS a PostgresBackendAdapter.
    let mut ids = self
        .storage
        .list_collection_ids(&parent, page_size + 1, offset as i32)
        .await
        .map_err(core_error_to_status)?;

    let has_more = ids.len() > page_size as usize;
    if has_more {
        ids.truncate(page_size as usize);
    }
    let next_page_token = if has_more {
        encode_page_token(offset + page_size as u32)
    } else {
        String::new()
    };
    Ok(Response::new(ListCollectionIdsResponse { collection_ids: ids, next_page_token }))
}
```

Identical fetch-one-extra-to-detect-more-pages technique `list_documents`
(this same file) and `handle_list_collection_ids` (non-agent,
`crates/embyr-server/src/grpc/handler.rs`) both already use. Requires
`BackendAdapter` to be in scope for the trait method to resolve on the
concrete `PostgresBackendAdapter` type — already imported
(`crates/embyr-agent/src/server.rs:19`), zero new import needed for that.

**Small in-file refactor, not a new mechanism**: `parent_prefix(parent: &str)
-> Result<String, Status>` is extracted from the existing
`build_collection_path`'s own marker-finding logic (same file), which
today inlines "find `databases/(default)/documents`, take the suffix,
trim the leading slash" before appending `collection_id`. `build_collection_path`
becomes a two-line wrapper calling `parent_prefix` then appending
`collection_id` — behavior unchanged (verified: identical output for both
the root-empty-suffix and nested-non-empty-suffix branches), one duplicated
block removed. This mirrors `crates/embyr-server/src/grpc/handler.rs::parse_parent_prefix`'s
own shape (same problem, same file-local solution) without importing across
the `embyr-agent`/`embyr-server` binary boundary — the two binaries do not,
and per `firestore-list-rpcs`'s own precedent (declining to extract the
agent's page-token functions into a shared cross-binary module), should not,
share gRPC-handler-local parsing helpers through `embyr-core` (which cannot
depend on `tonic::Status` under the NO-IO constraint).

## Decision 3 — `AgentBackendAdapter`: Page-Flattening Loop Over the Agent's Own ≤100-Per-Call Pagination

The driven-port contract (`BackendAdapter::list_collection_ids`) is a single
call returning up to `limit` results starting at `offset` — no page-token
concept at that layer. The agent's own new RPC (§ Decision 1) caps each
response to ≤100 items (Decision 2's own defensive clamp, matching
`list_documents`'s existing precedent). Callers of the trait method pass
`limit` values both ≤101 (`handle_list_collection_ids`'s own
`page_size + 1`, client `page_size` already clamped ≤100) and `i32::MAX`
(`handle_list_documents`'s own unpaginated `collection_id`-empty fan-out,
`handler.rs:1503`, backend-mode-agnostic — this feature is the first time
`backend_mode=agent` can satisfy that call at all, since the trait's
default-error body previously rejected it, ADR-051 § Decision 3).
`AgentBackendAdapter::list_collection_ids` must transparently loop the
agent's own paginated RPC until either `limit` is satisfied or the agent
signals exhaustion (`next_page_token` empty) — the same pattern any
Firestore pagination consumer already applies client-side.

```rust
// crates/embyr-server/src/adapters/agent_backend.rs

/// Build the agent RPC's own `parent` string from a `CollectionPath` used as
/// a PARENT PREFIX (ADR-051's own semantic overload for this trait method,
/// documented there) — distinct from `domain_path_to_agent_parent`
/// (&DocumentPath), which never carries a path suffix.
fn domain_collection_prefix_to_agent_parent(collection: &CollectionPath) -> String {
    let base = format!(
        "projects/{}/databases/(default)/documents",
        collection.project_id.as_str()
    );
    if collection.collection_path.is_empty() {
        base
    } else {
        format!("{base}/{}", collection.collection_path)
    }
}

async fn list_collection_ids(
    &self,
    parent: &CollectionPath,
    limit: i32,
    offset: i32,
) -> Result<Vec<String>, CoreError> {
    use embyr_proto::agent::{ListCollectionIdsRequest, ListCollectionIdsResponse};

    let parent_str = domain_collection_prefix_to_agent_parent(parent);
    let mut collected: Vec<String> = Vec::new();
    let mut page_token = embyr_core::pagination::encode_page_token(offset.max(0) as u32);
    let mut client = self.client.clone();

    loop {
        let req = ListCollectionIdsRequest {
            parent: parent_str.clone(),
            page_size: 100, // agent's own per-call ceiling — see Decision 2
            page_token,
        };
        let resp: ListCollectionIdsResponse =
            client.list_collection_ids(req).await.map_err(grpc_err)?.into_inner();
        collected.extend(resp.collection_ids);

        if resp.next_page_token.is_empty() || collected.len() as i64 >= limit as i64 {
            break;
        }
        page_token = resp.next_page_token;
    }

    collected.truncate(limit.max(0) as usize);
    Ok(collected)
}
```

**Correctness under the exact boundary the client-facing path exercises**
(worked through explicitly, not assumed): client `page_size=100`, 101 total
collections exist, `offset=0`. Outer `handle_list_collection_ids` calls
`list_collection_ids(&parent, 101, 0)`. Loop iteration 1: agent call
requests `page_size=100`; the agent's own Decision-2 handler internally
queries `page_size+1=101` rows from Postgres, finds all 101, sets its own
`has_more = 101 > 100 = true`, truncates its response to 100 ids, returns a
non-empty `next_page_token`. `collected.len()=100 < 101` and
`next_page_token` non-empty → loop continues. Iteration 2: requests
`page_size=100` at the new offset; only 1 row remains, agent's own
`has_more = 1 > 100 = false`, returns 1 id + empty `next_page_token`.
`collected.len()=101 >= limit(101)` → loop exits (also `next_page_token`
now empty — both conditions independently true). `truncate(101)` is a
no-op. `AgentBackendAdapter` returns a 101-element `Vec<String>` to the
outer caller, which compares `101 > page_size(100)` → `has_more=true`,
truncates to 100, and emits the correct `next_page_token` — byte-identical
outer behavior to the non-agent path with the same data shape. Two agent
round-trips in this specific boundary case (101 collections, 100-page
request) — a bounded, small cost accepted for correctness (§ Alternatives,
Option B).

`offset` is forwarded as the FIRST page's `page_token` directly (the
hex-offset scheme, `embyr_core::pagination::encode_page_token`, is an
absolute offset, not a page number — already true of every existing caller
of this scheme); every SUBSEQUENT request in the loop uses the agent's own
returned `next_page_token` verbatim, never recomputed — no drift risk
between `AgentBackendAdapter`'s own offset arithmetic and the agent's.

## Alternatives Considered

**A. Forward `limit` directly as the wire request's `page_size`, single
round trip, no loop.** Rejected: demonstrated incorrect at the exact
boundary worked through in § Decision 3 — when `limit > 100` (which happens
on every client request with `page_size=100`, since `handle_list_collection_ids`
always passes `page_size + 1 = 101`), the agent's own defensive
`page_size.min(100)` clamp (Decision 2, matching `list_documents`'s existing
precedent) would silently cap the response at 100 items even when 101+ rows
exist, producing `has_more=false` when the true answer is `true` — a real,
silent pagination-correctness bug at the single most common page size
(100, the default `page_size.min(100)` value used everywhere in this
codebase's own list RPCs). Removing the agent-side clamp instead of looping
was also rejected: it would special-case this one RPC's defensive ceiling
inconsistently with `list_documents`'s own identical clamp in the same file,
for no benefit — the loop is the honest fix, not a workaround.

**B. Cap the loop at a fixed small iteration count (e.g., 2) instead of
looping until `next_page_token` is empty or `limit` is reached.** Rejected:
a hard iteration cap would silently truncate results below the true `limit`
for a pathological but legitimate case (a single parent document with
>200 subcollections and a caller requesting a large `limit`, e.g. the
`collection_id`-empty `ListDocuments` fan-out's own `i32::MAX` call) —
reintroducing exactly the kind of silent data loss this feature's own AC-03
(pagination completeness) exists to prevent. The unbounded loop is safe: it
terminates deterministically because each iteration either exhausts the
agent's own `next_page_token` (finite result set) or brings `collected.len()`
to `limit` (finite bound) — never both false simultaneously across an
infinite Postgres result set, since `list_collection_ids`'s own SQL query is
itself `LIMIT`-bounded per call.

**C. Duplicate `PostgresBackendAdapter`-identical SQL inside the agent
binary, per ADR-051's own original rough sketch.** Rejected once ground
truth (§ Context) confirmed the agent binary already holds the exact struct
this SQL lives on — duplicating it would be pure unforced code duplication
with a real drift risk (two copies of a `split_part`/`substring`-based query
diverging over time), against this codebase's own established
reuse-over-duplication discipline (ADR-041, ADR-050, ADR-051 § Alternatives
B all reject unforced duplication on the same grounds).

## Consequences

**Positive**: zero new SQL, zero new domain type, zero new `CoreError`
variant (the agent's own handler reuses `core_error_to_status` unchanged;
`AgentBackendAdapter`'s own loop reuses `grpc_err` unchanged). One new proto
RPC + message pair, mirroring the client-facing shape field-for-field. One
small in-file refactor (`parent_prefix` extracted from `build_collection_path`,
behavior-preserving). One genuinely new mechanism
(`AgentBackendAdapter`'s page-flattening loop), verified correct at the
exact 100/101 boundary the client-facing path already exercises today.
`AC-01` through `AC-04` (§ this feature's own US-01) are all satisfied: the
agent's handler returns the SAME query result as the non-agent path
(correctness/scoping identical by construction, same SQL); pagination
correctness holds across the round-trip boundary (worked through above).

**Positive, unplanned**: `handle_list_documents`'s own `collection_id`-empty
fan-out (`handler.rs:1497-1512`, backend-mode-agnostic, pre-existing) now
also works correctly for `backend_mode=agent` for the first time — it was
silently rejecting with `FailedPrecondition` before this feature (inherited
from `BackendAdapter::list_collection_ids`'s own default-error body,
ADR-051 § Decision 1/3), since it calls the identical trait method this ADR
now gives `AgentBackendAdapter` a real override for. **Not itself covered by
a UAT scenario in this feature's own US-01** (which targets
`ListCollectionIds` directly, not `ListDocuments`'s internal fan-out) —
named here as a residual capability gain worth a follow-up regression test
if `ListDocuments`'s own `collection_id`-empty case for `backend_mode=agent`
becomes customer-visible traffic, not a DESIGN-time blocker.

**Negative, named explicitly**: `AgentBackendAdapter::list_collection_ids`
can require multiple mTLS round trips for a single trait-level call when a
parent has more than 100 immediate children and the caller's own `limit`
exceeds 100 — a real, bounded latency cost (worst case:
`ceil(min(limit, actual_child_count) / 100)` round trips) accepted over
Alternative A's silent correctness bug. No numeric latency target is set for
this feature, matching this session's own established precedent
(`firestore-list-rpcs`'s own Quality Validation, same reasoning).

**Residual, non-blocking**: the cross-version graceful-degradation question
(an old `embyr-agent` binary predating this RPC returns a clean gRPC
`Unimplemented` for `ListCollectionIds`) is NOT re-derived here — it is
being resolved once, across all wire-touching sibling features, by
`agent-mode-write-streaming`'s own concurrent DESIGN wave. This feature's own
failure mode under that scenario is identical and already clean by
construction: `AgentBackendAdapter::list_collection_ids`'s `grpc_err`
mapping turns any `tonic::Status` — including `Unimplemented` from an
old agent — into `CoreError::BackendUnavailable`, itself mapped to
`Status::internal` by `core_error_to_status` on the SaaS-facing side. This
feature inherits whatever version-skew UX conclusion the sibling feature
reaches; no code in this ADR needs to change regardless of that outcome,
since the failure is already structured and non-panicking.

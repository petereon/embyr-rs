# ADR-068: Composite Index Admin CRUD

## Status

Accepted

## Context

`firestore-composite-indexes-admin-api` (JOB-01, realization closing a documented-but-unbuilt gap)
gives Alex a real way to satisfy the composite-index gate `RunQuery` has enforced, correctly,
since the original walking skeleton (slice 04-02): a query filtering on one field and ordering by
a different one requires a READY composite index, checked via `IndexManager::is_index_ready`
against the `composite_indexes` table (migration 0003). That table and its gate have existed,
correctly wired, since that original slice — but no production code path has EVER written a row
into it. `composite_indexes` starts empty for every project and stays empty forever; the resulting
`FAILED_PRECONDITION` is permanent, unlike real Firestore's own equivalent (which gives the
developer a console link to actually create the missing index).

DISCUSS (`docs/feature/firestore-composite-indexes-admin-api/feature-delta.md`, Resolutions 1–5)
locked: reuse JOB-01 (mirrors the `aggregation-queries`/`batch-get-documents` "make it real"
pattern), synchronous `status: 'ready'` on create (no async `CREATING` simulation — this backend
has no real secondary-index build process to model), metadata-only (no real Postgres `CREATE
INDEX` DDL — query correctness never depended on one; only performance would, and no domain
example currently shows that as a problem), no dependency-safety check on delete, and idempotent
create on an exact-duplicate spec.

## Decision Drivers

1. **Zero change to the query path** (`crates/embyr-server/src/grpc/handler.rs`) — this feature is
   purely upstream: it populates a table `IndexManager`/`requires_composite_index` already read,
   unmodified.
2. **Reuse this codebase's own already-established admin-handler conventions exactly** —
   `define_access_rule`'s auth/ownership shape, `service_accounts`' CRUD SQL shape, `upsert_
   access_rule`'s `ON CONFLICT ... DO UPDATE` idempotency shape — never new, parallel machinery for
   a structurally identical problem.
3. **Reuse the `fields` JSONB shape already assumed elsewhere in this codebase**
   (`tests/acceptance/us_04_query_collection.rs`'s own fixture:
   `[{"field": "category", "order": "ASC"}, ...]`) — not a newly-invented convention.
4. **Simplest solution first**: no new `IndexManager` method, no new adapter/port trait, no new
   bounded context — 3 handlers in one new file, 2 new router entries.

## Decision — Types (`admin/handlers/composite_indexes.rs`, NEW file)

```rust
/// One field/order pair within a composite index — matches the `fields`
/// JSONB shape already assumed by `tests/acceptance/us_04_query_
/// collection.rs`'s own fixture (`[{"field": "category", "order":
/// "ASC"}, ...]`), never a newly-invented convention.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexFieldSpec {
    pub field: String,
    pub order: IndexFieldOrder,
}

/// Serializes to/from the exact `"ASC"`/`"DESC"` strings the existing
/// fixture already uses — a typed enum, not a raw string, so a malformed
/// `order` value is a clean 422 at the request-deserialization boundary,
/// never a value silently stored that the query path would later fail to
/// compare against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IndexFieldOrder {
    Asc,
    Desc,
}

/// Body for POST /admin/v1/projects/:project_id/indexes.
#[derive(Debug, Deserialize)]
pub struct CreateCompositeIndexBody {
    pub collection_path: String,
    pub fields: Vec<IndexFieldSpec>,
}

/// Response shape for create/list — one composite index, real Firestore
/// -adjacent field naming (`id`, `status`) without transliterating real
/// Firestore's own `google.firestore.admin.v1.Index` protobuf, matching
/// this codebase's own established "own JSON conventions, not a
/// transliterated Firestore Admin proto" precedent (confirmed by DISCUSS
/// Reading Confirmation — `access_rules`/`write_access_rules` never mirror
/// a real Firestore proto 1:1 either).
#[derive(Debug, Serialize)]
pub struct CompositeIndexResponse {
    pub id: String,
    pub project_id: String,
    pub collection_path: String,
    pub fields: Vec<IndexFieldSpec>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

## Decision — Handlers

```rust
pub async fn create_composite_index(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateCompositeIndexBody>,
) -> Result<(StatusCode, Json<CompositeIndexResponse>), StatusCode>;

pub async fn list_composite_indexes(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<CompositeIndexResponse>>, StatusCode>;

pub async fn delete_composite_index(
    Path((project_id, index_id)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode>;
```

`create`/`delete` gate `session.role < Role::Admin` (mirrors `define_access_rule`); `list` is any
role, read-only (mirrors `get_access_rule_history`). All 3 call `verify_project_ownership(pool,
&project_id, session.account_id).await?` before touching `composite_indexes` — identical to
`define_access_rule`'s own shape, never a new ownership-check helper.

## Decision — SQL

**Create** (Resolution 5, idempotent):
```sql
INSERT INTO composite_indexes (project_id, collection_path, fields, status)
VALUES ($1, $2, $3::jsonb, 'ready')
ON CONFLICT (project_id, collection_path, fields)
DO UPDATE SET collection_path = EXCLUDED.collection_path
RETURNING id::text AS id, project_id, collection_path, fields, status, created_at
```
The `DO UPDATE SET collection_path = EXCLUDED.collection_path` clause is a deliberate no-op
self-update (PostgreSQL's `ON CONFLICT` requires a non-empty `DO UPDATE` to make `RETURNING` fire
on the conflicting row too, since a bare `DO NOTHING` returns zero rows on conflict) — mirrors
`upsert_access_rule`'s own identical `ON CONFLICT (...) DO UPDATE` shape, just applied to a
column that never actually changes value, since `EXCLUDED.collection_path` always equals the
conflicting row's own existing value by construction (it's part of the conflict key itself).

**List**: `SELECT id::text AS id, project_id, collection_path, fields, status, created_at FROM
composite_indexes WHERE project_id = $1 ORDER BY created_at ASC` — mirrors `list_service_accounts`
exactly, project-scoped instead of account-scoped.

**Delete**: `DELETE FROM composite_indexes WHERE id = $1 AND project_id = $2` — the `project_id`
clause in the `WHERE` doubles as the ownership check (a caller who owns `project_id` per `verify_
project_ownership` cannot delete another project's own row even if they somehow guessed its
`id`); 0 rows affected → `StatusCode::NOT_FOUND`.

## Decision — Router (`admin/router.rs`, EXTEND)

```rust
.route(
    "/admin/v1/projects/:project_id/indexes",
    get(list_composite_indexes).post(create_composite_index),
)
.route(
    "/admin/v1/projects/:project_id/indexes/:index_id",
    delete(delete_composite_index),
)
```

## Enforcement

Unit tests for `IndexFieldOrder`'s own (de)serialization round-trip (the one piece of genuinely
new pure logic this feature adds) are written during Slice 01. A `cargo-mutants` pass is still
budgeted at QUALITY_GATE regardless, per this session's own established discipline — even a
small, mostly-I/O-shaped CRUD feature can hide a narrow boundary gap.

## Consequences

**Positive**:
- Zero change to the query path — confirmed by construction, this feature is purely upstream.
- Reuses 3 already-proven-many-times-over conventions (auth/ownership, CRUD SQL shape, `ON
  CONFLICT` idempotency) rather than inventing new ones — the lowest-risk admin feature built this
  session.
- The `fields` JSONB shape is byte-for-byte compatible with the ONE place this table was ever
  written to before this feature (the test fixture in `us_04_query_collection.rs`), so no
  migration or compatibility shim is needed for that existing test.

**Negative / accepted trade-offs**:
- No real Postgres index is provisioned — a composite query gated "ready" by this feature may
  still be slower than an equivalent query with a real secondary B-tree index, purely a
  performance (not correctness) trade-off, named and deferred (Resolution 3).
- `is_index_ready`'s own pre-existing per-collection (not per-field-set) granularity means a
  READY index for the wrong field combination still unblocks an unrelated query needing a
  DIFFERENT combination for the same collection — a pre-existing behavior this feature does not
  change or worsen, named explicitly in DISCUSS so it is never mistaken for a regression this
  feature introduces.

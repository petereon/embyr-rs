# ADR-051: `ListCollectionIds` — New `BackendAdapter::list_collection_ids` Trait Method, `split_part`-Based SQL, Agent-Mode Deferred (Resolves DISCUSS's Named-Not-Escalated Query-Primitive and Agent-Mode Questions)

## Status

Accepted

## Context

DISCUSS confirmed no existing trait method or SQL query in this codebase
enumerates "distinct immediate children of a path prefix" (§ Reading
Confirmation, re-verified directly in ADR-050's own Verification). It
recommended a new `BackendAdapter` trait method with a default-error body
mirroring `run_aggregation_query`'s ADR-041 precedent, left the exact
signature and SQL to DESIGN, and named agent-mode `ListCollectionIds` as a
hard proto-surface wall (`StorageAgent`'s proto has no such RPC at all) to be
deferred, mirroring `Write`'s own ADR-047 precedent — but did not itself write
an ADR confirming that mirroring holds under DESIGN-level scrutiny.

## Decision 1 — Trait Method Signature

```rust
/// Returns the distinct names of collections immediately under `parent`
/// (one path segment deeper than `parent.collection_path`; empty
/// `collection_path` = database root). Reuses `CollectionPath`'s existing
/// (project_id, path-string) shape for a PARENT PREFIX, a distinct semantic
/// from its other call sites (an exact collection name, or — with
/// `all_descendants` — a collection-group name); documented here to avoid
/// confusion for a future reader.
///
/// Default-provided body (ADR-041/ADR-051 precedent): implementors without
/// real support (`AgentBackendAdapter`, deferred — § Decision 3) compile
/// unmodified and reject at runtime via the existing `FailedPrecondition`
/// variant — never a new `CoreError` variant (`crates/embyr-agent`'s own
/// exhaustive `core_error_to_status` match has no wildcard arm).
async fn list_collection_ids(
    &self,
    parent: &CollectionPath,
    limit: i32,
    offset: i32,
) -> Result<Vec<String>, CoreError> {
    Err(CoreError::FailedPrecondition(
        "distinct collection listing is not supported by this backend".into(),
    ))
}
```

Added to `crates/embyr-core/src/storage/backend_adapter.rs`, alongside
`run_aggregation_query`'s own identically-shaped default body. Reuses
`CollectionPath` rather than introducing a new domain type for "a document
path prefix" — the existing type already carries exactly the two fields
needed (`project_id`, a path string); a new type for a single call site would
be an unrequested abstraction. The semantic overload (prefix vs. exact
name/group name elsewhere) is called out in the doc comment, the same
mitigation this codebase already uses for `all_descendants`' own dual meaning
of `collection_path`.

`limit`/`offset` are plain `i32`, matching `StructuredQuery.limit`/`.offset`'s
existing types — no new pagination type introduced.

## Decision 2 — `PostgresBackendAdapter` SQL Shape

Single technique for both root and nested `parent`, avoiding a root/nested SQL
branch by exploiting `split_part`'s own behavior: `split_part(s, '/', 1)`
always returns the first segment of `s` regardless of how many further
segments follow, which is exactly "collapse any deeper nesting down to the
immediate child" — so DISTINCT naturally de-duplicates grandchildren into
their immediate parent's name without a separate `NOT LIKE 'prefix/%/%'`
exclusion (directly answers Slice 02's own Learning Hypothesis: "cannot be
built without either double-counting nested grandchildren or missing
root-level collections" — `split_part` + `DISTINCT` avoids both failure modes
by construction, not by a second guard clause).

```rust
async fn list_collection_ids(
    &self,
    parent: &CollectionPath,
    limit: i32,
    offset: i32,
) -> Result<Vec<String>, CoreError> {
    use sqlx::QueryBuilder;
    let prefix = &parent.collection_path;

    let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new("SELECT DISTINCT ");
    if prefix.is_empty() {
        // Root: the first segment of collection_path IS the top-level
        // collection name, for every row regardless of nesting depth.
        qb.push("split_part(collection_path, '/', 1)");
    } else {
        // Nested: strip "{prefix}/" then take the first remaining segment.
        // char_length(prefix) + 2 = 1-indexed start, skipping prefix + '/'.
        qb.push("split_part(substring(collection_path FROM ");
        qb.push_bind(prefix.chars().count() as i32 + 2);
        qb.push("), '/', 1)");
    }
    qb.push(" AS child_id FROM documents WHERE project_id = ");
    qb.push_bind(parent.project_id.as_str());
    qb.push(" AND NOT deleted");
    if !prefix.is_empty() {
        // Required guard: without it, rows whose collection_path does NOT
        // start with "{prefix}/" would have `substring` compute nonsense
        // (or an out-of-range start), and would incorrectly appear as
        // spurious children of an unrelated parent.
        qb.push(" AND collection_path LIKE ");
        qb.push_bind(format!("{prefix}/%"));
    }
    qb.push(" ORDER BY child_id LIMIT ");
    qb.push_bind(limit as i64);
    qb.push(" OFFSET ");
    qb.push_bind(offset as i64);

    let rows = qb
        .build()
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        ids.push(
            row.try_get("child_id")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
        );
    }
    Ok(ids)
}
```

Matches `run_query`'s own established style exactly (`QueryBuilder`,
`try_get`/`BackendUnavailable` mapping, `NOT deleted` filter, `LIMIT` before
`OFFSET`). Root case needs no `LIKE` guard — every row has at least one
segment, so `split_part(..., '/', 1)` is always well-defined. `AC-02-03`
("root-level parent returns only top-level collections, never a nested one")
holds because `split_part` collapses depth, not because deeper rows are
excluded — a nested collection's own top ancestor IS what a root-level call
should surface for it, and no row's `collection_path` other than
its true top-level ancestor is ever produced. `AC-02-05` ("each distinct name
exactly once regardless of document count") holds by `DISTINCT` construction,
unchanged from `run_query`'s own existing `DISTINCT`-free row-per-document
default — this is the one query in the codebase that needs `DISTINCT` at all,
because it is the one query answering a "what names exist" question rather
than a "what documents match" question.

## Decision 3 — Agent-Mode `ListCollectionIds` Deferred (v1)

Confirmed, not re-litigated: `proto/embyr/agent/v1/storage_agent.proto` (372
lines, read in full per DISCUSS's own Reading Confirmation) declares no
`ListCollectionIds` RPC, no request/response messages, zero references. This
is the same hard-wall shape as `Write`'s own original agent-mode gap
(ADR-047) — zero existing RPC to proxy through, not a latency/threshold
question like `BatchWrite`'s (ADR-049). **Decision: defer, matching ADR-047's
own precedent exactly.** `AgentBackendAdapter` does not override
`list_collection_ids` — it inherits this ADR's own default-error body (§
Decision 1) unmodified, compiles with zero changes to
`crates/embyr-server/src/adapters/agent_backend.rs`, and rejects at runtime
with `CoreError::FailedPrecondition("distinct collection listing is not
supported by this backend")`, mapped by the existing shared
`core_error_to_status` to `Status::failed_precondition` — no new local
error-mapping function is added (unlike ADR-041's own
`aggregation_error_to_status`), because `docs/SPEC.md` documents no
operator-specific `Unimplemented` contract for `ListCollectionIds` the way it
does for unsupported aggregation operators; `failed_precondition` is an
honest, already-used-elsewhere status for "this backend does not support this
capability," and inventing a distinct status code for this case with no
SPEC-documented requirement would be an unevidenced addition.

Unlike `Write`'s own still-open follow-up (ADR-047, a genuinely large lift —
new bidirectional-streaming mechanism class), a future `ListCollectionIds`
agent-mode follow-up is comparatively small: one new unary RPC declaration on
`storage_agent.proto` (mirroring `RunAggregationQuery`'s own unary agent-side
shape, ADR-041 precedent) plus a `PostgresBackendAdapter`-identical SQL
handler inside the agent binary (§ Decision 2's own query, unchanged) and one
new `AgentBackendAdapter::list_collection_ids` override proxying it. Named
here as the follow-up's own rough shape, not built now — zero customer
evidence motivates building it ahead of `direct_pg`/`aws_secret`/`gcp_secret`
usage data, matching ADR-049's own "no unevidenced work" discipline.

## Alternatives Considered

**A. Root/nested SQL as two structurally different queries (branch on a
`NOT LIKE 'prefix/%/%'` exclusion for the nested case instead of relying on
`split_part`'s own segment-collapsing).** Rejected: works, but requires an
extra `NOT LIKE` predicate whose correctness depends on getting the escaping
of `prefix` right a second time; `split_part` achieves the same exclusion by
construction, with less SQL and one fewer bound parameter to get wrong.

**B. A new domain type (e.g. `PathPrefix`) instead of reusing `CollectionPath`
for `parent`.** Rejected: `CollectionPath`'s existing two fields already carry
exactly what's needed (project scope + a path string); introducing a
single-purpose wrapper type for one trait method's one parameter is an
unrequested abstraction this codebase's own established practice (ADR-041 §
Decision Drivers 3, reuse over invention) argues against. The semantic
overload is a real but small cost, mitigated by a doc comment, not a new type.

**C. Build agent-mode `ListCollectionIds` now, authoring new agent-binary
proto surface (mirroring `Write`'s Alternative A, ADR-047).** Rejected for the
same walking-skeleton-discipline reason ADR-047's Alternative A was rejected:
this feature's own scope (2 slices, ~2.5 days) would grow to include new
agent-binary proto authoring and a second `StorageAgentService` RPC
implementation for a segment (JOB-04/JOB-09 credential-isolation customers)
with no evidence this specific RPC is even commonly called by that segment
(unlike `ListDocuments`, `ListCollectionIds` has no dedicated agent-side RPC
today at all — building one from nothing, speculatively, is a larger
unforced bet than extending an RPC that already exists on both sides).

## Consequences

**Positive**: one new trait method, one new `PostgresBackendAdapter`
implementation, zero new `CoreError` variant, zero new domain type, zero
`crates/embyr-agent/` changes. `AC-02-01` through `AC-02-06` are all satisfied
by a single `SELECT DISTINCT ... split_part(...) ... LIMIT/OFFSET` query,
reusing `run_query`'s own established `QueryBuilder` idiom exactly.

**Negative, named explicitly**: `backend_mode=agent` customers cannot call
`ListCollectionIds` in v1 — a real, customer-visible capability gap for
JOB-04/JOB-09's credential-isolation segment, on top of `Write`'s own
still-open gap (ADR-047). Two of this session's RPC-completion features now
leave that segment with a growing, but individually small and individually
named, list of unserved capabilities — flagged here as a pattern worth a
dedicated cross-feature follow-up (a single `agent-mode-catch-up` feature
covering `Write` streaming AND `ListCollectionIds`, sized together) rather
than as a new concern unique to this ADR.

**Residual, non-blocking**: no index exists on `documents.collection_path`
today (unconfirmed by this ADR — not verified against the migration's own
index list); a `split_part`/`substring` expression in the `WHERE`/`SELECT
DISTINCT` clause is not sargable against a plain B-tree index on
`collection_path` even if one exists, so this query performs a sequential
scan filtered by `project_id` (and, for the nested case, a `LIKE 'prefix/%'`
prefix match, which IS sargable against a `collection_path` index if present).
Acceptable for this feature's own scope (matches `run_query`'s own existing
lack of a dedicated collection_path index, same table, same risk profile,
unchanged by this feature) — named as a DEVOPS/production-readiness indexing
candidate if per-project document volume ever makes this a measured hotspot,
not a DESIGN-time blocker.

# ADR-010: SessionContext Axum Extractor for Account-Scoping

## Status

Accepted

## Context

DISCUSS locked decision D3: "All session-auth queries must include `WHERE account_id = $session_account`. This is enforced in the handler, not in a shared middleware, to keep the join simple. Downstream risk: a missing WHERE clause leaks cross-account data — mitigated by integration tests that verify cross-account 403."

Every session-auth handler needs three pieces of information derived from a validated session:
1. `account_id: Uuid` — to scope all queries with `WHERE account_id = $account_id`
2. `user_id: Uuid` — for audit and self-reference (cannot demote self, cannot remove self as last owner)
3. `role: Role` — for RBAC checks (`Owner > Admin > Viewer`)

The question is the mechanism: how does this information reach the handler in a way that makes "forgetting to check account_id" structurally impossible?

Three options were analyzed:
1. **Middleware sets request extension** — middleware extracts and validates the session, then inserts `SessionContext` as a `Request::extensions` entry. Handlers access it via `Extension<SessionContext>`.
2. **Axum `FromRequestParts` extractor** — handlers declare `session: SessionContext` as a parameter. Axum calls the extractor before the handler body executes. If the session is invalid, the extractor returns a 401 response and the handler body never runs.
3. **Explicit parameter threading** — session info is extracted once in a shared function called at the top of each handler body.

## Decision

**`SessionContext` as an Axum `FromRequestParts` extractor (Option 2).**

`SessionContext` is a struct defined in `embyr-core::admin::session`:
```
SessionContext {
    user_id: Uuid,
    account_id: Uuid,
    role: Role,
}
```

`Role` is a domain enum in `embyr-core::admin::account`:
```
enum Role { Owner = 3, Admin = 2, Viewer = 1 }
```

The extractor implementation (in `embyr-server::admin::extractors::session_context`) does the following on every invocation:
1. Try to extract `embyr_session` cookie from the `Cookie` header. If present:
   a. BLAKE3-hash the raw cookie value → `token_hash`
   b. Query `sessions` table: `SELECT user_id, account_id, role, last_active_at, expires_at FROM sessions WHERE token_hash = $1 AND expires_at > now()`
   c. If found and idle < 24h: return `SessionContext { user_id, account_id, role }`
   d. Update `sessions.last_active_at = now()` (fire-and-forget spawn)
2. Else, try to extract Bearer token from `Authorization` header. If it starts with `embyr_adm_`:
   a. BLAKE3-hash the raw token → `key_hash`
   b. Query `admin_api_keys`: `SELECT user_id_or_sa_id, account_id, role, revoked_at FROM admin_api_keys WHERE key_hash = $1 AND revoked_at IS NULL`
   c. If found: return `SessionContext` with role from the key record
   d. Update `admin_api_keys.last_used_at = now()` (fire-and-forget spawn)
3. If neither credential is present or valid: return HTTP 401 `{"message": "Unauthenticated"}`

The extractor is implemented as `FromRequestParts` (not `FromRequest`) because it only reads headers and cookies, not the request body.

**Handler signature pattern:**
```rust
// Every session-auth handler begins with:
async fn list_projects(
    session: SessionContext,           // ← extractor; 401 if invalid
    State(state): State<UserAdminState>,
    // ... other extractors
) -> ApiResult<Json<Vec<ProjectSummary>>> {
    let rows = sqlx::query!(
        "SELECT ... FROM projects WHERE account_id = $1 AND status != 'deleted'",
        session.account_id  // ← account_id is always present; cannot be omitted
    )
    ...
}
```

**Why D3 says "enforced in the handler, not shared middleware":** The extractor pattern satisfies D3's intent. The extractor validates and delivers `SessionContext`; the handler applies `WHERE account_id = $session.account_id`. The account-scoping predicate is explicit in the handler's SQL — it is not hidden in a middleware layer, making it auditable and review-discoverable.

**Database dependencies:** The extractor requires access to `SystemDb`. It receives the db pool via the `FromRequestParts` implementation accessing the Axum state extension (`UserAdminState`). The state must be set on the session sub-router before the extractor can run.

## Alternatives Considered

### Option 1: Middleware sets request extension (rejected)

The `session_auth_middleware` (Tower layer on the session sub-router) validates the session and sets `session_ctx` as a request extension. Handlers access it via:
```rust
async fn list_projects(
    Extension(session): Extension<SessionContext>,
    State(state): State<UserAdminState>,
) -> ApiResult<...> { ... }
```

**Rejected because:**
- If a handler forgets to declare `Extension<SessionContext>`, it compiles and runs — but has no `account_id` to scope its queries. This is the failure mode D3 identifies: "a missing WHERE clause leaks cross-account data."
- `Extension<SessionContext>` would panic at runtime with "missing extension" rather than failing at compile time, shifting the error from development to production.
- The session auth middleware is already responsible for returning 401; having it also populate an extension couples two concerns in one middleware (auth validation + context population).

### Option 3: Explicit parameter threading (rejected)

```rust
async fn list_projects(...) -> ApiResult<...> {
    let ctx = validate_session(&headers, &state.system_db).await?;
    let rows = sqlx::query!(..., ctx.account_id)...
}
```

**Rejected because:**
- Identical to Option 1's failure mode: forgetting to call `validate_session` compiles without error.
- Duplication of the validation call across every handler body is boilerplate that obscures handler intent.
- Harder to test: testing the handler requires mocking or substituting the validation call site.

## Consequences

**Positive:**
- Account-scoping is structurally enforced at the Rust type level: `account_id` is only available through a valid `SessionContext`, which is only available after the extractor succeeds.
- Handlers are self-documenting: the presence of `session: SessionContext` in the parameter list signals "this route requires session auth."
- Testability: the extractor can be tested independently of handlers. Handlers can be tested by providing a pre-constructed `SessionContext` (via a stub implementation or Axum test utilities that insert extensions directly).
- `last_active_at` update is a fire-and-forget spawn — zero impact on handler latency.

**Negative / Trade-offs:**
- Every DB access via the extractor adds one SELECT query per request (the session lookup). At p99 ≤ 100ms target for `GET /admin/v1/projects` (AC-B02-05), the session lookup must be fast. The `sessions.token_hash` column requires a unique index (hash value lookup). Index must be confirmed in the migration.
- The extractor holds a reference to `UserAdminState` (specifically `system_db`). Axum's `FromRequestParts` trait implementation must access the state extension, which requires the state to be set on the router before merging. This is a composition-root constraint, documented in the startup sequence.

## Enforcement

- **Compile-time**: `SessionContext` is only constructable by the extractor implementation. Handlers cannot construct it from scratch without the DB lookup — it is not `#[derive(Default)]` and has no `pub` constructor.
- **Integration test (cross-account 403)**: CI includes a test that signs in as Account A, then calls `GET /admin/v1/projects/:id` with the Account A session against a project belonging to Account B. Must return 403. This is the primary guard against D3's "missing WHERE clause" risk.

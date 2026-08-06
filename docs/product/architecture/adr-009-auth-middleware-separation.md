# ADR-009: Auth Middleware Separation — Two Auth Stacks on One Admin Port

## Status

Accepted

## Context

The admin port (:9090) serves two fundamentally different principal types on the same TCP listener:

1. **Operator (Sam)** — authenticates with a static `EMBYR_ADMIN_KEY` Bearer token. Uses the 5 existing routes (`POST /admin/v1/projects`, `GET /admin/v1/projects/:id`, `POST .../suspend`, `POST .../activate`, `DELETE .../projects/:id`). These routes must not be broken by any change.

2. **User-Admin (Chris)** — authenticates with either:
   - An HTTP-only session cookie (`embyr_session=<token>`) issued by `POST /admin/v1/auth/signin`
   - A programmatic admin API key Bearer token (`embyr_adm_<32chars>`) stored as BLAKE3 hash in `admin_api_keys` (D6 from DISCUSS locked decisions)

The existing handlers inline `extract_bearer(&headers)` and compare against `state.admin_key` directly — a pattern that cannot be extended to accommodate session cookies without modifying every handler.

DISCUSS locked decision D2: "Routes tagged `#[operator_only]` vs `#[session_auth]` are cleanly separated at the router level. No middleware collision possible — Axum route matching is exhaustive."

There is one genuinely dual-access route: `GET /admin/v1/projects/:id`. Sam (operator) uses it unchanged; Chris (session auth) also uses it for the Database Detail view. Axum cannot register the same (method, path) pair twice in a merged router.

## Decision

**Sub-router merge with per-sub-router Tower layers (Option A).**

Four Axum routers are built independently, each with its own auth enforcement, then merged at the composition root:

```
admin_router =
    operator_router   (layer: operator_auth_middleware — Bearer EMBYR_ADMIN_KEY only)
    .merge(session_router)   (layer: session_auth_middleware — cookie OR admin_api_key Bearer)
    .merge(public_router)    (no auth — signin, OIDC callback)
    .merge(dual_auth_router) (layer: dual_auth_middleware — either principal; sets AuthPrincipal extension)
```

**Operator sub-router** (operator_auth_middleware layer — accepts only Bearer EMBYR_ADMIN_KEY):
- `POST /admin/v1/projects` — provision project
- `POST /admin/v1/projects/:project_id/suspend`
- `POST /admin/v1/projects/:project_id/activate`
- `DELETE /admin/v1/projects/:project_id`

**Session sub-router** (session_auth_middleware layer — accepts cookie OR admin_api_key Bearer, sets SessionContext extension):
- `POST /admin/v1/auth/signout`
- `GET /admin/v1/projects` — list (account-scoped)
- `PATCH /admin/v1/projects/:project_id`
- `GET /admin/v1/projects/:project_id/sdk_keys`
- `POST /admin/v1/projects/:project_id/sdk_keys`
- `DELETE /admin/v1/projects/:project_id/sdk_keys/:key_id`
- `GET /admin/v1/projects/:project_id/metrics`
- `GET /admin/v1/projects/:project_id/query_logs`
- `GET /admin/v1/members`
- `POST /admin/v1/members/invite`
- `PATCH /admin/v1/members/:member_id/role`
- `DELETE /admin/v1/members/:member_id`
- `GET /admin/v1/service_accounts`
- `POST /admin/v1/service_accounts`
- `DELETE /admin/v1/service_accounts/:service_account_id`
- `GET /admin/v1/admin_keys`
- `POST /admin/v1/admin_keys`
- `DELETE /admin/v1/admin_keys/:key_id`
- `GET /admin/v1/oidc_providers`
- `POST /admin/v1/oidc_providers`
- `PATCH /admin/v1/oidc_providers/:oidc_id`
- `DELETE /admin/v1/oidc_providers/:oidc_id`
- `GET /admin/v1/billing`

**Public sub-router** (no auth):
- `POST /admin/v1/auth/signin`
- `GET /admin/v1/auth/oidc/callback`

**Dual-auth sub-router** (dual_auth_middleware layer — accepts operator key OR session credential; sets `AuthPrincipal` request extension):
- `GET /admin/v1/projects/:project_id`

The `dual_auth_middleware` tries session cookie / admin_api_key Bearer first; if neither is present, falls back to operator Bearer. Sets `AuthPrincipal::User(SessionContext)` or `AuthPrincipal::Operator` as a request extension. The handler reads this extension to determine response shape (account-scoped for User, unrestricted for Operator) and returns 401 only if neither credential validates.

**State types:**
- `OperatorState { system_db, admin_key, credential_cache, aws_secret_fetcher, gcp_secret_fetcher }` — used by operator sub-router (renames existing `AdminState`)
- `UserAdminState { system_db, encryption_key: [u8; 32], email_sender: Arc<dyn IEmailSender>, credential_cache }` — used by session sub-router
- Both are `with_state()`-erased before merging so the final router is `Router<()>`

**File structure:**
```
crates/embyr-server/src/admin/
├── router.rs                  (extend: build_admin_router merges four sub-routers)
├── state.rs                   (new: OperatorState, UserAdminState)
├── middleware/
│   ├── operator_auth.rs       (new: Tower fn_with_state middleware)
│   ├── session_auth.rs        (new: cookie OR admin_api_key Bearer; sets SessionContext)
│   └── dual_auth.rs           (new: tries session then operator; sets AuthPrincipal)
├── extractors/
│   ├── session_context.rs     (new: FromRequestParts; queries sessions or admin_api_keys)
│   └── dual_auth_principal.rs (new: AuthPrincipal enum)
└── handlers/
    ├── provision.rs           (modify: AdminState → OperatorState)
    ├── get_project.rs         (modify: uses AuthPrincipal extension)
    ├── lifecycle.rs           (unchanged)
    └── [new handler modules]
```

## Alternatives Considered

### Option B: Single router with typed Axum extractors (rejected)

All routes in one router. Each handler declares its auth requirement via a typed extractor (`OperatorKey(String)`, `SessionAuth(SessionContext)`, `DualAuth(AuthPrincipal)`). The extractor implementation does the lookup.

**Rejected because:**
- Enforcement is per-handler convention, not structural. A handler that omits the auth extractor from its parameters compiles successfully but is unprotected. The compiler gives no warning.
- No centralized place to audit "which routes require which auth" — the answer is distributed across handler signatures.
- Violates D2's intent of "separated at the router level": per-extractor enforcement is at the handler level, not the router level.
- Cannot be tested as a layer in isolation — requires instantiating every handler to verify auth behaviour.

### Option C: Axum `route_layer` called twice on one router (rejected)

Use Axum's `route_layer` to apply different middleware to different route groups in a single builder chain. Routes added before the first `route_layer` call get one middleware; routes added after get another.

**Rejected because:**
- Axum 0.7 `route_layer` applies to all routes in the router at the time of the call. A second `route_layer` call also applies to all routes added so far, meaning routes in the first group receive both middleware layers — creating cross-contamination.
- The resulting layering order is not immediately obvious from reading the builder chain.
- Two route groups with the same path prefix but different auth requirements (e.g., `GET /admin/v1/projects/:id`) still cannot co-exist in the same router.

## Consequences

**Positive:**
- Operator routes are structurally isolated from session routes. A bug in `session_auth_middleware` cannot affect operator route authentication and vice versa.
- Each sub-router is testable in isolation using `axum_test::TestClient` without instantiating the full combined router.
- The public sub-router (signin, OIDC callback) has no middleware — no auth code runs on the signin path at all, eliminating accidental token extraction from an unauthenticated context.
- Route inventory per auth type is explicit and readable in `router.rs`.
- Compatible with Axum 0.7's `with_state()` + `merge()` pattern; the state-erasure means routers with different state types can be merged.

**Negative / Trade-offs:**
- `GET /admin/v1/projects/:id` requires the `dual_auth_middleware` and `AuthPrincipal` enum — additional complexity for one route. This is bounded and does not proliferate.
- Four separate router construction functions are needed instead of one. Each sub-router's state type must be defined and wired in the composition root.
- The `session_auth_middleware` handles two distinct credential sources (cookie and admin_api_key Bearer). This is correct per D6 but means this one middleware has two code paths. Each path must be tested independently.

## Enforcement

The architectural invariant "operator routes never run session_auth_middleware" and "session routes never run operator_auth_middleware" is enforced structurally by Axum's layer scoping — a layer applied to sub-router A does not apply to sub-router B after merging. No tooling beyond Axum's own behaviour is required. Unit tests for each sub-router validate this at CI.

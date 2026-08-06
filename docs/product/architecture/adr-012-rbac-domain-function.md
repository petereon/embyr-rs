# ADR-012: RBAC Enforcement via Pure Domain Function

## Status

Accepted

## Context

The admin-api-v2 introduces three roles: `Owner > Admin > Viewer`. Role enforcement rules include non-trivial business logic:

- Owner-only routes: `GET/POST/PATCH/DELETE /admin/v1/oidc_providers`, Danger Zone operations
- Admin+ routes: create/revoke SDK keys, invite members, manage service accounts, create/revoke admin keys
- Viewer routes: all GETs

Invariants that go beyond simple role comparison:
- The sole Owner cannot demote or remove themselves (AC-B05-03, AC-B05-04)
- An Admin cannot change another Owner's role (AC-B05-03)
- An Admin cannot create an Owner-level admin API key (AC-B05-09: "Role must be ≤ creating user's role")
- Last-owner invariant: `DELETE /admin/v1/members/:id` with the last Owner → 409 (AC-B05-04)

Three placement options:
1. **Middleware** — a Tower layer reads `SessionContext.role` and checks it against a route-level required permission annotation
2. **Shared guard function** — a free function `require_role(ctx, required) -> Result<(), StatusCode>` called at the top of each handler body
3. **Pure domain function** — a domain-typed function in `embyr-core::admin::rbac` that encapsulates all invariants

## Decision

**Pure domain function in `embyr-core::admin::rbac` (Option 3).**

The RBAC module defines:

```
enum Role { Owner = 3, Admin = 2, Viewer = 1 }  // in embyr-core::admin::account

enum RbacAction {
    // General
    Read,
    // Member management
    InviteMember, ChangeMemberRole, RemoveMember,
    // Key management
    CreateSdkKey, RevokeSdkKey,
    CreateAdminKey { key_role: Role }, RevokAdminKey,
    // Service accounts
    CreateServiceAccount, DeleteServiceAccount,
    // OIDC / Danger Zone (Owner only)
    ManageOidcProviders, DangerZoneOp,
    // Project management
    PatchProject,
}

enum RbacError {
    InsufficientRole { required: Role, actual: Role },
    SelfDemotionForbidden,
    CannotChangeOwnerRole,
    LastOwnerRemovalForbidden,
    KeyRoleExceedsActorRole { requested: Role, actor: Role },
}

fn check_rbac(
    actor: &SessionContext,
    action: &RbacAction,
    target_user_id: Option<Uuid>,
    target_role: Option<Role>,
    is_last_owner: bool,
) -> Result<(), RbacError>
```

The function is a pure, synchronous, deterministic computation. It depends on no IO and no state — only on the input values. It lives in `embyr-core` because the RBAC invariants are domain rules ("the sole Owner cannot remove themselves") rather than HTTP concerns.

**Handler usage pattern:**

```rust
async fn change_member_role(
    session: SessionContext,
    State(state): State<UserAdminState>,
    Path(member_id): Path<Uuid>,
    Json(body): Json<ChangeMemberRoleRequest>,
) -> ApiResult<()> {
    // Load target member's current role
    let target = load_member(&state.system_db, session.account_id, member_id).await?;
    
    // Check RBAC — pure domain function call
    check_rbac(
        &session,
        &RbacAction::ChangeMemberRole,
        Some(member_id),
        Some(target.role),
        false,
    ).map_err(rbac_error_to_response)?;
    
    // ... perform the change
}

fn rbac_error_to_response(e: RbacError) -> ApiError {
    match e {
        RbacError::InsufficientRole { .. } => ApiError::Forbidden("Insufficient role"),
        RbacError::SelfDemotionForbidden => ApiError::Forbidden("Cannot demote yourself"),
        RbacError::CannotChangeOwnerRole => ApiError::Forbidden("Cannot change an Owner's role"),
        RbacError::LastOwnerRemovalForbidden => ApiError::Conflict("Transfer ownership before removing the last Owner"),
        RbacError::KeyRoleExceedsActorRole { .. } => ApiError::Forbidden("Cannot create a key with higher role than your own"),
    }
}
```

**`is_last_owner` determination:** The handler queries `SELECT count(*) FROM account_members WHERE account_id = $1 AND role = 'owner'` before calling `check_rbac`. If the result is 1 and the target is the actor, `is_last_owner = true`. This is a separate DB query, but it is only executed for removal and demotion operations.

## Alternatives Considered

### Option 1: Middleware (rejected)

A Tower middleware layer reads `SessionContext.role` and compares against a required permission annotation on the route. The annotation could be a custom Axum layer parameter or a middleware chain that varies per route.

**Rejected because:**
- The business invariants (self-demotion, last-owner check) require querying the database (current owner count). Middleware with DB access is technically possible but anti-idiomatic for Tower layers — it blurs the middleware concern boundary.
- The "Admin cannot change an Owner's role" invariant requires knowing the target member's role, which is only known after loading the target member from the DB. This is handler-level data, not middleware-level data.
- Middleware-based RBAC requires spinning up a full Axum test server to unit-test the RBAC logic. The pure function can be tested with 10-line unit tests.

### Option 2: Shared guard function (rejected)

`fn require_role(ctx: &SessionContext, required: Role) -> Result<(), StatusCode>` is a free function in `embyr-server::admin::guard`. Called at the top of each handler.

**Rejected because:**
- A simple role comparison does not encode the non-trivial invariants. A separate function is needed for each invariant (self-demotion, last-owner, key role cap). This fragments the RBAC logic across multiple guard functions.
- The guard functions live in `embyr-server`, not `embyr-core`. RBAC is a domain rule. Placing it in the infrastructure layer means it cannot be reused if a second driving adapter (e.g., a CLI admin tool) is added without re-implementing the guards.

## Consequences

**Positive:**
- All RBAC invariants are in one pure function — auditable, testable in isolation, no test server required.
- `check_rbac` is `embyr-core` code — testable with `cargo test` against simple struct inputs.
- Non-trivial invariants (last-owner, self-demotion, key role cap) are explicit in the `RbacAction` variants and `check_rbac` match arms.
- When a new role invariant is added, the compiler enforces that all callers handle the new `RbacError` variant (if added to the enum).

**Negative / Trade-offs:**
- The `is_last_owner` flag requires an extra SELECT before calling `check_rbac` for member removal and demotion operations. This adds ~1-5ms per operation but is acceptable for infrequent management operations.
- `check_rbac` must be called explicitly in every handler — it is not automatic. However, this is mitigated by integration tests that exercise all role combinations for every management operation (cross-role AC matrix tests in the acceptance test suite).

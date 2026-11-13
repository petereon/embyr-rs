# account-deletion — Feature Delta (combined DISCUSS + DESIGN)

Resolves gap G-45 (Danger Zone / Delete account) of `admin-ui-api-wiring`. Decision (user, final): mirror projects — soft-delete, 168-hour
grace, purge by the existing sweeper; Owner-only with password + TOTP re-entry; cancels the Stripe subscription.
Depends on: `account-bootstrap-and-invitations` not strictly, but on `dsn-single-store` slice 2 (purge SQL covers the new column) and the
signin credential-check refactor below.

## 1. Problem

There is no way for an account to leave. The console shows "Delete account" but nothing exists server-side: no `deleted_at` on
`accounts`, no endpoint, no purge, and no way to stop Stripe billing. Projects already have soft-delete (`status='deleted'`,
`deleted_at`) and a purge sweeper (`soft_delete_purge_sweeper`, 168 h default, nulls credential columns).

## 2. Acceptance criteria

- AC-ADL-01: Given an Owner with correct password and TOTP (or recovery code), When `DELETE /admin/v1/account`, Then 202 `{purge_after}` (= now + grace) and the session cookie is cleared.
- AC-ADL-02: Given an Admin or Viewer, Then 403 and nothing changes; given wrong password, wrong code or missing code, Then 401 (same bodies as signin) and nothing changes.
- AC-ADL-03: Given repeated wrong credentials, Then the existing per-user TOTP lockout (429 after 3) and the per-IP limiter apply — re-entry is not a lockout bypass.
- AC-ADL-04: Given a Pro (or any Stripe-backed) subscription, Then the Stripe subscription is cancelled **before** any local write; if Stripe fails → 502 and nothing is deleted; retry after a partial failure succeeds (cancelling an already-cancelled subscription is a success).
- AC-ADL-05: Given success, Then in one transaction: `accounts.deleted_at = now()`, all the account's projects become `deleted` with `deleted_at` (SDK keys revoked), all sessions expired, all admin API keys revoked, all pending invitations revoked; and the credential cache is evicted for each project.
- AC-ADL-06: Given a deleted account, Then its users cannot sign in (401 "Invalid credentials"), SDK/Firestore requests fail as for a deleted project, and `GET /projects` of any former session is 401.
- AC-ADL-07: Given the grace window has elapsed, Then the purge removes users, memberships, recovery codes, service accounts, admin keys, invitations, OIDC providers, subscription rows and sessions, blanks `accounts.name` and keeps the `accounts` row as a tombstone (projects keep an FK to it); the users' emails become reusable.
- AC-ADL-08: Given the window has not elapsed, Then the purge changes nothing for that account.
- AC-ADL-09: Given the sole Owner is the only member, Then deletion is still allowed (the account is going away); given other Owners/members, Then they are signed out too.
- AC-ADL-10: Given a second `DELETE /account` on an already-deleted account, Then 401 (session already invalid) — never a second Stripe call.
- AC-ADL-11: A structured audit log line records account id, actor user id, project count and Stripe outcome (no secrets).

## 3. Schema and migration

```
ALTER TABLE accounts ADD COLUMN deleted_at TIMESTAMPTZ;
CREATE INDEX accounts_deleted_at_idx ON accounts(deleted_at) WHERE deleted_at IS NOT NULL;
```
No restore endpoint in v1: like projects, the grace window protects against premature purge and lets an operator restore manually by
clearing `deleted_at` and re-activating projects. (If self-service undo is wanted it is a small follow-up: `POST /account/restore`
valid until purge, requires password + TOTP.) Purge grace: `EMBYR_ACCOUNT_DELETE_GRACE_DAYS`, default 7 (168 h).
Assumption: `projects.account_id` references `accounts(id)` without cascade (seen in migration 0015), which is why the `accounts` row
is kept as a tombstone rather than deleted.

## 4. Endpoint

`DELETE /admin/v1/account` (session cookie only; a Bearer admin key is refused, 403, because re-authentication needs a user)
Request: `{ "password": "…", "totp_code": "123456" }` or `{ "password": "…", "recovery_code": "…" }` plus `{ "confirm_account_name": "…" }`.
Response 202: `{ "deleted_at": "…", "purge_after": "…", "projects_deleted": 3, "subscription": "cancelled|none" }`.
Sign-in change: the signin query joins `accounts` and treats `deleted_at IS NOT NULL` as unknown email.
Flow: role check → rate limit → **shared credential verification** (extract signin steps 2–5 into one function used by both:
lockout check, Argon2 under the gate, TOTP/recovery check with attempt counting) → name confirmation → Stripe cancel (idempotent) →
transaction → evict caches → clear cookie. The client-side name confirmation is also checked server-side.

## 5. Purge

Extend `soft_delete_purge_sweeper` with a second cycle function (same module, same interval, same advisory-lock wrapping): for accounts
with `deleted_at < now() - grace` and not yet tombstoned (`name <> ''`), delete dependent rows in dependency order in one transaction,
then blank `accounts.name`. Counter `embyr_account_purge_sweeper_purged_total`. Idempotent; a partial run is repeatable.

## 6. Security considerations

Destructive and irreversible after grace: requires Owner role, password, second factor, and the typed account name. Password+TOTP is
verified server-side through the same code path as signin (no weaker alternative). Stripe is cancelled first so a failure cannot leave
a billed but deleted account; the reverse (cancelled but not deleted) is safe and retryable. Session fixation: the session is expired in
the same transaction. Operator-driven cleanup of a half-deleted account is possible by re-running the request. Logs never contain
credentials. A stolen session alone cannot delete (needs password and TOTP).

## 7. Slice plan

1. Migration; extract shared credential-verification function from `signin` with no behaviour change. Tests: existing signin suite, AC-ADL-03.
2. `DELETE /account`: role, re-auth, Stripe cancel (new `StripeGateway::cancel_subscription`), transaction, cache eviction; signin join. Tests: AC-ADL-01, 02, 04..06, 09..11.
3. Purge cycle. Tests: AC-ADL-07, 08.
4. (UI, `admin-ui-api-wiring` slice 15) Danger Zone modal; "Transfer ownership" stays a role change in Members.

## 8. Test strategy

HTTP acceptance on the real router with testcontainers Postgres; Stripe via the existing real test-mode key convention
(`tests/card_payments_backend`), plus a failing-gateway stub for AC-ADL-04. Fault injection: Stripe failure then retry, crash between
Stripe and the transaction, wrong factor ×3 lockout, concurrent double delete. Purge tested by `run_cycle` with a backdated
`deleted_at`. Pure unit test for the grace cutoff. CI only.

# account-bootstrap-and-invitations — Feature Delta (combined DISCUSS + DESIGN)

Closes audit finding #63 (no first account; invitations cannot be accepted) and the remainder of #60 (email/role validation; the
caller-role cap is assumed done in the security PR but keeps its ACs here). Resolves gaps G-01, G-37, G-38 of `admin-ui-api-wiring`.

## 1. Problem

No production code creates an account, user, membership or TOTP secret; `invitations` has no token, so no one can accept; there is no
accept, TOTP-enrolment or revoke endpoint; `invite_member` accepts any email/role string (a bad role is a 500 from the CHECK).
`NoopEmailSender` is hardwired (`main.rs`), so nothing is delivered either.

## 2. Design decision: one mechanism

Bootstrap and invitation share **one enrolment path**: *an invitation token is redeemed in two steps, accept (password) then TOTP
confirm.* The operator bootstrap endpoint therefore does not create the user itself; it creates the account plus a single **Owner
invitation** and returns its token. The operator never sees a password or TOTP secret, which a "create account + Owner + TOTP secret"
endpoint would force them to handle. The first Owner is a real user after step 2, exactly as any invitee. (Refinement of decision 4's
wording; if the operator must receive an otpauth URI directly, add that variant behind the same redeem steps.)

## 3. Acceptance criteria

Operator bootstrap
- AC-ABI-01: Given the operator Bearer key, When `POST /admin/v1/accounts` with `account_name`, `owner_email`, Then 201 with `account_id`, `invitation_id`, `invite_token`, `expires_at` and no user or membership exists yet.
- AC-ABI-02: Given a session or no credential, Then 401/403; given an `owner_email` that already has a user or open invitation anywhere, Then 409; given a malformed email or empty name, Then 422.
Invite
- AC-ABI-03: Given an Admin or Owner, When inviting with a valid email and role ≤ their own, Then 202 with `invitation_id`, `invite_token` (shown once), `expires_at`; only the token's BLAKE3 hash is stored.
- AC-ABI-04: Given an Admin inviting `Owner` (#60), Then 403; given an unknown role or invalid email, Then 422 (never 500); given an email that already has a user, Then 409.
- AC-ABI-05: Given a Viewer, Then 403.
Redeem
- AC-ABI-06: Given a valid token, When `POST /admin/v1/auth/invitations/accept` with `token`, `display_name`, `password`, Then 200 with `otpauth_uri`, `totp_secret` (shown once) and a pending enrolment; no user, membership or session is created yet.
- AC-ABI-07: Given an unknown, expired, revoked, already-used or malformed token, Then the same 404 `{"message":"Invalid or expired invitation"}` (no oracle).
- AC-ABI-08: Given a password shorter than 12 characters, Then 422.
- AC-ABI-09: Given a pending enrolment and a correct current TOTP code, When `POST /auth/invitations/confirm` with `token`, `totp_code`, Then 200: user, membership (invited role, `joined_at = now`) and 10 recovery codes (shown once) are created, the invitation is marked accepted, and signin with the new credentials succeeds.
- AC-ABI-10: Given 5 wrong codes on one pending enrolment, Then it is invalidated and the user must accept again; the token stays valid until expiry.
- AC-ABI-11: Given a token is accepted twice concurrently, Then exactly one user is created.
- AC-ABI-12: Given a token redeemed once, Then it cannot be used to create a second user.
Revoke and list
- AC-ABI-13: Given an Admin/Owner, When `DELETE /admin/v1/members/invitations/:id` for a pending invitation of their account, Then 204 and its token no longer redeems; foreign or unknown id → 404; Admin revoking an Owner invitation → 403.
- AC-ABI-14: Given `GET /members`, Then pending rows carry `invitation_id` as `id`, `auth_method: "pending"` and exclude revoked and accepted invitations.
Abuse limits
- AC-ABI-15: Given more than the allowed attempts per source IP on accept/confirm, Then 429 with `retry-after-ms`, before any Argon2 work.
- AC-ABI-16: Given concurrent accepts, Then Argon2 runs under the process-wide permit gate (finding #61).

## 4. Schema and migration

```
ALTER TABLE invitations
  ADD COLUMN token_hash  BYTEA,                 -- BLAKE3 of the token; NULL only for legacy rows (unusable, expire naturally)
  ADD COLUMN revoked_at  TIMESTAMPTZ,
  ADD COLUMN invited_by  UUID REFERENCES users(id) ON DELETE SET NULL;   -- NULL for operator bootstrap
CREATE UNIQUE INDEX invitations_token_hash_idx ON invitations(token_hash) WHERE token_hash IS NOT NULL;
CREATE UNIQUE INDEX invitations_open_email_idx ON invitations(lower(email)) WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE TABLE pending_enrolments (
  invitation_id    UUID PRIMARY KEY REFERENCES invitations(id) ON DELETE CASCADE,
  display_name     TEXT        NOT NULL,
  password_hash    TEXT        NOT NULL,       -- Argon2id, same as users.password_hash
  totp_secret_enc  BYTEA       NOT NULL,       -- AES-256-GCM under the server key, same format signin decrypts
  failed_attempts  INT         NOT NULL DEFAULT 0,
  expires_at       TIMESTAMPTZ NOT NULL        -- now() + 30 min
);
```
Existing pending invitations (no token) cannot be redeemed; they expire in ≤ 7 days and can be re-issued. `users.account_id` is NOT NULL
and `users.email` is globally UNIQUE, so a person belongs to exactly one account (hence the 409s). Tokens: 32 random bytes, base64url,
looked up by hash; compared in constant time through the unique index. TOTP secret: 20 random bytes, stored as the signin path expects
(raw bytes, AES-GCM); the otpauth URI uses their base32 form. Recovery codes: 10 random codes, BLAKE3-hashed into `mfa_recovery_codes`.

## 5. Endpoints

| Route | Auth | Request → response |
|---|---|---|
| `POST /admin/v1/accounts` | operator Bearer | `{account_name, owner_email}` → 201 `{account_id, invitation_id, invite_token, expires_at}` |
| `POST /admin/v1/members/invite` (changed) | session Admin+ | `{email, role}` → 202 `{invitation_id, email, role, expires_at, invite_token}` |
| `DELETE /admin/v1/members/invitations/:id` | session Admin+ | → 204 |
| `POST /admin/v1/auth/invitations/accept` | public, rate-limited | `{token, display_name, password}` → 200 `{otpauth_uri, totp_secret, enrolment_expires_at}` |
| `POST /admin/v1/auth/invitations/confirm` | public, rate-limited | `{token, totp_code}` → 200 `{recovery_codes: [...], email}` (user then signs in normally) |

The invite token is returned to the inviter because no email transport exists (`NoopEmailSender`); the SPA shows a copyable link
`/admin/?invite=<token>`. When a real `IEmailSender` is later wired the response keeps the token (inviter can still copy), and the email
carries the same link. Accept/confirm do not set a session cookie: the first sign-in proves the credentials end to end.

## 6. Security considerations

Token is a bearer secret: hashed at rest, single-use, 7-day expiry, never logged, `Referrer-Policy: no-referrer` on the SPA; the link
puts it in the query string, so the SPA must remove it from history (`replaceState`) on load. Unknown/expired/used tokens are
indistinguishable (AC-ABI-07). Role cap compares to the caller's role, not the session's account. Operator endpoint reuses
`operator_auth_middleware` (constant-time compare, rotation window). Enrolment does not create access before TOTP is proven, and an
unconfirmed enrolment stores only hashes and an encrypted secret. Argon2 under the global gate; per-IP limiter reuses the signin limiter
type (separate bucket). Password is never returned or logged; request `Debug` redacts it.

## 7. Slice plan

1. Migration; token generation/hash helpers (pure); validation of email/role/cap (pure fn in `embyr-core`). Tests: AC-ABI-04.
2. `invite_member` change + revoke + list. Tests: AC-ABI-03..05, 13, 14.
3. Accept + confirm + limiter. Tests: AC-ABI-06..12, 15, 16.
4. Operator `POST /accounts`. Tests: AC-ABI-01, 02 and an end-to-end "fresh install → first sign-in" test.
5. (UI, in `admin-ui-api-wiring` slice 14) accept/enrol screen and invite-link reveal.

## 8. Test strategy

Pure unit tests: validation, role cap, token hash, otpauth URI. HTTP acceptance on the real admin router with testcontainers (extends
`tests/admin_api_v2` harness): full journey bootstrap → accept → confirm → signin → invite → accept. Fault injection: double redeem race,
expired token, confirm without accept, wrong codes ×5, DB failure between steps (no half-created user: the confirm step is one
transaction). Rate-limit behaviour with the in-process limiter. CI only.

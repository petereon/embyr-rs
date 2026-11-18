# admin-oidc-sso — Feature Delta (combined DISCUSS + DESIGN)

Decisions and rationale: **ADR-086** (`docs/product/architecture/adr-086-admin-oidc-sso.md`). This file is the buildable contract: scope,
ACs, schema, routes, UI, tests, slices, product-owner decisions (section 9, all answered 2026-11-18). Supersedes the earlier draft of this file (its email-based linking, TOTP-bypass
note and GET `start` were replaced; see ADR-086 Alternatives). Context: audit #58 (callback minted a session for the wrong user; the
route was disabled in PR #14 and always redirects to `/admin/?error=oidc_failed`), UI gap G-13 and slice 16 of `admin-ui-api-wiring`
(Google/GitHub buttons hidden, OIDC settings section removed in PR #64). What exists today: `oidc_providers` table (0013) and Owner-only
CRUD (`handlers/oidc_providers.rs`, `POST/GET/PATCH/DELETE /oidc_providers`), stub `oidc_callback`, `google_jwks_cache.rs` (hosted
identity, single fixed URL), `dsn_policy.rs` (SSRF predicate), `ServerKeys`, `SigninRateLimiter`. Nothing is salvaged from the old callback.

## 1. Scope

Admin-console users only (not end-user hosted identity, which is separate). Per-account providers managed by the account Owner; generic
OIDC via discovery; SSO adds a way to sign in, never a way to create accounts. Out: SAML, GitHub login, per-instance providers,
just-in-time provisioning, SCIM, IdP-initiated login, back-channel logout.

## 2. Flow (summary of ADR-086 decisions 2-4, 8)

1. SPA at `/admin/?sso=<provider_id>` (link shown to the Owner in Settings; invite screen for SSO invitations; Settings "Link SSO") calls
   `POST /admin/v1/auth/oidc/start` and navigates to the returned `authorization_url`
   (`response_type=code`, `scope=openid email`, `code_challenge_method=S256`, `state`, `nonce`, fixed `redirect_uri`).
2. The IdP redirects to `GET /admin/v1/auth/oidc/callback?code&state`. The server consumes the state row, checks the `embyr_sso`
   cookie binding, exchanges the code server-side with the PKCE verifier and client secret, validates the ID token, resolves the user
   from `admin_sso_identities` (or performs the invite/link intent), issues the normal session, and answers `302 /admin/`.

## 3. Acceptance criteria

Start / state
- AC-SSO-01: Given an enabled provider of a live account and `EMBYR_PUBLIC_BASE_URL` set, `POST .../oidc/start {provider_id}` returns 200
  `{authorization_url}` with the parameters above, stores one single-use row, and sets `embyr_sso` (`HttpOnly; Secure; SameSite=Lax;
  Path=/admin/v1/auth/oidc; Max-Age=600`).
- AC-SSO-02: Unknown, disabled, deleted-account or discovery-failed provider, or unset `EMBYR_PUBLIC_BASE_URL` -> the same 200-with-error
  body `{error:"sso_unavailable"}`; no row is stored.
- AC-SSO-03: Callback with unknown, reused, expired (>10 min), or cookie-mismatched `state`, or with an IdP `error` parameter ->
  `302 /admin/?error=sso_failed`, no session; the state row is consumed on every outcome including failure.
- AC-SSO-04: `start` and `callback` are limited per source IP under key `sso:{ip}` (429 + `retry-after-ms` on start; `sso_failed` on
  callback); live rows per provider are capped (200) and expired rows are purged.

Token validation (pure, exhaustively unit-tested; also end-to-end with the mock IdP)
- AC-SSO-05: The ID token is accepted only if: alg in {RS256, ES256} and matches the key type; signature valid against the JWKS of the
  provider's own discovery document; `iss` equals the stored issuer exactly; `aud` contains client_id (`azp` must equal client_id when present and is
  required when `aud` has several); `exp` and `iat` required and within 60 s leeway; `nonce` equals the stored nonce; `sub` non-empty and <= 255 bytes. Any miss -> `sso_failed`.
- AC-SSO-06: Discovery `issuer` differs from stored issuer, or any endpoint/JWKS URL is non-https, resolves to a forbidden address,
  redirects, exceeds the body cap or the 5 s timeout -> `sso_unavailable`/`sso_failed`; the JWKS URL is never derived from a token.
- AC-SSO-07: Provider create/enable validates the issuer (https, policy, discovery reachable and matching) -> 422 otherwise.

Identity and session
- AC-SSO-08: Verified token whose `(provider_id, sub)` is linked, user is a current member of the provider's account -> session row
  (`auth_method='sso'`) and cookie identical to password sign-in, `302 /admin/` with no query string, `Cache-Control: no-store`.
- AC-SSO-09: Verified token with an unlinked `sub` and intent `signin` -> `sso_failed`, regardless of whether its email matches any user
  (#58 regression test: two members, one owner; the session user is always the linked one).
- AC-SSO-10: A link whose user was removed from the account (member row gone, `users` row alive) cannot sign in; `remove_member` deletes
  the user's links (and any pending link request) and expires their sessions, password or SSO, in the same transaction as
  the member row (already how it ends password members' sessions today).
- AC-SSO-11: Invitation with `auth_method='sso'` (non-Owner role, account has an enabled provider): start with the invite token, callback
  with `email_verified=true` and email equal case-insensitively to the invitation email -> user (`password_hash NULL`, no TOTP), member,
  link and accepted invitation in one transaction, then session. Email mismatch, unverified email, expired/accepted invitation, or an
  Owner invitation with `sso` -> failure and nothing created.
- AC-SSO-12: Self-link: `start {intent:"link", password, totp_code}` requires a valid session plus `verify_credentials`; on callback the
  `(provider, sub)` is linked to that user; a `sub` already linked to another user fails. `DELETE /auth/sso_identity` unlinks (409 for an
  SSO-only user).
- AC-SSO-13 (O6): A provider deletion deletes its links and ends every session minted through it (FK cascade). Disabling a provider
  ends its SSO sessions in the same transaction (`DELETE FROM sessions WHERE sso_provider_id = $1`); re-enabling or a no-op patch
  ends nothing. Password sessions and other providers' sessions are never touched. No back-channel logout. Issuer and client_id
  cannot be edited.

Password interplay
- AC-SSO-14: `password_hash IS NULL`: `signin` returns the generic 401 (same body, one dummy Argon2 verify so timing matches), `verify_credentials`
  fails safely, password reset request returns generic success without sending and confirm never sets a password.
- AC-SSO-15: `PATCH /account {sso_required:true}` (Owner; needs an enabled provider and a caller with a link) makes password sign-in
  return the generic 401 for non-Owners. Owners always sign in with password + TOTP. Owners can never be SSO-only. Toggling is audited.
- AC-SSO-16: `DELETE /account` and self-link still require password + TOTP (SSO-only members cannot perform them).

Secrets, audit, hygiene
- AC-SSO-17: `client_secret_enc` is AES-256-GCM under `EMBYR_ENCRYPTION_KEY` via `ServerKeys`; decrypts with the previous key and is
  re-encrypted lazily; `PUT /oidc_providers/:id/secret` (Owner) replaces it; never returned or logged.
- AC-SSO-18: No code, verifier, token, secret or email claim appears in logs or any URL (including audit rows and metrics labels);
  audit rows exist for the events in ADR-086 decision 9 and carry a granular failure reason that the browser never sees. The
  audit writer accepts labels only (`[a-z0-9_.:-]`, <= 64 chars) for event and reason, so free text cannot be stored. Audit rows are
  kept 12 months (swept); there is no owner-facing audit view at launch (O5).
- AC-SSO-19: Every failure after state is read produces the same redirect and message; response shape and status do not differ between
  "no such link", "link exists but user removed" and "bad token".

## 4. Data model — migration `0051_admin_sso.sql` (verified next free number on master at implementation: 0050 was the highest; 0043, 0044 and 0046 are unused gaps)

Authoritative text is `migrations/0051_admin_sso.sql`; summary of what changed against the earlier draft of this section:
`sessions` also gets `sso_provider_id UUID REFERENCES oidc_providers(id) ON DELETE CASCADE` (O6) with a CHECK that it is set exactly
when `auth_method = 'sso'`; `admin_audit_events` gains a mandatory `outcome` ('ok'|'failed') and an index on `at` for the retention
sweep, and has no foreign keys on purpose (rows outlive what they describe); `admin_sso_identities` gets an index on `user_id` and
`sso_auth_requests` one on `provider_id`.

```sql
-- admin-oidc-sso (ADR-086), slices 1-2. Expand-only (docs/operations/migration-policy.md):
-- one relaxed constraint, new nullable/defaulted columns, new tables, new indexes. No drops, no renames.
-- N-1 safety: no NULL password_hash exists until the new binary creates an SSO-only user, and
-- no N-1 code path can violate the new constraints (they only cover columns N-1 never writes).

-- NULL password_hash = SSO-only user. Every reader treats NULL as "no password".
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- Anchors for the composite foreign keys that make a cross-account SSO link unrepresentable.
CREATE UNIQUE INDEX users_id_account_uidx          ON users(id, account_id);
CREATE UNIQUE INDEX oidc_providers_id_account_uidx ON oidc_providers(id, account_id);

ALTER TABLE invitations ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password'
  CHECK (auth_method IN ('password', 'sso'));
ALTER TABLE accounts ADD COLUMN sso_required BOOLEAN NOT NULL DEFAULT false;

-- O6: a session minted by SSO remembers its provider, so disabling the provider can end it and
-- deleting the provider ends it by cascade. Password sessions keep NULL.
ALTER TABLE sessions
  ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password' CHECK (auth_method IN ('password', 'sso')),
  ADD COLUMN sso_provider_id UUID REFERENCES oidc_providers(id) ON DELETE CASCADE,
  ADD CONSTRAINT sessions_sso_provider_chk CHECK ((auth_method = 'sso') = (sso_provider_id IS NOT NULL));
CREATE INDEX sessions_sso_provider_idx ON sessions(sso_provider_id) WHERE sso_provider_id IS NOT NULL;

-- The only way to sign in as a user through a provider: (provider, IdP subject) -> user.
CREATE TABLE admin_sso_identities (
  provider_id   UUID        NOT NULL,
  subject       TEXT        NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
  account_id    UUID        NOT NULL,
  user_id       UUID        NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_login_at TIMESTAMPTZ,
  PRIMARY KEY (provider_id, subject),
  UNIQUE (provider_id, user_id),
  FOREIGN KEY (provider_id, account_id) REFERENCES oidc_providers(id, account_id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, account_id)     REFERENCES users(id, account_id)          ON DELETE CASCADE
);
CREATE INDEX admin_sso_identities_user_idx ON admin_sso_identities(user_id);

-- One row per started authorization request; consumed (deleted) on every callback outcome.
CREATE TABLE sso_auth_requests (
  state_hash        BYTEA       PRIMARY KEY,                       -- BLAKE3(state)
  provider_id       UUID        NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  intent            TEXT        NOT NULL CHECK (intent IN ('signin', 'invite', 'link')),
  invitation_id     UUID        REFERENCES invitations(id) ON DELETE CASCADE,
  user_id           UUID        REFERENCES users(id) ON DELETE CASCADE,
  nonce             TEXT        NOT NULL,
  code_verifier_enc BYTEA       NOT NULL,
  browser_binding   BYTEA       NOT NULL,                          -- BLAKE3(embyr_sso cookie)
  expires_at        TIMESTAMPTZ NOT NULL,
  CHECK ((intent = 'signin' AND invitation_id IS NULL     AND user_id IS NULL)
      OR (intent = 'invite' AND invitation_id IS NOT NULL AND user_id IS NULL)
      OR (intent = 'link'   AND user_id IS NOT NULL       AND invitation_id IS NULL))
);
CREATE INDEX sso_auth_requests_expiry_idx ON sso_auth_requests(expires_at);
CREATE INDEX sso_auth_requests_provider_idx ON sso_auth_requests(provider_id);

-- Generic admin audit trail (labels only, no tokens or claims). No foreign keys on purpose: rows
-- must outlive the user/provider/account they describe. 12-month retention (swept).
CREATE TABLE admin_audit_events (
  id            BIGSERIAL   PRIMARY KEY,
  at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  event         TEXT        NOT NULL,
  outcome       TEXT        NOT NULL CHECK (outcome IN ('ok', 'failed')),
  account_id    UUID,
  actor_user_id UUID,
  provider_id   UUID,
  reason        TEXT,
  source_ip     INET
);
CREATE INDEX admin_audit_events_account_idx ON admin_audit_events(account_id, at DESC);
CREATE INDEX admin_audit_events_at_idx      ON admin_audit_events(at);
```
Rollout: expand-only apart from `DROP NOT NULL` (a relaxation, allowed by `docs/operations/migration-policy.md`; `scripts/check-migration-policy.sh`
passes without an opt-out). No NULL password exists until the feature code creates an SSO-only user (slice 4 at the earliest), so N-1
binaries are safe (deploy migration, then binary). New constraints only cover columns and tables N-1 never writes. `admin_audit_events`
is generic so later features (key rotation, member changes) can reuse it; its 12-month purge rides the existing
`signin_rate_limit_sweeper` cycle (one more idempotent DELETE), so no new sweeper is needed.

## 5. API

| Route | Auth | Notes |
|---|---|---|
| `POST /admin/v1/auth/oidc/start` `{provider_id, invite_token?, intent?, password?, totp_code?}` | public; `link` needs session | returns `{authorization_url}` or `{error}`; rate-limited |
| `GET /admin/v1/auth/oidc/callback?code&state` | public | replaces the disabled stub; always a 302 |
| `DELETE /admin/v1/auth/sso_identity` | session | unlink own identity |
| `PUT /admin/v1/oidc_providers/:id/secret` `{client_secret}` | Owner | rotate client secret; 204, never echoed |
| `PATCH /admin/v1/account` `{sso_required}` | Owner | new; existing provider routes keep their shapes, create/enable gain issuer validation |
| `GET /admin/v1/auth/session` | session | add `auth_method`, `sso_linked` (bool), `sso_required` for the SPA |
| Invitation create | existing | `auth_method` on create (`sso` rejected for Owner or when no enabled provider) |
| `POST /admin/v1/auth/invitations/lookup` `{token}` | public, rate-limited | new in slice 4: `{auth_method, sso_provider_ids?}` (see slice 4-5 notes) |

No audit read endpoint in V1 (O5). New config: none beyond `EMBYR_PUBLIC_BASE_URL` (already exists; note it must be the console origin that proxies `/admin/v1`).

## 6. SPA changes (`admin-ui-api-wiring` slice 16 and a Settings section)

- Sign-in view: if `?sso=<id>` is present, call `start` and navigate; otherwise show a collapsed "Sign in with SSO" help text pointing to the
  Owner's link (no email box, so no account-existence oracle). Error banners map `sso_failed` and `sso_unavailable` to one static sentence each.
- Settings -> Single sign-on (Owner only): list/add/enable/disable/delete provider, rotate secret, copy the sign-in link, copy the
  redirect URI to register at the IdP, "Require SSO for non-Owners" switch with the break-glass explanation. Any member: "Link SSO" /
  "Unlink" (password + TOTP prompt).
- Invite dialog: "Sign-in method" (Password + authenticator | SSO) shown only for non-Owner roles when a provider is enabled; invite accept
  screen shows "Continue with SSO" for SSO invitations. The GitHub button stays dropped (G-13); AC-AUW-28 lifts only after slice 6.

SPA status: slice 7a (sign-in start, providers, SSO invitations, link/unlink) shipped in PR #88. Slice 7b (this section's `sso_required`
parts, needs the slice 6 server at runtime) adds: session fields `auth_method` / `sso_linked` / `sso_required` (absent on older servers:
password / false / false); Settings "Require SSO for members" (Owner only, confirm before enabling, 422 `sso_required_prerequisites` and
403 mapped to plain sentences); a persistent banner (non-Owner linked: informational; non-Owner not linked: link before the session
ends; Owner: reminder of the exemption); Link / Unlink driven by the session (SSO-session users see neither, per O4; non-Owners cannot
unlink while required); invite dialog allows only SSO invitations for Admin/Viewer while required.

## 7. Test plan

Pure unit/property tests in `embyr-core`: PKCE vector (RFC 7636 appendix B), state/nonce entropy and uniqueness, claim validator (every
rejection in AC-SSO-05 plus boundaries), link-intent rules, outbound URL/IP policy table. Acceptance tests on the real router with
testcontainers Postgres (CI only) and a **tiny in-process mock OIDC server (axum, ~150 lines, generated RSA and EC keys, configurable
claims, discovery/JWKS/token endpoints, fault switches)** shared with future OIDC-adjacent tests; it needs `EMBYR_ALLOW_PRIVATE_DSN_HOSTS`
because it listens on 127.0.0.1, so the SSRF refusal itself is tested separately at unit level and by an acceptance test with the flag
off against a loopback issuer. A fake IdP is required for: AC-SSO-01..03, 05, 06, 08..12, 14, 15, 17, 19. Fault injection: wrong nonce,
replayed/expired/foreign state, missing/mismatched cookie, wrong aud/iss/azp, `alg=none`/HS256/key-type confusion, unknown `kid` then
rotation, expired token, unverified email, email of another member (#58), unlinked sub, JWKS 500/slow/oversized, token endpoint 500,
discovery issuer mismatch, redirect from discovery, removed member with surviving user row, `sso_required` with a non-Owner and with an
Owner, SSO-only user on password signin and reset. No real-IdP test in CI; before release a manual contract check against Google (and
the chosen second IdP) with a throwaway client.

## 8. Slice plan

| # | Slice | Size | Depends on |
|---|---|---|---|
| 1 (done, PR "slices 1-2") | Migration 0051 (next free at implementation time); pure core (PKCE, state/nonce, claim validator, URL/IP policy extracted from `dsn_policy`); `issue_session` extraction; mock IdP harness | M | none |
| 2 (done, same PR) | Nullable `password_hash` in `signin`, `verify_credentials`, password reset; `admin_audit_events` writer; `remove_member` deletes links; SSO-session revocation on provider disable (O6) | S | 1 |
| 3 (done, PR "slice 3", stacked on slices 1-2) | `start` + discovery/JWKS caches + `callback` for intent `signin`; provider create/enable validation; `ServerKeys` for provider secret + `PUT .../secret` | L | 1, 2 |
| 4 (done, PR "slices 4-5", stacked on slice 3) | Invitation `auth_method='sso'` (create, lookup, callback intent `invite`) | M | 3, `account-bootstrap-and-invitations` (landed) |
| 5 (done, same PR) | Self-link/unlink (intent `link`, re-auth) | S | 3 |
| 6 (done, PR "slice 6", stacked on slices 4-5) | `sso_required` + session fields + Owner exemption | S | 3 |
| 7 | SPA: sign-in start, Settings section, invite dialog/accept, banners | M | 3 (sign-in, Settings), 4-6 for their parts |

Slice 1 delivered: migration 0051; core modules `embyr_core::admin::sso::{pkce, claims, url_policy}` and `admin::audit`; server
`adapters::outbound_policy` (resolve + vet) with `dsn_policy::is_forbidden_ip` now re-exported from core; `issue_session` /
`SessionOrigin` in `handlers/auth.rs`; mock IdP `tests/common/mock_oidc_idp.rs` with a Docker-free self-test. Slice 2 delivered:
NULL-safe `signin`/`verify_credentials`/password reset, `adapters::admin_audit::write_audit_event`, `remove_member` link cleanup,
provider disable/delete ending SSO sessions, audit retention purge.

Slice 3 delivered: `handlers/oidc_sso.rs` (`start`, `callback`; the old stub and its `oidc_failed` redirect are gone),
`adapters/sso_client.rs` (capped, SSRF-policed fetches pinned to the vetted address, no redirects, no proxy, 5 s;
discovery and JWKS caches), pure `embyr_core::admin::sso::discovery` (issuer match, endpoint policy, authorization URL),
provider create/enable validation (422 `{error:"issuer_invalid", reason}`), `PUT /oidc_providers/:id/secret` (204), provider
secrets via `ServerKeys` with lazy re-encryption from the previous key, provider lifecycle audit events, expired-request
purge in the `signin_rate_limit_sweeper` cycle, and the `AdminTestContext::with_sso` / `with_public_base_url` harness
variants. Details settled while building (they refine, not change, the design above):

- The per-provider cap of 200 live requests answers `429` with `retry-after-ms: 60000` (not `sso_unavailable`); it is one
  conditional `INSERT`, so it is race-free enough for an abuse bound. `start` also deletes up to 100 of the provider's expired rows.
- `start` and `callback` share one limiter key `sso:{ip}` on the existing sign-in limiter. A throttled callback redirects to
  `sso_failed` before touching the database: its state row is not consumed (it expires and is swept) and no audit row is written,
  like a callback without any `state`, so a flood cannot become a write amplifier.
- `start` answers `400` for an `intent` other than `signin` until slices 4 and 5; the state table, the CHECK constraint and the
  callback's `intent` match already carry `invite` and `link`.
- `redirect_uri` needs an `https` `EMBYR_PUBLIC_BASE_URL` unless `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true`; otherwise `sso_unavailable`.
- The forced JWKS refetch on an unknown `kid` happens at most once a minute per provider; inside that minute an unknown `kid` fails.
- Outbound calls build one client per request with `no_proxy()` so the connection really goes to the vetted address; an egress
  proxy therefore makes SSO unavailable (as the ADR already accepts).
- Create and the secret route reject an empty or over-1024-byte `client_id` / `client_secret` with 422 (new, cheap hygiene).
- Not done in this slice: the manual contract check against Google and the second IdP (O1); the unauthenticated `invite`
  and `link` intents (slices 4-5); SPA work (slice 7).

Slices 4-5 delivered (no migration; slice 1 created the tables): `handlers/oidc_sso_links.rs` (`complete_invite`, `complete_link`,
`DELETE /auth/sso_identity`; the only code that reads the e-mail claim or writes `users`, while `oidc_sso.rs` keeps its source-scan
guarantee of never touching `users` or e-mail), pure `embyr_core::admin::sso::enroll` (e-mail rule, Owner rule, display name),
`start` intents `invite` and `link`, `issue_invitation`/`send_invite_email` carrying `auth_method`, public
`POST /auth/invitations/lookup`, audit event `sso_invite_accepted`. Details settled while building:

- **Lookup is a new route.** No invitation lookup existed; `POST /admin/v1/auth/invitations/lookup {token}` (body, not URL;
  rate-limited like accept/confirm; same 404 as every dead token) returns `{auth_method}` and, for `sso`,
  `sso_provider_ids` (all enabled providers of the invitation's account; an array, not the single `sso_provider_id` of section 5,
  because an account can have several).
- **Password redemption of an SSO invitation is refused** (`accept`/`confirm` match only `auth_method='password'`): otherwise the
  invitee could bypass the Owner's choice with a password.
- `start` for `invite` answers `sso_unavailable` for every bad case (unknown, expired, revoked, used, password-method, Owner
  invitation, malformed or missing token, provider of another account), so there is no invitation-state oracle. For `link` it
  answers 401 without a valid session cookie or with failed `verify_credentials` (401/429, same as account deletion), the usual
  `sso_unavailable` for another account's provider, and `409 {error:"already_linked"}` when the user already has a link at that provider.
- `callback` `link` success is `302 /admin/?sso_linked=1` and mints no session (the user keeps theirs); every failure is the
  same `sso_failed` redirect, audited as `sso_link_created` / `sso_invite_accepted` with outcome `failed` and a reason
  (`invite_email_unverified|missing|mismatch`, `invitation_unavailable`, `owner_invitation`, `email_taken`, `link_conflict`, `member_removed`, ...).
- The invitation e-mail must also not belong to any existing user (case-insensitive, checked inside the transaction): an IdP
  login can never take over or merge into an existing user.
- SSO invitees get `display_name` = local part of the invitation e-mail (they enter no profile).
- Unlink: optional `?provider_id=`; removes the caller's link(s), ends the caller's other sessions minted through them, 404 when
  nothing was linked, 409 for an SSO-only user (user row locked so concurrent unlinks cannot race the check).
- The account-deletion re-auth already refused a NULL-password Owner (s02 test); SSO-only members are never Owners and get 403
  (s04 and s05 tests).
- The members list reports `auth_method: "sso"` for members without a password.

Slice 6 delivered (no migration; `accounts.sso_required` and the session columns came with 0051): pure
`embyr_core::admin::sso::policy` (`may_use_password`, `password_invite_allowed`, `may_enable_sso_required`, exhaustive table
tests), `PATCH /admin/v1/account {sso_required}` in `handlers/account.rs`, enforcement in `signin`, `invite_member` and both
password-reset endpoints, and `auth_method` / `sso_linked` / `sso_required` on `GET /auth/session`. Details settled while building:

- **Lock-out guard (the design said "enabled provider and the caller has a link"):** enabling needs an enabled provider on the
  account AND a link of the calling Owner to one of the account's enabled providers; otherwise 422
  `{error:"sso_required_prerequisites"}` (audited as `failed` with reason `no_enabled_provider` or `caller_not_linked`).
  Disabling is Owner-only, immediate and unconditional (no prerequisite, so it works after the provider was deleted).
  A repeat of the current value is a 200 no-op that writes no audit row. `deny_unknown_fields`: only `sso_required` is accepted.
- **Sign-in:** the member's role and the account flag are read in the same query as the user; for a non-Owner of a required
  account the request continues as an SSO-only user (no hash, no lockout state), so the answer is the generic 401 after one dummy
  Argon2 verify, same body and timing as a wrong password, and no lockout counters move. The former separate role query (step 6)
  is gone; a missing member row still answers 500 after verification.
- **Invitations:** `auth_method='password'` (explicit or default) for Admin/Viewer is 422 while required; Owner password invitations
  (exempt) and SSO invitations are unaffected.
- **Password reset:** `request` treats a non-Owner of a required account like an unknown address; `confirm` answers the usual 404 for
  a token that was issued before the flag was switched on.
- **Session probe:** `auth_method` is the session's own (`password` | `sso`; a Bearer admin key reports `password`), `sso_linked`
  is "this user has at least one link", `sso_required` is the account flag.
- The two gaps first noted here (an Owner demoted while required, password sessions surviving the switch) are closed, see
  "Known-gap fixes" below.

Known-gap fixes (branch `fix/sso-known-gaps`; no migration, no new environment variable). Pure decisions live in
`embyr_core::admin::sso::policy` (`check_role_change`, `may_remove_enabled_provider`, `may_unlink`, exhaustive table tests):

- **Role changes** (`PATCH /members/:id/role`): only a change across the Owner boundary alters how a member can sign in, so
  only those are checked, in one transaction with a shared lock on the account row. Owner -> Admin/Viewer while `sso_required`
  and the target has no link to an enabled provider of the account is 422 `{error:"sso_link_required", message}`. Non-Owner ->
  Owner for a member without a password is 422 `owner_requires_password` (O3: Owners are never SSO-only; found while
  reading, same lock-out family). Admin <-> Viewer and no-op changes are never refused, so an already locked-out member can
  still be re-roled. Refusals change nothing and are audited (`member_role_changed`, outcome `failed`, reason = the error
  code); successful changes are now audited too (reason `promoted`|`demoted`|`unchanged`).
- **Providers** (`PATCH`/`DELETE /oidc_providers/:id`): while `sso_required`, disabling or deleting the last enabled provider
  is 422 `{error:"sso_required_active", reason:"last_enabled_provider", message}` ("Turn off the SSO requirement first"),
  audited as `failed` (`sso_provider_disabled`/`sso_provider_deleted`, reason `sso_required_last_provider`). The decision
  is refuse, not auto-disable: switching a security setting off as a side effect would be surprising. Disabling or deleting a
  provider that is already disabled is unaffected; the account row is locked `FOR UPDATE` so two concurrent removals cannot
  each see the other provider enabled. `PATCH /account` takes the same lock before reading its own facts.
- **Unlink** (`DELETE /auth/sso_identity`): a non-Owner of a required account may not remove their last link to an enabled
  provider (409 `{error:"sso_link_required"}`, audited `sso_link_removed` failed, reason `sso_required_last_link`). Owners
  keep password + TOTP and are unaffected. This is the "last link" case: there is no separate guard for an Owner's link,
  because Owners never depend on SSO.
- **Password sessions on enable** (`PATCH /account {sso_required:true}`): in the same transaction as the flag, the sessions
  with `auth_method='password'` of the account's non-Owner members are deleted. SSO sessions, every Owner session (the
  caller's included) and API keys stay. The audit reason carries the count as a label: `enabled:sessions_ended_<n>`
  (previously `enabled`). A repeat of the current value is still a no-op (no audit row, nothing ended); disabling ends nothing.
- **JWKS refetch throttle is injectable**: `SsoClient::with_forced_refetch_min_age(Duration)` (default 60 s, which is what the
  composition root gets). `build_admin_router_with_sso_client` takes the client; `build_admin_router` is a thin wrapper
  passing `SsoClient::new()`. The harness exposes `AdminTestContext::with_sso_jwks_refetch_min_age`. Acceptance tests
  (`s03_start_and_callback.rs`) rotate the mock's key: inside the window the new `kid` is refused with no refetch, after it
  exactly one refetch makes the sign-in succeed, and a `kid` that never appears causes one refetch per window and no more.
- Still true: a non-Owner who had only a password and no link when the requirement was switched on cannot sign in (that is
  the requirement working); the Owner can turn it off, remove and re-invite the member with SSO, or link them first.

Slices 1-3 deliver working SSO for pre-linked users only if a link exists, so 4 or 5 is required before any real use; ship 1-5 together.
Total roughly 4-5 weeks of one engineer including the mock IdP.

## 9. Decisions (product owner, 2026-11-18; replaces the former open decisions)

- **O1 IdP coverage: generic OIDC.** No provider-specific code. Contract-checked before release against Google and one more IdP; the
  second one is still to be named (recommend Microsoft Entra with a tenant-specific issuer, which exercises the unverified-`email`
  and issuer-template quirks; Okta or Keycloak are the easy alternatives). Microsoft `/common` is unsupported (templated issuer fails
  exact match). Record the chosen pair and the check results in the slice 3 PR.
- **O2 No JIT provisioning in V1.** Links come only from an SSO invitation or an authenticated self-link. A domain-verified allowlist
  would be a separate future feature.
- **O3 Owners always keep password + TOTP** and are exempt from `sso_required`; Owners can never be SSO-only. That is the break-glass path.
- **O4 SSO-only members cannot delete the account or self-link** (both need password + TOTP). No fresh-login SSO reauth in V1.
- **O5 Audit.** Rows are written in V1 (`admin_audit_events`). No owner-facing audit view at launch; no read API. Retention 12 months,
  enforced by one DELETE in the `signin_rate_limit_sweeper` cycle (no new sweeper).
- **O6 Provider lifecycle ends sessions.** Sessions carry `auth_method` and a nullable `sso_provider_id` (FK to `oidc_providers`, `ON DELETE
  CASCADE`). Disabling a provider deletes its SSO sessions; deleting it removes them by cascade. Removing a member ends their sessions
  (password or SSO) as it does today and also deletes their links. No back-channel logout; SSO sessions keep the 24 h lifetime.
- **O7 Multi-account people: out of scope.** `users.email` stays globally unique with one account per user.
- **SAML:** explicitly out; revisit on customer demand (separate ADR).

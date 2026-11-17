# admin-oidc-sso — Feature Delta (combined DISCUSS + DESIGN)

Decisions and rationale: **ADR-086** (`docs/product/architecture/adr-086-admin-oidc-sso.md`). This file is the buildable contract: scope,
ACs, schema, routes, UI, tests, slices, open decisions. Supersedes the earlier draft of this file (its email-based linking, TOTP-bypass
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
  provider's own discovery document; `iss` equals the stored issuer exactly; `aud` contains client_id (`azp` checked when several);
  `exp` and `iat` within 60 s leeway; `nonce` equals the stored nonce; `sub` non-empty and <= 255 bytes. Any miss -> `sso_failed`.
- AC-SSO-06: Discovery `issuer` differs from stored issuer, or any endpoint/JWKS URL is non-https, resolves to a forbidden address,
  redirects, exceeds the body cap or the 5 s timeout -> `sso_unavailable`/`sso_failed`; the JWKS URL is never derived from a token.
- AC-SSO-07: Provider create/enable validates the issuer (https, policy, discovery reachable and matching) -> 422 otherwise.

Identity and session
- AC-SSO-08: Verified token whose `(provider_id, sub)` is linked, user is a current member of the provider's account -> session row
  (`auth_method='sso'`) and cookie identical to password sign-in, `302 /admin/` with no query string, `Cache-Control: no-store`.
- AC-SSO-09: Verified token with an unlinked `sub` and intent `signin` -> `sso_failed`, regardless of whether its email matches any user
  (#58 regression test: two members, one owner; the session user is always the linked one).
- AC-SSO-10: A link whose user was removed from the account (member row gone, `users` row alive) cannot sign in; `remove_member` deletes
  the user's links in the same transaction as the member row.
- AC-SSO-11: Invitation with `auth_method='sso'` (non-Owner role, account has an enabled provider): start with the invite token, callback
  with `email_verified=true` and email equal case-insensitively to the invitation email -> user (`password_hash NULL`, no TOTP), member,
  link and accepted invitation in one transaction, then session. Email mismatch, unverified email, expired/accepted invitation, or an
  Owner invitation with `sso` -> failure and nothing created.
- AC-SSO-12: Self-link: `start {intent:"link", password, totp_code}` requires a valid session plus `verify_credentials`; on callback the
  `(provider, sub)` is linked to that user; a `sub` already linked to another user fails. `DELETE /auth/sso_identity` unlinks (409 for an
  SSO-only user).
- AC-SSO-13: A provider deletion deletes its links (cascade). Disabling or deleting a provider does not end existing sessions; issuer
  and client_id cannot be edited.

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
  audit rows exist for the events in ADR-086 decision 9 and carry a granular failure reason that the browser never sees.
- AC-SSO-19: Every failure after state is read produces the same redirect and message; response shape and status do not differ between
  "no such link", "link exists but user removed" and "bad token".

## 4. Data model — migration `0041_admin_sso.sql` (next free: master ends at 0040, no open branch adds one)

```sql
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;            -- NULL = SSO-only
CREATE UNIQUE INDEX users_id_account_uidx           ON users(id, account_id);
CREATE UNIQUE INDEX oidc_providers_id_account_uidx  ON oidc_providers(id, account_id);
ALTER TABLE invitations ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password' CHECK (auth_method IN ('password','sso'));
ALTER TABLE accounts    ADD COLUMN sso_required BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE sessions    ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password' CHECK (auth_method IN ('password','sso'));

CREATE TABLE admin_sso_identities (
  provider_id UUID NOT NULL, subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
  account_id  UUID NOT NULL, user_id UUID NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(), last_login_at TIMESTAMPTZ,
  PRIMARY KEY (provider_id, subject),
  UNIQUE (provider_id, user_id),
  FOREIGN KEY (provider_id, account_id) REFERENCES oidc_providers(id, account_id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, account_id)     REFERENCES users(id, account_id)          ON DELETE CASCADE
);
CREATE TABLE sso_auth_requests (
  state_hash BYTEA PRIMARY KEY,                       -- BLAKE3(state)
  provider_id UUID NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  intent TEXT NOT NULL CHECK (intent IN ('signin','invite','link')),
  invitation_id UUID REFERENCES invitations(id) ON DELETE CASCADE,
  user_id UUID REFERENCES users(id) ON DELETE CASCADE,
  nonce TEXT NOT NULL, code_verifier_enc BYTEA NOT NULL,
  browser_binding BYTEA NOT NULL,                     -- BLAKE3(embyr_sso cookie)
  expires_at TIMESTAMPTZ NOT NULL,
  CHECK ((intent='signin' AND invitation_id IS NULL AND user_id IS NULL)
      OR (intent='invite' AND invitation_id IS NOT NULL AND user_id IS NULL)
      OR (intent='link'   AND user_id IS NOT NULL AND invitation_id IS NULL))
);
CREATE INDEX sso_auth_requests_expiry_idx ON sso_auth_requests(expires_at);
CREATE TABLE admin_audit_events (
  id BIGSERIAL PRIMARY KEY, at TIMESTAMPTZ NOT NULL DEFAULT now(),
  account_id UUID, actor_user_id UUID, provider_id UUID,
  event TEXT NOT NULL, reason TEXT, source_ip INET
);
CREATE INDEX admin_audit_events_account_idx ON admin_audit_events(account_id, at DESC);
```
Rollout: expand-only apart from `DROP NOT NULL`; no NULL password exists until the feature code runs, so N-1 binaries are safe
(deploy migration, then binary; consistent with the migration policy work, go-live O2). `admin_audit_events` is generic so later
features (key rotation, member changes) can reuse it.

## 5. API

| Route | Auth | Notes |
|---|---|---|
| `POST /admin/v1/auth/oidc/start` `{provider_id, invite_token?, intent?, password?, totp_code?}` | public; `link` needs session | returns `{authorization_url}` or `{error}`; rate-limited |
| `GET /admin/v1/auth/oidc/callback?code&state` | public | replaces the disabled stub; always a 302 |
| `DELETE /admin/v1/auth/sso_identity` | session | unlink own identity |
| `PUT /admin/v1/oidc_providers/:id/secret` `{client_secret}` | Owner | rotate client secret |
| `PATCH /admin/v1/account` `{sso_required}` | Owner | new; existing provider routes keep their shapes, create/enable gain issuer validation |
| `GET /admin/v1/auth/session` | session | add `auth_method`, `sso_linked` (bool), `sso_required` for the SPA |
| Invitation create/lookup | existing | `auth_method` on create (`sso` rejected for Owner or when no enabled provider); lookup returns `sso_provider_id` when `sso` |

New config: none beyond `EMBYR_PUBLIC_BASE_URL` (already exists; note it must be the console origin that proxies `/admin/v1`).

## 6. SPA changes (`admin-ui-api-wiring` slice 16 and a Settings section)

- Sign-in view: if `?sso=<id>` is present, call `start` and navigate; otherwise show a collapsed "Sign in with SSO" help text pointing to the
  Owner's link (no email box, so no account-existence oracle). Error banners map `sso_failed` and `sso_unavailable` to one static sentence each.
- Settings -> Single sign-on (Owner only): list/add/enable/disable/delete provider, rotate secret, copy the sign-in link, copy the
  redirect URI to register at the IdP, "Require SSO for non-Owners" switch with the break-glass explanation. Any member: "Link SSO" /
  "Unlink" (password + TOTP prompt).
- Invite dialog: "Sign-in method" (Password + authenticator | SSO) shown only for non-Owner roles when a provider is enabled; invite accept
  screen shows "Continue with SSO" for SSO invitations. The GitHub button stays dropped (G-13); AC-AUW-28 lifts only after slice 6.

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
| 1 | Migration 0041; pure core (PKCE, state/nonce, claim validator, URL/IP policy extracted from `dsn_policy`); `issue_session` extraction; mock IdP harness | M | none |
| 2 | Nullable `password_hash` in `signin`, `verify_credentials`, password reset; `admin_audit_events` writer; `remove_member` deletes links | S | 1 |
| 3 | `start` + discovery/JWKS caches + `callback` for intent `signin`; provider create/enable validation; `ServerKeys` for provider secret + `PUT .../secret` | L | 1, 2 |
| 4 | Invitation `auth_method='sso'` (create, lookup, callback intent `invite`) | M | 3, `account-bootstrap-and-invitations` (landed) |
| 5 | Self-link/unlink (intent `link`, re-auth) | S | 3 |
| 6 | `sso_required` + session fields + Owner exemption | S | 3 |
| 7 | SPA: sign-in start, Settings section, invite dialog/accept, banners | M | 3 (sign-in, Settings), 4-6 for their parts |

Slices 1-3 deliver working SSO for pre-linked users only if a link exists, so 4 or 5 is required before any real use; ship 1-5 together.
Total roughly 4-5 weeks of one engineer including the mock IdP.

## 9. Open decisions for the product owner

- **O1 IdP coverage:** generic OIDC (recommended) vs "Google + Microsoft only". Microsoft needs tenant-specific issuers and has an unverified
  `email` (safe here only because email is never a lookup key); Google works as is. Confirm which two IdPs we test against and document.
- **O2 JIT provisioning:** recommended none in V1; confirm, or approve a later domain-verified allowlist feature (DNS TXT proof, Viewer only).
- **O3 Owners and SSO:** Owners always keep password + TOTP and are exempt from `sso_required` (break-glass). Confirm, or accept a heavier
  alternative (break-glass Owner designated and checked).
- **O4 SSO-only members cannot delete the account or self-link.** Accept for V1, or require a fresh-login (`max_age=0`) SSO reauth.
- **O5 Audit:** table and writes are in V1; is an Owner-facing audit view required before launch, and what retention?
- **O6 Session lifetime:** SSO sessions use the same 24 h; should disabling/deleting a provider (or removing the IdP user) end SSO
  sessions (today it does not, as for passwords)? No back-channel logout in V1.
- **O7 Multi-account people:** `users.email` is globally unique and a user has one account, so the same IdP identity cannot belong to two
  accounts' consoles. Out of scope unless multi-account membership is planned.
- **SAML:** explicitly out; revisit on customer demand (separate ADR).

# admin-oidc-sso — Feature Delta (combined DISCUSS + DESIGN)

Replaces the current `GET /admin/v1/auth/oidc/callback` (audit #58: it trusts a client-supplied `id_token`, has a presence-only state
check, role lookup `'owner'` never matches, and mints a session for an arbitrary member). The orchestrator disables that route
separately; this feature implements the real flow. Not part of `admin-ui-api-wiring`: until it lands the console hides the Google/GitHub
buttons (gap G-13). Decision (user, final): authorization-code flow with PKCE + state/nonce, a subject→user link table, nullable
password for SSO-only users.

## 1. Problem

`oidc_providers` (per account: issuer, client_id, encrypted client_secret, enabled) can be configured but cannot be used to sign in:
there is no flow start, no code exchange, no stored state/nonce, and no way to bind an IdP identity to an embyr user.

## 2. Acceptance criteria

- AC-SSO-01: Given an enabled provider, When `GET /admin/v1/auth/oidc/start?provider_id=<uuid>`, Then 302 to the IdP authorization endpoint (from discovery) with `response_type=code`, `scope=openid email`, `code_challenge_method=S256`, `state`, `nonce`, `redirect_uri`; a single-use request row is stored and a short-lived `embyr_sso_state` cookie is set.
- AC-SSO-02: Given an unknown/disabled provider, Then 404; discovery failure → 502 page-safe redirect to `/admin/?error=sso_unavailable`.
- AC-SSO-03: Given a callback with an unknown, reused, expired (>10 min) or cookie-mismatched `state`, Then redirect `/admin/?error=sso_failed` and no session.
- AC-SSO-04: Given a valid callback, Then the code is exchanged server-side with the PKCE verifier and the client secret; the `id_token` signature (JWKS), `iss`, `aud`, `exp`, and `nonce` are all verified; any failure → `sso_failed`.
- AC-SSO-05: Given a verified token whose `(provider, sub)` is linked, Then a session cookie identical to password signin is issued and the browser is redirected to `/admin/`.
- AC-SSO-06: Given an unlinked `sub` with `email_verified = true` and an email matching an existing member of **the provider's own account**, Then the link is created and the session issued; otherwise `sso_failed` (no JIT account or membership creation).
- AC-SSO-07: Given an SSO-only user (NULL password), Then password signin returns the generic 401 and TOTP is not required for SSO sessions (the IdP is the second factor; stated trade-off).
- AC-SSO-08: Given a user removed from the account, Then existing links are deleted and SSO sign-in fails.
- AC-SSO-09: Given a provider is disabled or deleted, Then SSO sign-in through it fails immediately; existing sessions are unaffected (as today).
- AC-SSO-10: Given an Owner/Admin invitation accepted via password (separate feature), Then SSO later links by verified email (AC-SSO-06); inviting SSO-only users is out of scope.
- AC-SSO-11: Given tokens/codes/verifiers, Then none appear in logs or in the redirect URL back to the SPA.
- AC-SSO-12: Given the per-IP limiter, Then `start` and `callback` are rate-limited.

## 3. Schema and migration

```
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;      -- NULL = SSO-only; signin treats NULL as no match (constant-time dummy verify)

CREATE TABLE admin_sso_identities (
  provider_id UUID NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  subject     TEXT NOT NULL,
  user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (provider_id, subject)
);
CREATE INDEX admin_sso_identities_user_idx ON admin_sso_identities(user_id);

CREATE TABLE sso_auth_requests (
  state_hash         BYTEA PRIMARY KEY,         -- BLAKE3(state)
  provider_id        UUID NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  nonce              TEXT NOT NULL,
  code_verifier_enc  BYTEA NOT NULL,            -- AES-256-GCM under the server key
  browser_binding    BYTEA NOT NULL,            -- BLAKE3 of the embyr_sso_state cookie value
  expires_at         TIMESTAMPTZ NOT NULL       -- now() + 10 min
);
```
Request rows are deleted on first use (callback) or by a small purge on `expires_at`. Sign-in code reading `password_hash` must treat NULL
explicitly (an `Option`); this is the only change to existing signin. `users.password_hash NOT NULL` is currently relied on by the
members and invitation paths: bootstrap/invitation (separate feature) always sets a password, so no conflict.

## 4. Endpoints

| Route | Auth | Behaviour |
|---|---|---|
| `GET /admin/v1/auth/oidc/start?provider_id=` | public, rate-limited | discovery (cached, 1 h) → store request → 302 to IdP |
| `GET /admin/v1/auth/oidc/callback?code&state[&error]` | public, rate-limited | state+cookie check → token exchange (server-side) → id_token verify → link/lookup → session → 302 `/admin/` or `/admin/?error=sso_failed` |

Provider discovery: `{issuer}/.well-known/openid-configuration`; JWKS fetched from `jwks_uri` and cached with kid-miss refresh (reuse
`jsonwebtoken`, already a dependency, with RS256/ES256 allow-list; reject `none`/HS*). New config: `EMBYR_ADMIN_PUBLIC_URL` (builds the
redirect URI `…/admin/v1/auth/oidc/callback`; startup refuses SSO routes when unset). The SPA discovers SSO via a per-account sign-in
link `/admin/?sso=<provider_id>` shown in Settings → OIDC (avoids email-based account discovery and its enumeration problem); the
sign-in view starts the flow by navigating to `start` when `?sso=` is present.

## 5. Security considerations

PKCE S256 + `state` (CSRF) + `nonce` (replay) + cookie binding (login CSRF); redirect URI fixed server-side, never from the request;
all validation of the id_token done by the server, nothing trusted from the browser; `email_verified` required for linking and the match
is limited to members of the provider's own account (a provider an Owner registered cannot sign in into other accounts); algorithm
allow-list; discovery/JWKS fetches use a 5 s timeout, https-only, no redirects to other hosts; client secret decrypted with rotation, only
in memory. Session cookie attributes unchanged. SSO sessions bypass TOTP deliberately; an account that wants mandatory embyr TOTP
should not enable a provider (document, and show a note in Settings). Token endpoint errors are not echoed.

## 6. Slice plan

1. Migration; pure functions: PKCE verifier/challenge, state/nonce generation, id_token claim validation (iss/aud/exp/nonce/email_verified) in `embyr-core`. Tests: AC-SSO-04 (unit, all rejections).
2. `start` + request storage + discovery cache. Tests: AC-SSO-01, 02, 12.
3. `callback`: exchange, verify, link, session. Tests against an in-test mock IdP (own axum server with a generated RSA/EC key, like `tests/oauth_providers`): AC-SSO-03..09, 11.
4. Nullable password handling in signin. Tests: AC-SSO-07 plus the existing signin suite.
5. (UI, `admin-ui-api-wiring` slice 16) `?sso=` start, Settings link, error banners.

## 7. Test strategy

Pure unit/property tests for PKCE and claim validation. Acceptance against the real router with testcontainers Postgres and an in-process
mock OIDC IdP (discovery, JWKS, token endpoint) served by axum; fault injection: wrong nonce, replayed state, expired state, bad signature,
`alg=none`, unverified email, unlinked foreign email, JWKS unreachable, token endpoint 500. Contract tests against a real IdP are manual
and recommended before release (external integration: Google). CI only.

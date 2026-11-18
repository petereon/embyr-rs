# ADR-086: Admin-Console SSO via OpenID Connect — Authorization Code + PKCE, Link-Only Identity Binding

## Status

Proposed (design wave; product-owner decisions O1-O7 recorded 2026-11-18, see "Product decisions"; becomes Accepted when slice 3 merges). Closes the design half of audit #58 (the callback was disabled in PR #14;
this ADR defines its replacement). Depends on ADR-083 (`ServerKeys` encrypt/decrypt with rotation) and ADR-076 (sign-in rate limiter).
Feature detail, ACs, schema and slices: `docs/feature/admin-oidc-sso/feature-delta.md`.

## Context

- `oidc_providers` (migration 0013; per account: issuer, client_id, `client_secret_enc`, enabled) can be configured by an Owner, but
  nothing can sign in with it. `GET /admin/v1/auth/oidc/callback` is a stub that always redirects to `/admin/?error=oidc_failed`.
- The removed callback (#58) accepted a client-supplied `id_token`, fetched a JWKS from a URL built from the token's own `iss`, checked
  `state` for presence only, then picked a session user by a role string that never matched, falling back to the first member row.
  The failure class is **identity selected by a query that is not keyed on the verified subject**.
- Auth model today: `users` (global unique email, exactly one `account_id`), `account_members` (role Owner/Admin/Viewer), `sessions`
  (BLAKE3 token hash, 24 h, cookie `embyr_session; HttpOnly; Secure; SameSite=Strict; Path=/admin`), Argon2id password + mandatory
  TOTP, invitations with a token (accept = password + TOTP enrolment), password reset, per-IP sign-in limiter, `verify_credentials`
  shared with account deletion. No audit store exists. `remove_member` deletes only the `account_members` row and expires sessions;
  the `users` row survives.
- Hosted identity (end users of customer projects) is a separate system with its own keys and tables and is unaffected.

## Decision

1. **Scope.** SSO for admin-console users only. Providers stay **per account**, managed by that account's Owner; no per-instance
   provider. Generic OIDC with discovery; no provider-specific code. SAML and GitHub (not OIDC) are out.
2. **Flow.** Authorization code + PKCE (S256) + `state` + `nonce`. `POST /admin/v1/auth/oidc/start` (JSON, not a GET redirect, so
   invitation tokens and passwords never enter a URL) returns `{authorization_url}`; the SPA navigates to it. Start stores a single-use
   `sso_auth_requests` row (BLAKE3(state), nonce, code_verifier encrypted under the server key, intent, expiry 10 min) and sets
   `embyr_sso=<random>; HttpOnly; Secure; SameSite=Lax; Path=/admin/v1/auth/oidc; Max-Age=600`; the row stores BLAKE3 of that cookie.
   **Lax, not Strict:** the IdP returns by cross-site top-level GET, where Strict cookies are not sent. The callback deletes the row
   with `DELETE ... WHERE state_hash=$1 AND expires_at>now() RETURNING` as its first statement (single use even on failure), then
   requires the cookie to match the stored binding. `redirect_uri` is `{EMBYR_PUBLIC_BASE_URL}/admin/v1/auth/oidc/callback`, never taken
   from a request; start answers `sso_unavailable` when the variable is unset. The code goes only to the token endpoint.
3. **Provider resolution before any network fetch.** The provider row (by the state row's `provider_id`) supplies issuer and client_id.
   Discovery is `{issuer}/.well-known/openid-configuration`; its `issuer` must equal the stored issuer byte for byte; `jwks_uri` and the
   endpoints come from that document, never from a token. Outbound policy for discovery, token and JWKS requests, one shared helper
   extracted from `dsn_policy`: `https` only, no userinfo, resolve all addresses and refuse any in `is_forbidden_ip` (loopback, private,
   link-local, unique-local, v4-mapped), connect to the checked address (reqwest `resolve`, defeats DNS rebinding), no redirects, 5 s total
   timeout, body caps (discovery 64 KiB, token response 64 KiB, JWKS 256 KiB). `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true` lifts host and scheme
   rules (dev, compose, tests with the mock IdP), as for DSNs. Caches (in memory, per instance, bounded): discovery 1 h; JWKS 1 h, with
   at most one forced refetch per minute per provider on an unknown `kid`. The issuer is validated (https, policy, discovery succeeds) at
   provider create and at enable, so an Owner gets a 422 instead of a dead button.
4. **ID token validation** (`jsonwebtoken`, already a dependency; no `openidconnect` crate, whose oauth2 and url stack costs more than the
   ~300 lines it replaces). Algorithm allow-list `{RS256, ES256}`, and the key's type must match the header `alg` (rejects `none`, HS*,
   key confusion). Key chosen by `kid` from the cached JWKS. Required: `iss` == stored issuer; `aud` contains client_id (and `azp` ==
   client_id when `aud` has several); `exp` (60 s leeway); `iat` not more than 60 s in the future; `nonce` == stored; `sub` present,
   non-empty, at most 255 bytes. `email_verified` must be the JSON boolean `true` and `email` present, whenever the token is used to
   match an email (decision 5). Token-endpoint auth is `client_secret_basic`, or `_post` only when discovery does not advertise basic.
5. **Identity binding makes #58 structurally impossible.**
   - The session user is derived **only** from `admin_sso_identities` keyed `(provider_id, sub)` (provider is bound to one issuer, so
     this is `(issuer, sub)` per account). There is no fallback query, no "first member", no role-based pick. The link row carries
     `account_id`, and composite foreign keys to `oidc_providers(id, account_id)` and `users(id, account_id)` make a cross-account link
     unrepresentable. The callback additionally joins `account_members` (a removed member's `users` row survives), and `remove_member`
     deletes the member's links.
   - **Email is never used to find an existing user at sign-in.** Links are created in exactly two ways:
     (a) **Invitation** (`invitations.auth_method = 'sso'`): the invitee proves mailbox control with the invite token at start; the
     callback requires `email_verified = true` and IdP email equal (case-insensitive) to the invitation email, then creates the user
     with `password_hash NULL`, no TOTP, the member row and the link in one transaction. An invitation is the **only** way to add an
     SSO-only member. Owner invitations cannot be SSO-only.
     (b) **Self-link** of an existing password member from Settings, started with password + TOTP re-authentication (`verify_credentials`),
     so a stolen session cookie cannot plant a persistent second login. Intent and `user_id` live in the state row.
   - **No just-in-time account or member creation in V1.** If product later wants it: only with an Owner-managed allowlist of
     DNS-TXT-verified domains, `email_verified`, role Viewer, and a per-domain toggle. Not designed further here.
   - A provider's issuer and client_id are immutable (change = delete and recreate, which cascades its links away).
6. **TOTP and enforcement.** A completed IdP login **satisfies MFA**: the Owner chose the IdP and owns its second-factor policy;
   re-prompting for an embyr TOTP the SSO-only user cannot have (no secret) would force one, and a TOTP prompt after the IdP adds a
   phishable factor behind a stronger one. SSO sessions are tagged `sessions.auth_method='sso'`. Sensitive re-authentication
   (`DELETE /account`, self-link) stays password + TOTP, so SSO-only members cannot delete the account (accepted; revisit with a fresh-login
   reauth). `accounts.sso_required` (Owner toggle, requires an enabled provider and the caller to have a link) makes password sign-in
   return the generic 401 for every non-Owner. **Owners are always exempt: they keep password + TOTP by construction (Owners are never
   SSO-only), which is the break-glass path** and cannot be removed without a separate change. Password-reset request/confirm never sets
   a password on a user whose `password_hash IS NULL`, and is a no-op (generic success) for non-Owners while `sso_required`.
7. **Provider secret.** `client_secret_enc` is AES-256-GCM under `EMBYR_ENCRYPTION_KEY` via `ServerKeys` (current/previous rotation;
   a value that decrypts with the previous key is re-encrypted lazily, as `connection_secrets` does for DSNs); decrypted only in memory
   for the token call. `PUT /admin/v1/oidc_providers/:id/secret` (Owner) replaces it. Never returned, never logged. Replaces the
   hand-rolled AES in `oidc_providers.rs`.
8. **Session.** One shared `issue_session` (extracted from `signin` steps 7-8; `SessionOrigin::{Password, Sso{provider_id}}`) inserts the `sessions` row and builds the identical cookie;
   the callback answers `302 /admin/` with `Cache-Control: no-store` and `Referrer-Policy: no-referrer`, clears `embyr_sso`, and puts
   nothing in the URL. The static page needs no cookie; the SPA then calls `GET /auth/session`. **Sessions know their provider:**
   `sessions.auth_method` plus a nullable `sessions.sso_provider_id REFERENCES oidc_providers(id) ON DELETE CASCADE` (CHECK: set exactly
   for `sso`). Disabling a provider deletes its SSO sessions in the same transaction; deleting it removes them by cascade; password
   sessions and other providers' sessions are untouched; removing a member expires all their sessions and deletes their links. There
   is no back-channel logout and SSO sessions keep the 24 h lifetime (product decision O6).
9. **Abuse, errors, audit.** `start` and `callback` use `SigninRateLimiter` under a distinct key (`sso:{ip}`); `start` also caps live
   rows per provider (200) and opportunistically deletes expired rows (bounded) because it is unauthenticated and writes (cf. #14).
   Every failure after the state row is read, and every IdP-originated `error`, redirects to the same `/admin/?error=sso_failed`
   with a static message; unknown, disabled, deleted-account and discovery-failed providers all give `sso_unavailable` (the provider UUID
   is the only handle, so there is no account-exists oracle). Granular reasons go to a new `admin_audit_events` table
   (`sso_signin_ok`, `sso_signin_failed{reason}`, link created/removed, provider created/enabled/disabled/secret_rotated/deleted,
   `sso_required` toggled; actor, account, provider, source IP, time; no tokens or claims) and to an `embyr_sso_signin_total{result}`
   counter. A read API/UI for the audit table is not part of V1 and none is planned for launch; rows are retained 12 months by one extra
   DELETE in the existing `signin_rate_limit_sweeper` cycle (O5). The writer (`adapters::admin_audit::write_audit_event`) accepts
   labels only (`[a-z0-9_.:-]`, at most 64 characters) for event and reason, so a token, claim or e-mail address cannot be stored.

## Product decisions (2026-11-18)

Answers to the questions this ADR raised; the feature delta (section 9) carries the same list.

- **O1** Generic OIDC; test against Google plus one more IdP (to be named; Entra with a tenant-specific issuer recommended).
- **O2** No just-in-time provisioning in V1 (decision 5).
- **O3** Owners always keep password + TOTP and are exempt from `sso_required` (decision 6).
- **O4** SSO-only members cannot delete the account or self-link (decision 6).
- **O5** Audit rows written in V1; no owner-facing view at launch; 12-month retention (decision 9).
- **O6** Disabling or deleting a provider ends its SSO sessions; removing a member ends their sessions; no back-channel logout (decision 8).
- **O7** Multi-account membership out of scope.

## Alternatives Considered

- **Link by verified email at sign-in** (the previous draft's AC-SSO-06). Rejected: it makes the IdP's email claim an account-takeover
  primitive (mutable or unverified at some IdPs; Entra's `email` is a known case) and re-creates the #58 shape "find a user by something
  other than the verified subject". Invitation and authenticated self-link cover both real needs.
- **Per-instance (operator) providers.** Rejected for V1: needs a tenant-routing rule and an account-discovery oracle.
- **TOTP on top of SSO.** Rejected (decision 6); the opposite policy is available per account through `sso_required` plus the IdP's own MFA.
- **`openidconnect` crate.** Rejected on dependency weight and bundle policy; the validation surface is small and fully testable.
- **GET `start` with 302.** Rejected: invitation token or re-auth secrets would travel in a URL and logs.

## Consequences

- One migration (`0051_admin_sso.sql`, number verified) adds three tables, four columns (`invitations.auth_method`, `accounts.sso_required`,
  `sessions.auth_method`, `sessions.sso_provider_id`) and two composite-FK anchor unique indexes; `users.password_hash` becomes nullable. Every
  reader must treat NULL as "no password": `signin` (constant-time dummy verify, generic 401), `verify_credentials`, password reset.
  Old binaries decode NULL as an error, but no NULL exists until the feature code is deployed (expand-then-use ordering).
- Members who are SSO-only can use the console but not delete the account or self-link (they have no password to re-enter).
- Microsoft Entra is supported only with a **tenant-specific issuer** (`/common` returns a templated issuer that fails exact match), and
  its `email` claim is not verified, which is acceptable only because email is never the lookup key (it is checked against an invitation
  the invitee already holds). Confirmed by product (O1: generic OIDC, contract-checked against Google and one more IdP).
- Outbound fetches from an unauthenticated-triggered path exist (cached, bounded, SSRF-policed); a misconfigured corporate egress proxy
  makes SSO unavailable, not insecure.

## Enforcement

- Pure functions in `embyr-core::admin::sso` (PKCE, state/nonce, claim validation, outbound URL/IP policy) and `admin::audit` (labels) with
  exhaustive unit tests; no IO crates there. DNS resolution lives in `embyr-server::adapters::outbound_policy`; the DSN SSRF predicate
  `is_forbidden_ip` moved to core and is shared (`dsn_policy` re-exports it, behaviour unchanged). The SSO outbound check additionally refuses
  `0.0.0.0/8`, CGNAT `100.64.0.0/10`, broadcast and multicast.
- A source-scan test: the callback module contains no `SELECT ... FROM users` that is not joined from `admin_sso_identities`.
- Acceptance tests against the real router and an in-process mock IdP with fault injection (feature-delta section 7).

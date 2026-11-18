# Changelog

All notable changes to embyr-server are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Admin OIDC SSO enforcement (ADR-086, slice 6 of `admin-oidc-sso`): new `PATCH /admin/v1/account {sso_required}` (Owner only).
  Enabling needs an enabled provider on the account and a link of the calling Owner to it (else 422
  `sso_required_prerequisites`); disabling is immediate. While required, password sign-in for non-Owner members is the same
  generic 401 as a wrong password, `POST /members/invite` with `auth_method=password` for Admin/Viewer is 422, and password reset
  is a neutral no-op for them (reset tokens issued earlier stop working). Owners are always exempt and keep password + TOTP.
  Toggles are audited (`sso_required_toggled`, labels only). `GET /admin/v1/auth/session` now also returns `auth_method`
  (`password`|`sso`), `sso_linked` and `sso_required`. No migration, no new environment variables.
- Admin OIDC SSO invitations and self-link (ADR-086, slices 4-5 of `admin-oidc-sso`). `POST /admin/v1/members/invite` accepts
  `auth_method` (`password` default, or `sso`: 422 for an Owner invitation or when the account has no enabled provider; the
  response echoes it and the invite e-mail is worded for SSO). A password enrolment (`accept`/`confirm`) of an SSO invitation
  is refused like any dead token; new public `POST /admin/v1/auth/invitations/lookup {token}` returns `auth_method` (and, for
  `sso`, the account's enabled `sso_provider_ids`) so the SPA can branch. `POST /admin/v1/auth/oidc/start` gains intent
  `invite` (+ `invite_token`) and `link` (+ session cookie, `password`, `totp_code`, verified like account deletion); the
  callback, for `invite`, requires a verified e-mail equal (case-insensitively) to the invitation's and then creates the
  SSO-only user (no password, no TOTP), member, link and accepted invitation in one transaction before the session; for
  `link` it links `(provider, sub)` to the user who re-authenticated (a sub linked to anyone else fails) and answers
  `302 /admin/?sso_linked=1` without minting a session. `DELETE /admin/v1/auth/sso_identity[?provider_id=]` unlinks the
  caller's own identity (409 for an SSO-only user; ends their other sessions minted through it). New audit event
  `sso_invite_accepted`; invite/link failures are audited under their own event with a granular reason. The members list
  reports `auth_method: "sso"` for SSO-only members. No migration, no new environment variables.
- Admin OIDC SSO sign-in (ADR-086, slice 3 of `admin-oidc-sso`): `POST /admin/v1/auth/oidc/start` (JSON, returns the IdP
  authorization URL; authorization code + PKCE + `state` + `nonce`, `embyr_sso` browser-binding cookie, single-use state rows
  expiring in 10 minutes, 200 live rows per provider) and `GET /admin/v1/auth/oidc/callback`, which replaces the disabled stub:
  server-side code exchange, ID-token validation against the provider's own discovery/JWKS (RS256/ES256, exact issuer, aud/azp,
  nonce, clock), and a session only for the user linked to `(provider, sub)`, never by e-mail. Every failure is the same
  `302 /admin/?error=sso_failed`. Discovery/JWKS/token fetches are https-only, SSRF-policed, unredirected, 5 s and size-capped.
  Provider create and enable now validate the issuer (422 `issuer_invalid`); `PUT /admin/v1/oidc_providers/:id/secret` rotates
  the client secret (stored under the server key, never returned); provider lifecycle events are audited. Needs
  `EMBYR_PUBLIC_BASE_URL` (the console origin); without it SSO answers `sso_unavailable`. No new environment variables.
  Users can only sign in once linked (invitation and self-link arrive in slices 4-5).
- Admin OIDC SSO groundwork (ADR-086, slices 1-2 of `admin-oidc-sso`).
  Migration 0051: nullable `users.password_hash` (NULL = SSO-only user; `signin`, account-deletion re-auth and password
  reset treat it as "no password" with the usual generic responses), `admin_sso_identities`, `sso_auth_requests`,
  `admin_audit_events` (12-month retention, swept by the `signin_rate_limit_sweeper` cycle), `sessions.auth_method` and
  `sessions.sso_provider_id`. Removing a member now also deletes their SSO links; disabling an OIDC provider deletes the
  sessions minted through it (deleting it does so by cascade). Pure SSO core (PKCE, ID-token claim validation, outbound URL
  and IP policy) in `embyr-core`.
- TTL policies (go-live P5): `GET`/`PUT /admin/v1/projects/:id/ttl_policies` and `DELETE .../ttl_policies/:collection_group`
  (Admin+ to change, any role to list) configure a per-collection-group Timestamp field; a background `TtlSweeper`
  (`EMBYR_TTL_SWEEP_INTERVAL_SECS`, default 3600; `EMBYR_TTL_MAX_DELETES_PER_RUN`, default 10000 per policy per sweep)
  deletes expired documents through the normal commit path, so Listen subscribers see the removals. Subcollections
  survive; `agent`-mode projects are not supported. Migration 0050; metrics `embyr_ttl_deleted_total`,
  `embyr_ttl_sweep_errors_total`, `embyr_ttl_skipped_projects_total`, `embyr_ttl_last_run_timestamp_seconds`.
- Vector search (go-live P5): `StructuredQuery.find_nearest` on `RunQuery` (EUCLIDEAN, COSINE, DOT_PRODUCT,
  `limit` 1..=1000, `distance_threshold`, `distance_result_field`). Exact brute-force kNN over the filter's matches,
  no pgvector needed; capped at 20,000 scanned documents (`FailedPrecondition` beyond) and 2048 dimensions.
  Same rules/index/rate-limit gates as `RunQuery`. Rejected with `InvalidArgument` on `Listen`,
  `RunAggregationQuery`, `PartitionQuery` and when combined with `order_by`/`limit`/`offset`/cursors. Writes now
  reject vector values over 2048 dimensions.
- `PartitionQuery` RPC (go-live P5): split cursors on `__name__` for parallel reads, gated exactly like
  `RunQuery`; scan capped at 50,000 documents. RunQuery now honours `__name__` cursors and orders `__name__`
  by full document name (`collection_path`, `document_id`).
- Customer connection secrets now live in one server-key-encrypted store (ADR-083): migration
  0039 adds `agent_tls_bundle_srv_enc`; legacy ECIES rows convert lazily on their next
  authenticated request. New `GET`/`PUT /admin/v1/projects/:id/connection` (probe before save,
  mode switch); metrics `embyr_projects_unconverted_connection_secrets` and
  `embyr_dsn_convert_divergent_total`.
- Project API key rotation and revocation (ADR-085, slice 4): `POST /admin/v1/projects/:id/api_key/rotate`
  (old key keeps working for `EMBYR_API_KEY_ROTATION_WINDOW_SECS`, default 24 h) and `.../revoke` (old key
  dead at once), Owner/Admin, new key returned once. Refused with 409 while connection secrets are
  unconverted. Other replicas may serve a cached old key for up to 60 s longer; open Listen streams are not
  closed; a legacy hosted-identity signing key is regenerated (end-user tokens, 1 h, stop verifying).
  `GET /admin/v1/projects/:id` adds `api_key_rotated_at` / `api_key_previous_expires_at`.
- Customer DSNs submitted to provision / `PUT /connection` must require TLS and must not resolve
  to loopback, link-local, private or unspecified addresses (audit #65, X10); dev/test escape
  hatch `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true` (set in `docker-compose.yml`).
- SMTP delivery of member invites and password resets (`EMBYR_SMTP_URL` + `EMBYR_EMAIL_FROM`);
  `EMBYR_REQUIRE_SMTP=true` refuses startup when SMTP is unset.
- Bounded graceful shutdown: on SIGTERM `/healthz` turns 503, all three listeners stop
  accepting and drain for at most `EMBYR_SHUTDOWN_GRACE_SECS` (default 25).
- `EMBYR_SHUTDOWN_DELAY_SECS` (default 0, max 300): after SIGTERM `/healthz` turns 503 immediately
  but the listeners keep serving for this long before they stop accepting, so load balancers can
  deregister the instance first. Recommended 5-10 behind an LB.
- Container images published to ghcr.io: immutable `:sha-<short sha>` on every `master`
  build, `:vX.Y.Z` only from a matching `vX.Y.Z` git tag.
- Service-down, metrics-absent and restart-loop alert rules with runbook sections;
  error counters and alerting for the four background sweepers;
  `embyr_signin_rate_limit_pg_error_total` (finding #56).
- Operations docs: single-host deployment, secrets-manager setup, resource sizing, network
  topology, Grafana dashboard, incident runbook, root `README.md` and `.env.example`.
- Explicit request-body size ceilings on every listener (finding #24); JSON-structured logs
  (finding #25); panic guards at SQL-building call sites (finding #26).

### Changed
- `PATCH /admin/v1/projects/:id` no longer accepts `backend_pg_dsn` (422): use
  `PUT /admin/v1/projects/:id/connection`. Provision no longer writes the ECIES columns.
- `:vX.Y.Z` images are no longer overwritten by `master` merges; rollback is redeploying a
  previous immutable image instead of rebuilding from a git tag.
- Concurrent Argon2id hashing is bounded; vulnerable dependencies bumped.

### Fixed
- Type-aware cursor bounds for Timestamp/Boolean/Bytes/Reference pagination.
- `embyr-agent` registers its Postgres `LISTEN` before `subscribe()` returns.
- `embyr-db-prep` backfills `collection_id` on reprovisioning (finding #48).
- ECIES KDF domain separation (finding #33); resume-token and Stripe-webhook findings #31/#39;
  admin findings #58–#60.
- Shutdown no longer hangs until SIGKILL on an open gRPC Listen stream.

## [0.1.1] - 2026-09-15

### Added
- Versioning/tagging/CHANGELOG release process (finding #19). `embyr-server`'s startup log now
  reports its own version, sourced from `CARGO_PKG_VERSION`.
- `docker-compose.yml` for local/single-host evaluation — starts `embyr-server` and Postgres 15
  together via the existing root `Dockerfile`, with dev-only default credentials and a named
  volume for data persistence.
- `docs/operations/release-process.md` documenting the version-bump/git-tag/CHANGELOG convention.

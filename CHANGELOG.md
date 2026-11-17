# Changelog

All notable changes to embyr-server are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- TTL policies (go-live P5): `GET`/`PUT /admin/v1/projects/:id/ttl_policies` and `DELETE .../ttl_policies/:collection_group`
  (Admin+ to change, any role to list) configure a per-collection-group Timestamp field; a background `TtlSweeper`
  (`EMBYR_TTL_SWEEP_INTERVAL_SECS`, default 3600; `EMBYR_TTL_MAX_DELETES_PER_RUN`, default 10000 per policy per sweep)
  deletes expired documents through the normal commit path, so Listen subscribers see the removals. Subcollections
  survive; `agent`-mode projects are not supported. Migration 0050; metrics `embyr_ttl_deleted_total`,
  `embyr_ttl_sweep_errors_total`, `embyr_ttl_skipped_projects_total`, `embyr_ttl_last_run_timestamp_seconds`.
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

# dsn-single-store — Feature Delta (combined DISCUSS + DESIGN)

ADR: `docs/product/architecture/adr-083-dsn-single-store.md` (supersedes the ECIES-DSN parts of ADR-014 and closes the ADR-055 gap).
Prerequisite for the admin-UI DSN-edit and backend-mode-switch screens (`admin-ui-api-wiring` slice 07b).

## 1. Problem

Customer connection secrets live in two incompatible stores. Request serving reads `ecies_encrypted_dsn` and `agent_tls_bundle_enc`
(ECIES to a key derived from the project API key; `project_auth.rs`, `grpc/handler.rs` `authenticate`). The server cannot decrypt these
without the client's API key. Everything else (`TransactionSweeper`, `customer_db_connect`, `PATCH backend_pg_dsn`) uses
`backend_pg_dsn_enc` (AES-256-GCM under `EMBYR_ENCRYPTION_KEY`, with rotation). Consequences, all by grep: a DSN edit via `PATCH` is
written but never served; the sweeper skips every project provisioned through `provision` (ADR-055); the admin UI cannot edit a DSN,
switch backend mode, or probe a new DSN; the agent TLS bundle cannot be replaced.

**Decision (user, final):** one server-held store, encrypted under the server key. The ECIES store for connection secrets is retired.
Not in scope: `hosted_identity_signing_keys.private_key_enc`, which is also ECIES-to-API-key (`hosted_identity.rs`, `sign_up.rs`,
`sign_in_with_password.rs`) and keeps that scheme.

## 2. Acceptance criteria

- AC-DSS-01: Given a new `direct_pg` provision, Then only `backend_pg_dsn_enc` is written (`ecies_encrypted_dsn` stays NULL) and Firestore requests with the returned API key are served.
- AC-DSS-02: Given a new `agent` provision, Then the TLS bundle is stored only in `agent_tls_bundle_srv_enc` and requests are served.
- AC-DSS-03: Given a legacy row with only `ecies_encrypted_dsn`, When its first authenticated request succeeds, Then `backend_pg_dsn_enc` is populated with the same DSN, the request is unaffected, and the next request is served from the server store.
- AC-DSS-04: Given a legacy agent row, Then the same holds for `agent_tls_bundle_srv_enc`.
- AC-DSS-05: Given a row with both columns whose DSNs differ, When converted, Then the ECIES (currently served) DSN wins, a WARN and `embyr_dsn_convert_divergent_total` record it, and traffic behaviour does not change.
- AC-DSS-06: Given a conversion write fails or races with another request, Then the request still succeeds and the conversion is retried on a later request (idempotent `WHERE backend_pg_dsn_enc IS NULL`).
- AC-DSS-07: Given a converted or new `direct_pg` project, Then `TransactionSweeper` and index building resolve its DSN (the ADR-055 gap is closed for it).
- AC-DSS-08: Given `PUT /projects/:id/connection` with a DSN that cannot connect or whose schema is not ready, Then 400 (`backend_unavailable` / `customer_db_not_prepped` / `customer_db_schema_stale`) and the stored value is unchanged.
- AC-DSS-09: Given a probe-passing DSN, Then it is saved, the credential cache entry is evicted, and the next Firestore request connects to the new database.
- AC-DSS-10: Given a mode switch `direct_pg`↔`agent`, Then `backend_mode` and the new mode's secrets are written, the old mode's secrets are NULLed, and the cache is evicted, all in one transaction.
- AC-DSS-11: Given a Viewer, a foreign account, a suspended or deleted project, Then 403 / 403 / 409 / 404 and no write.
- AC-DSS-12: Given `GET /projects/:id/connection`, Then it returns mode and a masked summary and never a password, key, certificate or full DSN.
- AC-DSS-13: Given `EMBYR_ENCRYPTION_KEY` rotation (current + previous), Then both stored secrets decrypt with the previous key and are re-encrypted under the current key on next edit.
- AC-DSS-14: Given a soft-deleted project past grace, Then the purge nulls the new column as well.
- AC-DSS-15: The gauge `embyr_projects_unconverted_connection_secrets` counts non-deleted `direct_pg`/`agent` rows still lacking a server-store secret.

## 3. Schema and migration

Expand (this feature):
```
ALTER TABLE projects ADD COLUMN agent_tls_bundle_srv_enc BYTEA;   -- AES-256-GCM: nonce(12) || ct, same format as backend_pg_dsn_enc
-- backend_pg_dsn_enc already exists (migration 0015); no change.
```
Contract (separate, later migration, only when the gauge has been 0 for a release): `DROP COLUMN ecies_encrypted_dsn, agent_tls_bundle_enc`.
Until then the legacy columns are read-only fallbacks and are never written by new code. Formats cannot be mixed in one column (AES-GCM
and ECIES are not distinguishable by length), hence the new bundle column.

**Lazy convert-on-next-authenticated-request.** `authenticate` (gRPC) and `project_auth` (REST) already hold the plaintext API key
after the Argon2 check. Read order: server-store column → else legacy column decrypted with the API key (today's behaviour). After a
successful legacy decrypt and a successful backend connect, the handler spawns a best-effort task: encrypt under the server key and
`UPDATE … SET backend_pg_dsn_enc = $1 WHERE id = $2 AND backend_pg_dsn_enc IS NULL` (agent: same for the bundle column). The task
never blocks or fails the request. Rows with both columns populated follow AC-DSS-05.

**Sweeper meanwhile.** Unconverted rows stay unreachable exactly as ADR-055 accepts today (no regression); each is reached after its first
authenticated request. Idle projects never convert; their owners can force it by saving a connection through the new endpoint, and the
gauge (AC-DSS-15) makes the remainder visible. Purge (`soft_delete_purge_sweeper`) must null the new column (AC-DSS-14).

**Credential cache.** `CachedEntry{adapter, project_status, dsn}` is keyed by BLAKE3(api key) and holds the live adapter. The cache is
unchanged in shape; `evict_project` is called after every edit and mode switch (AC-DSS-09/10). Cache fill after conversion uses the
same decrypted DSN, so no behaviour change.

## 4. Endpoints (session auth, Owner/Admin to write, any role to read)

`GET /admin/v1/projects/:id/connection` → 200
`{ "backend_mode": "direct_pg|agent|aws_secret|gcp_secret", "summary": { "host": "db.prod.internal", "port": 5432, "database": "embyr_prod", "tls": true }  // direct_pg
   | { "endpoint": "10.0.0.5:9191", "cert_expires_at": "…" }  // agent
   | { "reference": "arn:…" }, "secret_stored": true, "converted": true }`
`host/port/database` are parsed from the decrypted DSN server-side; userinfo and query parameters are never emitted. Secret-manager modes
return the reference only (already non-secret).

`PUT /admin/v1/projects/:id/connection`
Request: `{ "backend_mode": "direct_pg", "dsn": "postgres://…" }` or
`{ "backend_mode": "agent", "agent_endpoint": "host:port", "agent_ca_pem": "…", "agent_client_cert_pem": "…", "agent_client_key_pem": "…" }`
Flow: role + ownership + status checks → validate shape → **probe** (`probe_customer_db` + `verify_schema_readiness`; agent: connect via
`AgentBackendAdapter::new`) → in one transaction write the new secrets, set `backend_mode`, NULL the other mode's secrets → evict cache.
Response 200 `{ "backend_mode": "…", "probe": "ok" }`; errors: 400 `{error: backend_unavailable | customer_db_not_prepped | customer_db_schema_stale | invalid_dsn | agent_*_required}`, 403, 404, 409 (project not active), 422.
`aws_secret`/`gcp_secret` projects: read-only here; switching away is allowed, switching to them is not offered (needs fetcher wiring, out of scope).
`PATCH /projects/:id` stops accepting `backend_pg_dsn` (422 pointing at the new route) so there is one write path.
`POST /projects` (provision) is changed per AC-DSS-01/02: no more `ecies::encrypt` of connection secrets; the API key is only hashed.

## 5. Security considerations

- Trust model changes: before, a DB dump plus `EMBYR_ENCRYPTION_KEY` could not reveal DSNs (client API keys are never stored); now it can. Accepted by the user. Mitigations already present: key from secret manager, dual-key rotation (ADR-018), non-root image. Document in the threat notes.
- Secrets never logged, never in responses, redacted `Debug` on request structs; the probe error text is sanitised (ADR-075), never echoing the DSN.
- SSRF/exfiltration: the probe makes the server connect to a user-supplied host. Same exposure as `provision` today; apply the same 5 s timeout and 2-connection pool, and reject non-`postgres(ql)://` schemes.
- Edit is Owner/Admin only and audit-logged (account, project, mode, actor — no secret).
- Probe-before-save prevents a typo from taking a production project offline.

## 6. Slice plan

1. Server-key read path + lazy convert (`project_auth.rs`, `grpc/handler.rs`, `system_db.rs` row struct) with divergence handling and gauge. Tests: AC-DSS-03..06, 15.
2. Provision writes server store only; purge column; `customer_db_connect` unchanged. Tests: AC-DSS-01, 02, 07, 14.
3. `GET`/`PUT /connection` incl. probe, mode switch, cache eviction; `PATCH` DSN removal. Tests: AC-DSS-08..13.
4. (later release) contract migration dropping legacy columns once the gauge is 0.

## 7. Test strategy

HTTP-level acceptance against the real admin router and gRPC handler with testcontainers Postgres (both system and customer DB), following
`tests/admin_api_v2`. Fault injections: unreachable DSN, schema-not-prepped customer DB, wrong key on decrypt, conversion race (two
concurrent first requests), row with both columns divergent, rotation window. Pure unit tests for DSN masking and the read-order function
(extracted as a pure fn over the row). The legacy-fixture rows are inserted directly with `ecies::encrypt`. CI only; not run on the laptop.

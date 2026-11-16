# api-key-rotation — Feature Delta (combined DISCUSS + DESIGN)

ADR: `docs/product/architecture/adr-085-api-key-rotation.md` (supersedes ADR-014 in full and the signing-key custody and enable-time `api_key` parts of ADR-036).
Closes go-live finding X2 (`docs/product/go-live-assessment-2026-11-13.md`).
Touches `admin-ui-api-wiring` slice 08 (the Keys tab). Depends on `dsn-single-store` slices 1-2 for slice 4 only.

## 1. Problem and evidence

A project's API key can be neither rotated nor revoked, and the product shows a key surface that does nothing. All facts verified against master `affb14c`.

1. **Data-plane auth reads only the two hashes on `projects`.** `get_project_for_auth` selects `api_key_hash_current` and `api_key_hash_previous` (`adapters/system_db.rs:266-272`). The gRPC path verifies current then previous (`grpc/handler.rs:265-275`); the REST path does the same (`adapters/project_auth.rs:112-117`). Two more sites verify the current hash only: `rest/sign_in_anonymously.rs:111` and `admin/handlers/hosted_identity.rs:114`. **Nothing writes `api_key_hash_previous`** (only `provision.rs` writes `api_key_hash_current`, at insert), so there is no way to change a key short of re-provisioning the project.
2. **SDK keys are decorative.** `sdk_api_keys` (migration 0012) is written by `admin/handlers/sdk_keys.rs` (create, list, revoke) and by `delete_project`'s cascade (`admin/handlers/lifecycle.rs:171-178`). No data-plane code reads it. A key minted there cannot authenticate; "revoking" one changes nothing. ADR-014 designed these routes to rotate the project key through a `RotateAuthKey` flow with ECIES re-encryption of the DSN; the implementation only inserts a BLAKE3 row.
3. **Re-keying breaks stored secrets.** Three stores are ECIES-encrypted to a key derived from the API key: the DSN and agent TLS bundle (`ecies_encrypted_dsn`, `agent_tls_bundle_enc`; read at `grpc/handler.rs:339,372`, `project_auth.rs:151`) and the hosted-identity signing seed (`hosted_identity_signing_keys.private_key_enc`; written `hosted_identity.rs:137-138`, read `rest/sign_up.rs:247`, `rest/sign_in_with_password.rs:175`). The server cannot decrypt any of them without the old API key, so a key change without migration bricks the project.
4. **Credential cache.** `CredentialCache` (`adapters/credential_cache.rs`) is keyed by (project, BLAKE3(api key)) and a hit skips verification (`handler.rs:236-246`). `evict_project` is process-local. A revoked key therefore keeps working from any cache that holds it.

Consequence: a leaked API key (shipped in a mobile app, pasted in a ticket, left in a former employee's laptop) cannot be contained short of deleting the database.

## 2. Scope

**In (decided, final):**
- One project API key, with rotation. `POST /admin/v1/projects/:project_id/api_key/rotate` and `.../revoke`.
- Rotate moves the current hash to `previous` with an expiry (default window 24 h, env-configurable). Revoke is rotate with a zero window: the old key is dead immediately.
- Verification ignores the previous hash after expiry. No sweeper.
- Compare-and-swap on the current hash: concurrent rotates yield 409.
- Remove the `sdk_keys` routes and their core helper; drop the cascade in `delete_project`; keep the table until a later contract migration.
- Repoint the admin SPA Keys tab at rotate/revoke, reusing the reveal-once modal.
- Hosted-identity signing key moves under `EMBYR_ENCRYPTION_KEY` for new rows; legacy ECIES rows are regenerated on first rotate.
- Audit as a structured `tracing` event (no audit store exists).

**Out of scope:**
- Multiple concurrent keys, per-key names, scopes, or per-key revocation (the removed SDK-key idea). Revisit only on a customer need.
- Per-call window override in the request body (env only; revisit if the UI needs it).
- Distributed cache invalidation. Cross-replica death of a revoked key is bounded by the credential-cache TTL (added by the parallel X6 PR) plus the window.
- Closing already-open Listen streams at rotation. They authenticated at open and stay open until the client closes them (documented, AC-AKR-15).
- Rotating anonymous/OAuth signing keys (already server-key encrypted, ADR-037/043), the admin key, or `EMBYR_ENCRYPTION_KEY`.
- Dropping `sdk_api_keys` and the legacy ECIES signing-key column (contract migration, a later release).
- `embyr-agent`: it never references the API key, so there is no BYOC impact.
- Converting legacy DSN rows: owned by `dsn-single-store`.

## 3. Schema and migration

Numbers are provisional: master's last migration is `0038`; the `dsn-single-store` and `account-bootstrap-and-invitations` branches are also taking low numbers, so whichever slice merges later renumbers.

Slice 1, expand (`0039_api_key_rotation.sql`):
```
ALTER TABLE projects ADD COLUMN api_key_previous_expires_at TIMESTAMPTZ;  -- NULL with a non-NULL previous hash = previous is dead (fail closed)
ALTER TABLE projects ADD COLUMN api_key_rotated_at          TIMESTAMPTZ;  -- display only; NULL = never rotated
```
Slice 2, expand (`0040_hosted_signing_key_srv_enc.sql`):
```
ALTER TABLE hosted_identity_signing_keys ADD COLUMN private_key_srv_enc BYTEA;           -- AES-256-GCM: nonce(12) || ct, EMBYR_ENCRYPTION_KEY, same format as anonymous_signing_keys.private_key_enc
ALTER TABLE hosted_identity_signing_keys ALTER COLUMN private_key_enc DROP NOT NULL;
ALTER TABLE hosted_identity_signing_keys ADD CONSTRAINT hisk_one_seed CHECK (private_key_enc IS NOT NULL OR private_key_srv_enc IS NOT NULL);
```
Contract (later, separate): `DROP TABLE sdk_api_keys`; `DROP COLUMN hosted_identity_signing_keys.private_key_enc` once no row has it.

**Live-previous predicate, computed in SQL (one time source, no replica clock skew).** `get_project_for_auth` returns the previous hash only when it is live:
```
CASE WHEN api_key_previous_expires_at > now() THEN api_key_hash_previous END AS api_key_hash_previous
```
`ProjectAuthRow.api_key_hash_previous` keeps its type and meaning ("previous hash that may still authenticate"). The existing gRPC and REST verify sites then need no logic change. The two current-only sites move to the same helper (§6 slice 1). The three copies of "current, else previous" collapse into one pure function `embyr_core::auth::argon2::verify_project_key(key, current, previous_live) -> bool`.

**Hosted signing-key read order** (sign_up, sign_in_with_password): `private_key_srv_enc` decrypted with `decrypt_with_rotation` if present, else legacy `private_key_enc` ECIES-decrypted with the request's API key (today's behaviour). A project whose row is still legacy keeps working until its first rotate.

## 4. Endpoints

Both: session auth, `session.role >= Role::Admin` (Owner or Admin; Viewer 403), `verify_project_ownership` (404 deleted/missing, 403 foreign account). Allowed on `active` and `suspended` projects (containing a leaked key must not wait on billing). No request body.

`POST /admin/v1/projects/:project_id/api_key/rotate` -> 200, header `Cache-Control: no-store`
```
{ "api_key": "<43 chars, shown once>", "rotated_at": "…", "previous_key_expires_at": "…" }
```
`POST /admin/v1/projects/:project_id/api_key/revoke` -> 200, same shape with `"previous_key_expires_at": null`. Revoke also returns a new key (the project must keep a usable key); the old key is rejected immediately.

Errors: 403, 404, 409 `{ "error": "rotation_conflict" }` (CAS lost), 409 `{ "error": "connection_secrets_unconverted" }` (§5 step 2), 500 sanitised (ADR-075).

`GET /admin/v1/projects/:project_id` (existing `get_project.rs`) additionally returns `api_key_rotated_at` and `api_key_previous_expires_at` (nullable); never a hash. This feeds the Keys tab.

Config: `EMBYR_API_KEY_ROTATION_WINDOW_SECS`, default `86400`, valid `1..=2592000` (30 d); anything else fails startup like the other typed env vars in `config.rs`.

Removed: `GET|POST /admin/v1/projects/:project_id/sdk_keys`, `DELETE .../sdk_keys/:key_id` (`admin/router.rs:237-244`), `admin/handlers/sdk_keys.rs`, `embyr-core/src/admin/sdk_key.rs` and its `mod.rs:11` line, the cascade in `lifecycle.rs::delete_project`. Removed routes answer 404.

## 5. Rotation flow and state transitions

Per project, the key lifecycle is: `K0 current` -> rotate -> `K1 current, K0 previous until T` -> (T passes: `K1 current`, `K0` dead by predicate) or (revoke or second rotate: previous replaced or cleared).

Flow of `rotate_or_revoke(project_id, window)` (window = configured value for rotate, 0 for revoke):
1. Role, ownership. Read the row: `api_key_hash_current` (the observed value), `status`, `backend_mode`, connection columns.
2. **Unconverted guard.** `direct_pg` with `backend_pg_dsn_enc IS NULL`, or `agent` with `agent_tls_bundle_srv_enc IS NULL` -> 409 `connection_secrets_unconverted`, nothing written. This is the exact "unconverted" predicate of `dsn-single-store` AC-DSS-15 and must reuse its SQL helper. Without it, rotating bricks the project because the ECIES secrets were encrypted to the old key. `aws_secret`/`gcp_secret` rows hold no ECIES secrets and pass.
3. Generate the new key (32 random bytes, base64url without padding, as `provision.rs:189-191`); Argon2id hash on `spawn_blocking`.
4. One transaction:
   ```
   UPDATE projects SET
     api_key_hash_previous       = CASE WHEN $window > 0 THEN api_key_hash_current END,
     api_key_previous_expires_at = CASE WHEN $window > 0 THEN now() + make_interval(secs => $window) END,
     api_key_hash_current = $new_hash, api_key_rotated_at = now(), updated_at = now()
   WHERE id = $1 AND api_key_hash_current = $observed
     AND status IN ('active','suspended') AND <unconverted guard repeated in SQL>
   ```
   0 rows -> re-read to classify: hash changed -> 409 `rotation_conflict`; guard failed -> 409 `connection_secrets_unconverted`; gone -> 404.
   Same transaction: if the project has a `hosted_identity_signing_keys` row with `private_key_srv_enc IS NULL`, replace it with a fresh Ed25519 key (new `public_key`, `private_key_srv_enc` set, `private_key_enc` NULL). Rollback restores the old row.
5. Commit, then `credential_cache.evict_project(project_id)`, then emit the audit event. A second rotate while a window is open overwrites `previous`: the key from two rotations ago dies at once (intended for emergencies; at most one previous key exists).

Audit event (no audit store exists, so this is a `tracing` event the log pipeline retains): `tracing::info!(target: "embyr::audit", event = "api_key_rotated" | "api_key_revoked", project_id, account_id = session.account_id, session_id = session.session_id, window_secs, previous_expires_at)`. Never the key, never a hash. Fixed field set so a log alert can key on `event`.

**Cache semantics.** Eviction is local. Other replicas keep serving a cached old key until their entry expires, so the old key's effective lifetime is `window + cache TTL` for rotate and `cache TTL` for revoke, and the guarantee is only as good as the TTL from the parallel X6 PR. A cached entry also outlives the window by up to the TTL. This is the same bound X6 accepts for suspension; the endpoint docs and the UI confirm text state it.

## 6. Slice plan

1. **Two-hash verify with expiry** (migration 0039, `system_db.rs` query + `ProjectAuthRow`, `verify_project_key`, call sites `grpc/handler.rs`, `project_auth.rs`, `sign_in_anonymously.rs`, `hosted_identity.rs`). Behaviour-neutral until something writes `previous`. No dependency. Tests: AC-AKR-02, 03, 20.
2. **Hosted signing key under server key** (migration 0040, `enable_hosted_identity` writes `private_key_srv_enc`, `api_key` in the enable body becomes optional and ignored, sign_up / sign_in_with_password read order). No dependency. Tests: AC-AKR-13, 14 (first half).
3. **Remove sdk_keys + SPA Keys tab.** Delete routes, handler, core helper, cascade; Keys tab becomes a "Project API key" panel (last rotated, previous-key expiry, Rotate, Revoke now with confirm), reuse `SecretRevealed` modal. SPA calls land when slice 4 exists; until then the panel is read-only (renders `api_key_rotated_at`). No server dependency for the removal. Tests: AC-AKR-16, 17, 19.
4. **Rotate/revoke endpoints** (config var, handler, shared SQL helper, cache evict, audit event, `get_project` fields, signing-key regeneration). **Requires `dsn-single-store` slices 1-2 merged and deployed first** (new provision writes only server-held secrets and legacy rows can convert) and slice 1 of this feature fully rolled out; otherwise rotating a `direct_pg` or `agent` project whose DSN is ECIES-encrypted to the old key bricks it. The 409 guard is the runtime backstop, not a substitute for the ordering. Tests: AC-AKR-01, 04..12, 14 (second half), 15, 18.

Slices 1, 2, 3 are independent and may land in any order or in parallel. `admin-ui-api-wiring` slice 08 is re-scoped here (its table rows 81-83 and slice row now point to this document).

## 7. Acceptance criteria

Verify sites below means: gRPC `authenticate`, REST `resolve_customer_db_adapter`, `signInAnonymously`, and `enable_hosted_identity`.

- AC-AKR-01: Given an Owner/Admin and project key K0, When `rotate` is called, Then 200 returns a new key K1 once, K1 authenticates gRPC and REST, and `previous_key_expires_at` is now + the configured window.
- AC-AKR-02: Given a rotation whose window is still open, Then K0 and K1 both authenticate at every verify site.
- AC-AKR-03: Given the window has passed (fixture sets `api_key_previous_expires_at` into the past), Then K0 is rejected at every verify site with the same error as a wrong key, and K1 still works. No background job ran.
- AC-AKR-04: Given a non-NULL `api_key_hash_previous` with NULL `api_key_previous_expires_at`, Then the previous hash is ignored (fail closed).
- AC-AKR-05: Given `revoke`, Then 200 returns a new key, the old key is rejected immediately on the serving replica even if it was cached, and `previous` columns are NULL.
- AC-AKR-06: Given K1 current and K0 previous in an open window, When rotate runs again, Then K0 is rejected at once, K1 becomes previous with a fresh window, K2 is current.
- AC-AKR-07: Given two or more concurrent rotates (barrier-started, N=2 and N=8), Then exactly one returns 200 and the rest return 409 `rotation_conflict`; the stored current hash verifies the winner's key and no loser's key ever authenticates.
- AC-AKR-08: Given a `direct_pg` project with `backend_pg_dsn_enc` NULL (legacy ECIES DSN), or an `agent` project with `agent_tls_bundle_srv_enc` NULL, When rotate or revoke is called, Then 409 `connection_secrets_unconverted`, no column changes, K0 still serves requests.
- AC-AKR-09: Given a converted `direct_pg` project (and an `aws_secret` one), When rotated, Then K1 serves a Firestore request end to end through the server-held DSN.
- AC-AKR-10: Given K0 is cached on this replica, When rotate is called, Then the entry is evicted and the next K0 request re-verifies via the previous hash (window open) or fails (revoke).
- AC-AKR-11: Given a replica whose cache still holds K0 (simulated by a second `CredentialCache` populated before the rotate), Then K0 is served from that cache at most until its TTL and is rejected after; the test asserts the documented bound, not instant death.
- AC-AKR-12: Given a Viewer, a foreign account, a missing or deleted project, or no session, Then 403, 403, 404, 401 and no write; a suspended project is rotatable. Each success emits exactly one `embyr::audit` event with the fixed fields; the response has `Cache-Control: no-store`; neither logs nor the audit event contain the key or a hash.
- AC-AKR-13: Given hosted identity is enabled on a new project, Then only `private_key_srv_enc` is written, the enable request needs no `api_key`, and sign-up and sign-in mint verifiable tokens, also after a rotation with the new key.
- AC-AKR-14: Given a legacy row (ECIES `private_key_enc` only, inserted by fixture), Then sign-in works with K0 before rotation; after rotate the row has a new `public_key`, `private_key_srv_enc` set and `private_key_enc` NULL, sign-in works with K1, and a token minted before the rotate no longer verifies (documented, 1 h lifetime). If the rotate transaction fails after the update, the old row and old key are unchanged.
- AC-AKR-15: Given a Listen stream opened with K0 before a revoke, Then it continues until closed (documented); a new Listen with K0 is rejected.
- AC-AKR-16: Given the `sdk_keys` routes, Then GET/POST/DELETE return 404; `delete_project` no longer references `sdk_api_keys`; the table and its rows still exist.
- AC-AKR-17: Given the Keys tab, When Rotate or Revoke is confirmed and returns 2xx, Then the new key appears once in the reveal modal and is absent from the model after close, navigation and sign-out (AC-AUW-20); on 403/409 a toast explains and the model is unchanged (AC-AUW-19); Viewer sees the buttons disabled.
- AC-AKR-18: Given `EMBYR_API_KEY_ROTATION_WINDOW_SECS` unset, Then 86400; set to `0`, negative, non-numeric or above 2592000, Then startup fails with a message naming the variable.
- AC-AKR-19: Given `GET /projects/:id`, Then it returns `api_key_rotated_at` and `api_key_previous_expires_at` and never a hash.
- AC-AKR-20: Given a project that has never been rotated (new columns NULL), Then authentication behaves exactly as before the migration at every verify site.

## 8. Test strategy

- **Pure unit:** `verify_project_key` table-driven (current ok, previous ok, both wrong, previous absent), mutation-tested per feature. Config parsing of the window variable.
- **Acceptance (HTTP-level)** against the real admin router and gRPC handler with testcontainers Postgres, following `tests/admin_api_v2`. Expiry is exercised by writing `api_key_previous_expires_at` into the past, never by sleeping, which works because the live predicate is evaluated by Postgres `now()`.
- **Fault injections:** concurrent rotates; unconverted DSN row (fixture inserted with `ecies::encrypt`); transaction failure after the key update (forced error between the two statements) to prove rollback; second cache instance for cross-replica staleness; legacy ECIES hosted row; both-key window with an in-flight Listen.
- **Cache TTL tests (AC-AKR-11)** depend on the TTL from the X6 PR; if it has not merged, the test is marked as pending on it and the endpoint doc states the weaker bound (entry lives until LRU eviction or restart).
- **SPA:** update-function tests for the Rotate/Revoke messages (reveal-once, no secret in model after close), as for the other slice-08 flows.
- CI only; never run whole testcontainers-backed targets on the laptop.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Cross-replica cache keeps a revoked key alive | Documented bound (TTL, plus window for rotate); depends on the X6 TTL PR; release note before advertising "revoke". |
| Rotating before `dsn-single-store` bricks ECIES-encrypted projects | Slice 4 ordered after it; runtime 409 guard (AC-AKR-08); guard predicate shared with the AC-DSS-15 gauge. |
| Rolling deploy: a replica without slice 1 still honours `previous` without expiry | Slice 1 ships and rolls out fully before slice 4 is enabled; no `previous` value exists until slice 4. Rolling the binary back past slice 1 after a rotation would re-honour the previous key; treat as a rollback precondition. |
| Regenerating the hosted signing key invalidates end-user tokens (1 h) and changes the JWKS key | Accepted by the user; only first rotate of legacy rows; stated in the confirm text. |
| Customers cannot redeploy a mobile app inside 24 h | Window is env-configurable up to 30 d; the response states the expiry; a customer-facing doc recommends rotate-then-ship-then-wait. |
| Two Argon2 verifications for wrong keys while a window is open | Bounded by the existing Argon2 gate and pre-auth limiter; valid keys hit the cache. |
| `dsn-single-store` divergent-row semantics (AC-DSS-05) could make "converted" ambiguous | The guard reuses its definition verbatim; any change there changes this guard with it. |
| Removing `sdk_keys` breaks an unknown caller | No data-plane reader exists; the only consumer is the admin SPA, repointed in slice 3; table retained for rollback. |

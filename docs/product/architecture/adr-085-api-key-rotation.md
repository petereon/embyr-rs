# ADR-085: Project API Key Rotation and Revocation — One Key, Two-Hash Window, Server-Held Secrets

## Status

Proposed (design wave; becomes Accepted when the feature's slice 1 merges). **Supersedes ADR-014** entirely (SDK keys as a rotation
vehicle: `new_sdk_key_material`, `api_key_hash_secondary`, ECIES re-encryption of the DSN on rotation; the routes were never wired to
the data plane). **Supersedes ADR-036** in two parts: Decision 2's custody of the hosted-identity signing seed (ECIES to the API key)
for new rows, and Decision 5's "gap found" resolution (the enable request carrying the raw `api_key`), which existed only to derive
that ECIES recipient. Depends on ADR-083 (DSN and agent TLS bundle under the server key). Closes go-live finding X2.

## Context

- Data-plane auth reads only `projects.api_key_hash_current` / `api_key_hash_previous` (`adapters/system_db.rs:266-272`,
  `grpc/handler.rs:265-275`, `adapters/project_auth.rs:112-117`; two further sites check current only:
  `rest/sign_in_anonymously.rs:111`, `admin/handlers/hosted_identity.rs:114`). Nothing writes `api_key_hash_previous`, so no project
  key can ever change.
- `sdk_api_keys` (migration 0012) is written by `admin/handlers/sdk_keys.rs` and `lifecycle.rs` and read by nothing on the data
  plane: its keys cannot authenticate and revoking one is a no-op.
- Three secrets are ECIES-encrypted to the API key: DSN, agent TLS bundle, hosted-identity signing seed. Changing the key strands
  them. ADR-083 removes the first two from that scheme; the signing seed (`hosted_identity.rs:137`, `sign_up.rs:247`,
  `sign_in_with_password.rs:175`) is the remaining one. Its sibling tables `anonymous_signing_keys` and `oauth_signing_keys` are
  already AES-256-GCM under `EMBYR_ENCRYPTION_KEY`, so the pattern is established.
- `CredentialCache` (per process, keyed by project and BLAKE3(key)) skips verification on a hit; `evict_project` is local.
- No audit store exists in the codebase.

## Decision

1. **One project API key, with rotation.** `POST /admin/v1/projects/:project_id/api_key/rotate` generates a key (same generator as
   `provision`), returns it once, moves the current hash to `api_key_hash_previous` and stamps `api_key_previous_expires_at =
   now() + window`. `.../revoke` is the same operation with a zero window: `previous` is cleared and the old key dies at once.
   Owner/Admin only; allowed on `active` and `suspended` projects.
2. **Expiry is evaluated by the reader, in SQL, with no sweeper.** `get_project_for_auth` returns the previous hash only while
   `api_key_previous_expires_at > now()`; a NULL expiry means dead (fail closed). Every verify site uses one pure function
   `verify_project_key(key, current, previous_live)`. Default window 24 h (`EMBYR_API_KEY_ROTATION_WINDOW_SECS`, 1 s to 30 d). One
   previous key at most: a second rotate overwrites it.
3. **Compare-and-swap.** The `UPDATE` is conditioned on the current hash observed at the start of the request; a lost race returns
   409 `rotation_conflict`. No row lock, no retry.
4. **Rotation requires server-held connection secrets.** A `direct_pg` row without `backend_pg_dsn_enc`, or an `agent` row without
   `agent_tls_bundle_srv_enc`, returns 409 `connection_secrets_unconverted` (the ADR-083 AC-DSS-15 predicate, shared). This feature's
   endpoint slice therefore ships after `dsn-single-store` slices 1-2.
5. **Hosted signing seed moves under the server key.** New column `private_key_srv_enc` (AES-256-GCM, rotation-aware via
   `decrypt_with_rotation`); `private_key_enc` becomes nullable; new rows write only the server column and enable no longer needs the
   API key. Legacy ECIES rows keep working until the project's first rotate, which, in the same transaction, regenerates the keypair
   (the seed is undecryptable once the key changes). End-user tokens (1 h) minted under the old key stop verifying.
6. **Remove SDK keys.** Delete the three `sdk_keys` routes, their handler and `embyr-core::admin::sdk_key`, and the cascade in
   `delete_project`. Keep the table until a later contract migration. The admin SPA Keys tab becomes a Project API key panel
   (rotate, revoke now, last-rotated and previous-expiry display) reusing the reveal-once modal.
7. **Audit** is a structured `tracing` event, target `embyr::audit`, fixed fields (event, project, account, session, window,
   expiry), never key material. A durable audit store is a separate decision.
8. **Cache.** Evict the project on commit. Cross-replica death of the old key is bounded by the credential-cache TTL (parallel X6
   change) plus, for rotate, the window. Open Listen streams are not terminated.

## Alternatives Considered

1. **Keep multiple SDK keys and make the data plane read `sdk_api_keys`.** Rejected: a second credential type with per-key
   revocation needs a lookup keyed by hash (Argon2 is not indexable, the table uses BLAKE3), a cache rework and a new key format on
   every SDK, for a need nobody has stated. One key with a window covers the leaked-key case. Revisit on demand.
2. **Sweeper clears expired `previous`.** Rejected: a second moving part with a failure mode (sweeper down means a dead key lives);
   evaluating expiry at read time is one `CASE` and cannot be late.
3. **Replica-local clock for expiry.** Rejected in favour of SQL `now()`: one time source, no skew between replicas.
4. **Row lock (`SELECT ... FOR UPDATE`) instead of CAS.** Rejected: serializes rotates but lets the second one succeed silently,
   issuing a second key the first caller never saw; 409 is the better signal.
5. **Re-encrypt the hosted seed on rotate by asking the admin for the old key.** Rejected: puts the secret on an admin request, which
   ADR-036 spent a decision avoiding; regeneration costs one hour of end-user tokens.
6. **Convert hosted seeds lazily on the next request (as ADR-083 does for DSNs), avoiding regeneration.** Viable and cheaper for end
   users, but adds a write path to two hot REST handlers for a rare event. Deferred; slice 2 does not preclude it.
7. **Distributed cache invalidation (NOTIFY or pub/sub) so revoke is instant everywhere.** Rejected for now: TTL already covers
   suspension and DSN edits (X6); one mechanism, one bound.
8. **Rotate on any project regardless of conversion state.** Rejected: bricks ECIES-encrypted projects permanently.

## Consequences

Positive: a leaked key can be contained without re-provisioning; one verify function replaces three copies and fixes two sites that
ignored `previous`; the last ECIES-to-API-key secret leaves the signing path; a dead UI surface and dead routes disappear.

Negative / accepted: revoke is bounded by the cache TTL on other replicas, not instant; the hosted signing key's confidentiality now
also rests on `EMBYR_ENCRYPTION_KEY` (a dump plus that key can mint end-user tokens, as for anonymous and OAuth keys); first rotate of
a legacy hosted-identity project logs end users out once; during a window a wrong key costs two Argon2 verifications; clients that
cannot redeploy inside the window must choose a longer one globally (no per-call override). Rolling the binary back past slice 1 after a
rotation would honour the previous key without expiry.

## Enforcement

Pure `verify_project_key` unit tests and per-feature mutation testing; acceptance tests per feature-delta AC-AKR-01..20 including
concurrent rotates, unconverted-DSN 409, expired previous (fixture-driven, no sleeps), second-cache staleness, rollback of the
signing-key regeneration, and 404 on removed routes; a source scan that no non-test code calls `ecies::encrypt` once ADR-083 and slice 2 have landed; a test that audit events and responses carry no key or hash.

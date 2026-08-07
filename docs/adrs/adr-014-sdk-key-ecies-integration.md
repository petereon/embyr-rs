# ADR-014: SDK Key Creation via RotateAuthKey — ECIES Integration Path

## Status

Accepted

## Context

DISCUSS locked decision D4: "The existing ECIES scheme in `embyr-core` links an API key to an ECIES private key derived from `HKDF(api_key, project_id)`. SDK keys created via the UI must go through the same `RotateAuthKey` aggregate command to register the hash correctly. UI key creation cannot bypass BC-1."

Existing `provision.rs` performs the following for `backend_mode=direct_pg`:
1. Generate 32 random bytes → `api_key` (base64url encoded)
2. Argon2id hash → stored as `api_key_hash_current` in `projects`
3. ECIES pubkey derived from `api_key` via `ecies::derive_public_key(api_key.as_bytes())`
4. DSN encrypted with the pubkey → stored as `ecies_encrypted_dsn`

The SDK key creation flow (`POST /admin/v1/projects/:id/sdk_keys`) must:
1. Generate a new `embyr_sdk_<base64url(32_bytes)>` format key
2. Store `BLAKE3(key)` in `sdk_api_keys.key_hash` (for listing/revoking)
3. Update `projects.api_key_hash_current` with `Argon2id(new_key)` (for Firestore SDK authentication)
4. Promote old `api_key_hash_current` to `api_key_hash_secondary` (dual-hash rotation window)
5. If `backend_mode=direct_pg`: re-encrypt the DSN with the new key's ECIES pubkey
6. Evict the credential cache entry for this project

The question is where this logic lives and how to avoid duplicating the crypto from `provision.rs`.

Two options were analyzed:
1. **New domain function in `embyr-core::domain::project`** — a pure function that takes raw key bytes and returns the computed hashes and pubkey. Handlers use this function.
2. **Duplicate crypto in the SDK key handler** — copy the Argon2id + ECIES pattern from `provision.rs` into `sdk_keys.rs`.

## Decision

**New pure domain function `new_sdk_key_material` in `embyr-core::domain::project` (Option 1).**

The function computes all cryptographic materials from raw key bytes without performing any IO:

```rust
/// Inputs: raw random key bytes (32 bytes, NOT yet formatted with prefix)
/// Output: all derived values needed to persist an SDK key + update the project
pub struct SdkKeyMaterial {
    /// Argon2id hash of the raw key — stored in projects.api_key_hash_current
    pub argon2id_hash: Vec<u8>,
    /// ECIES X25519 public key derived from raw key via HKDF — used to re-encrypt DSN
    pub ecies_pubkey: [u8; 32],
    /// BLAKE3 hash of the raw key — stored in sdk_api_keys.key_hash for lookup/audit
    pub blake3_hash: [u8; 32],
}

pub fn new_sdk_key_material(raw_key: &[u8]) -> Result<SdkKeyMaterial, CoreError>
```

This function combines `argon2::hash_api_key`, `ecies::derive_public_key`, and `blake3::derive_cache_key` (repurposed: BLAKE3 hash for storage, not just cache). It must be called on a `tokio::task::spawn_blocking` thread because Argon2id is CPU-intensive (~200–500ms).

**Handler flow for `POST /admin/v1/projects/:id/sdk_keys`:**

```
1. Extract SessionContext (via extractor) → verify role ≥ Admin (check_rbac)
2. Verify project belongs to session.account_id → 403 if not
3. Load project record: backend_mode, current ecies_encrypted_dsn
4. Generate raw 32 random bytes via OsRng
5. Format key string: "embyr_sdk_" + base64url(raw_key_bytes)
6. spawn_blocking: SdkKeyMaterial::new(raw_key_bytes)  ← Argon2id on blocking thread
7. BEGIN transaction:
   a. INSERT INTO sdk_api_keys (project_id, name, key_hash, prefix, created_at)
      VALUES ($project_id, $name, $blake3_hash, first_8_chars(key_string), now())
   b. UPDATE projects SET
        api_key_hash_secondary = api_key_hash_current,   -- rotation window
        api_key_hash_current = $argon2id_hash,
        [IF backend_mode=direct_pg]:
          ecies_encrypted_dsn = ecies::encrypt(&material.ecies_pubkey, &current_plaintext_dsn)
      WHERE id = $project_id AND account_id = $account_id
   COMMIT
8. credential_cache.evict_project(project_id) -- fire-and-forget
9. Return 201: { id, name, key: key_string, prefix, created_at }
   -- key_string ONLY returned here; not accessible in any subsequent GET
```

**DSN re-encryption challenge (backend_mode=direct_pg only):** Step 7b requires the plaintext DSN to re-encrypt with the new ECIES pubkey. The current DSN is stored as `ecies_encrypted_dsn` — encrypted with the OLD API key's ECIES private key. To re-encrypt, the old key is needed. 

The old API key is NOT stored anywhere in the system (by design — only its Argon2id hash is stored). Therefore the re-encryption path requires the old plaintext key.

**Resolution:** The `sdk_api_keys` table stores an additional column `ecies_encrypted_dsn_snapshot: BYTEA` — a copy of the project's `ecies_encrypted_dsn` at the time the key was created, encrypted with that key's ECIES pubkey. When a new SDK key is created:
- The new key's pubkey is used to encrypt the DSN
- The encrypted snapshot is stored alongside the new key in `sdk_api_keys`
- When the key is revoked, the project's `ecies_encrypted_dsn` is restored from the previous key's snapshot (or set to NULL if no remaining active key)

Alternatively (simpler): store the DSN plaintext in a server-side encrypted column using `EMBYR_ENCRYPTION_KEY` (AES-256-GCM), separate from the ECIES-encrypted column. The ECIES-encrypted column is then derived from the plaintext at key creation time. This avoids the "chicken and egg" problem at the cost of having the plaintext available server-side (protected by `EMBYR_ENCRYPTION_KEY`).

**Selected resolution:** Store DSN as AES-256-GCM encrypted under `EMBYR_ENCRYPTION_KEY` in a new `projects.backend_pg_dsn_enc` column. The existing `ecies_encrypted_dsn` is derived from this on key creation and updated on rotation. This is consistent with how `oidc_providers.client_secret_enc` is stored (AES-256-GCM under `EMBYR_ENCRYPTION_KEY`, per AC-B06-02).

## Alternatives Considered

### Option 2: Duplicate crypto in the SDK key handler (rejected)

Copy `argon2::hash_api_key`, `ecies::derive_public_key`, and BLAKE3 calls directly into `sdk_keys.rs`.

**Rejected because:**
- Violates DRY at the domain function level. Any change to key derivation parameters (e.g., a future HKDF salt change) requires updating two places.
- The `RotateAuthKey` command is documented in the domain model as the canonical path for key rotation. Bypassing it creates an undocumented dual-path for key management.
- The ECIES scheme in `provision.rs` was specifically designed to link the API key to the credential encryption. Duplicating it without encapsulation risks subtle divergence.

## Consequences

**Positive:**
- All key material derivation is in `embyr-core::domain::project` — one canonical location, auditable.
- `new_sdk_key_material` is a pure function: testable in `cargo test` without DB or Axum.
- The handler is clean: generate bytes → compute material → transactional write → return.
- Dual-hash rotation window (`api_key_hash_secondary`) ensures zero downtime during SDK key rotation — existing Firestore SDK clients authenticated with the old key continue to work until the rotation window is cleared.

**Negative / Trade-offs:**
- Adding `backend_pg_dsn_enc` (AES-GCM under `EMBYR_ENCRYPTION_KEY`) is a schema addition. Migration must add this column and backfill it from `ecies_encrypted_dsn` using the first-created SDK key's raw key — which is not available (only the hash is stored). Therefore, backfill is not possible for projects provisioned before this feature. Those projects require re-provisioning to use SDK key rotation with DSN re-encryption. The migration marks existing `backend_pg_dsn_enc` as NULL for pre-existing projects, and the SDK key creation handler skips DSN re-encryption (step 7b) when `backend_pg_dsn_enc IS NULL`, falling back to keeping the existing `ecies_encrypted_dsn` unchanged.
- **Known limitation — pre-existing projects:** Projects provisioned before admin-api-v2 (i.e., before this migration runs) have `backend_pg_dsn_enc = NULL`. They are permanently excluded from SDK key rotation DSN re-encryption. The handler detects this condition and skips step 7b silently (no error, no DSN update). The existing `ecies_encrypted_dsn` remains unchanged on those projects. The only remediation is to re-provision the project (delete + recreate). New projects created in V1+ (after this migration) have full SDK key rotation support. Acceptance test `pre_existing_project_with_null_dsn_enc_skips_re_encryption_silently` covers this path (AC-B04, OQ-B01).
- Argon2id remains on the blocking thread (spawn_blocking) — consistent with the existing provision path. No latency regression.
- The dual-hash secondary column must be cleared explicitly after key rotation is confirmed complete. The handler does not auto-clear it (same as the existing rotation window behaviour).

## Earned Trust

The acceptance test for AC-B03-03 exercises the full integration path:
1. Call `POST /admin/v1/projects/:id/sdk_keys` → receive `{ key: "embyr_sdk_..." }`
2. Send a real Firestore gRPC GetDocument request to embyr-server using the new SDK key as the Bearer token
3. Assert: 200 response (Argon2id verify succeeds, credential cache populated, storage op executes)
This is the behavioral proof that `new_sdk_key_material` produces hashes compatible with the auth middleware's verification path.

<!-- markdownlint-disable MD024 -->
# User Stories — secrets-management

> Feature: secrets-management
> Wave: DISCUSS
> Persona: Sam Chen (P2 — Service Operator / Platform Engineer)
> job_id (all stories): JOB-14
> Updated: 2026-08-09

---

## US-SM-01: Admin Key Sourced from AWS/GCP Secrets Manager at Startup

**job_id:** JOB-14
**slice:** Walking Skeleton — Release 1

### Elevator Pitch
**Before:** `EMBYR_ADMIN_KEY` must be a literal value in the deployment manifest — visible in
`kubectl describe pod`, in process listings, and in any manifest committed or backed up
anywhere. Sam's customers whose security policy mandates AWS/GCP Secrets Manager for all
credentials flag this in every audit.
**After:** `EMBYR_ADMIN_KEY_AWS_SECRET_ARN=arn:aws:secretsmanager:us-east-1:...:secret:embyr-prod-admin-key`
(or the GCP equivalent) sources the admin key at startup with zero literal value in the
manifest. `cargo run -p embyr-server` starts exactly as before for every environment that
still uses the plain `EMBYR_ADMIN_KEY` var.
**Decision enabled by:** Sam decides whether a given deployment sources its admin key from a
secrets manager or a plain env var, per environment, without any code change either way.

### Problem
Sam Chen is a service operator running embyr-server for customers whose security policy
requires all credentials to be sourced from their existing AWS or GCP Secrets Manager
infrastructure. He finds it impossible to satisfy that policy today: `EMBYR_ADMIN_KEY` — the
Bearer token guarding every operator route (provision/delete/suspend projects) and the
`/metrics` endpoint — can only be set as a literal environment variable. Every audit flags
the literal value sitting in the deployment manifest.

### Who
- Sam Chen (P2) | Service operator deploying embyr-server for secrets-manager-policy
  customers | Needs the admin key sourced the same way the customer already sources every
  other credential, without giving up the simple plain-env-var path for local/CI/dev.

### Solution
`ServerConfig::from_env()` gains two new optional env vars, `EMBYR_ADMIN_KEY_AWS_SECRET_ARN`
and `EMBYR_ADMIN_KEY_GCP_SECRET_NAME`. When one is set, the admin key is fetched once at
startup via a new raw-string fetch method added to `AwsSecretFetcher`/`GcpSecretFetcher`
(both currently DSN-JSON-specific via `parse_dsn`). When neither is set, plain
`EMBYR_ADMIN_KEY` behaves exactly as it does today. Setting more than one source for the
admin key is a startup config error.

### Domain Examples

**1. Happy path (AWS):** Sam sets `EMBYR_ADMIN_KEY_AWS_SECRET_ARN=arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr-prod-admin-key-AbCdEf`
and leaves plain `EMBYR_ADMIN_KEY` unset. `cargo run -p embyr-server` fetches the secret
value at startup and uses it as the admin key. The server logs
`fetched EMBYR_ADMIN_KEY from AWS Secrets Manager (arn=arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr-prod-admin-key-AbCdEf)`
— never the fetched value.

**2. Happy path (GCP):** Sam sets `EMBYR_ADMIN_KEY_GCP_SECRET_NAME=projects/finops-prod/secrets/embyr-admin-key`.
Startup fetches the same way via the GCP Secret Manager REST API path already used by
`GcpSecretFetcher` for customer DSNs.

**3. Local/CI fallback — zero behavior change:** Sam's CI pipeline (from the
production-readiness feature) sets plain `EMBYR_ADMIN_KEY=test-admin-key` with neither
secret-ref var set. The server starts exactly as it did before this feature shipped.

**4. Error — ambiguous sourcing:** Sam's manifest accidentally sets both
`EMBYR_ADMIN_KEY=literal-key` and `EMBYR_ADMIN_KEY_AWS_SECRET_ARN=arn:...`. Startup exits 1
before any I/O, naming both conflicting variables in stderr.

**5. Error — fetch fails:** Sam's IAM role is missing `secretsmanager:GetSecretValue` on the
configured ARN. Startup logs `startup failed: could not fetch EMBYR_ADMIN_KEY from AWS
Secrets Manager: access denied` and exits 1 — the same "exit 1, no partial startup"
contract used by every other startup failure in `ServerConfig::from_env()`.

### UAT Scenarios (BDD)

```gherkin
Scenario: Server starts with the admin key sourced from AWS Secrets Manager
  Given DATABASE_URL and EMBYR_ENCRYPTION_KEY are set to valid values
  And EMBYR_ADMIN_KEY_AWS_SECRET_ARN points to a secret containing "prod-admin-secret-xyz"
  And plain EMBYR_ADMIN_KEY is not set
  When Sam runs "cargo run -p embyr-server"
  Then the server starts successfully and logs "embyr-server ready"
  And a request with "Authorization: Bearer prod-admin-secret-xyz" to an operator route succeeds
  And no log line at any level contains "prod-admin-secret-xyz"

Scenario: Server starts with the admin key sourced from GCP Secret Manager
  Given DATABASE_URL and EMBYR_ENCRYPTION_KEY are set to valid values
  And EMBYR_ADMIN_KEY_GCP_SECRET_NAME points to a secret containing "prod-admin-secret-abc"
  When Sam runs "cargo run -p embyr-server"
  Then the server starts successfully
  And a request with "Authorization: Bearer prod-admin-secret-abc" to an operator route succeeds

Scenario: Local development keeps working via the plain env var unchanged
  Given DATABASE_URL and EMBYR_ENCRYPTION_KEY are set
  And EMBYR_ADMIN_KEY is set to "local-dev-key" directly
  And no AWS or GCP secret-ref variables are set
  When Sam runs "cargo run -p embyr-server"
  Then the server starts successfully using "local-dev-key" as the admin key
  And no AWS or GCP Secrets Manager client is constructed

Scenario: Startup refuses ambiguous admin-key sourcing
  Given EMBYR_ADMIN_KEY is set to "literal-key"
  And EMBYR_ADMIN_KEY_AWS_SECRET_ARN is also set
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr names both EMBYR_ADMIN_KEY and EMBYR_ADMIN_KEY_AWS_SECRET_ARN as conflicting

Scenario: Startup fails cleanly when the secret cannot be fetched
  Given EMBYR_ADMIN_KEY_AWS_SECRET_ARN points to a secret Sam's IAM role cannot read
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr contains "could not fetch EMBYR_ADMIN_KEY from AWS Secrets Manager"
  And no TCP port is bound

Scenario: Secret value is never written to the system DB or echoed in logs
  Given EMBYR_ADMIN_KEY_AWS_SECRET_ARN points to a secret containing "sentinel-do-not-log-987"
  When the server starts and handles 5 operator API requests
  Then no log line at any level contains "sentinel-do-not-log-987"
  And no row in the system DB contains "sentinel-do-not-log-987"
```

### Acceptance Criteria
- [ ] `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` (optional) sources the admin key from AWS Secrets Manager at startup
- [ ] `EMBYR_ADMIN_KEY_GCP_SECRET_NAME` (optional) sources the admin key from GCP Secret Manager at startup
- [ ] When neither secret-ref var is set, plain `EMBYR_ADMIN_KEY` behavior is unchanged (backward compatible)
- [ ] Setting more than one admin-key source (plain + AWS, plain + GCP, or AWS + GCP) is a config error; server exits 1 before any I/O
- [ ] Secret fetch failure (access denied, not found, malformed) causes exit 1 with a named error; no port is bound
- [ ] The fetched admin key value never appears in any log line at any level
- [ ] `AwsSecretFetcher`/`GcpSecretFetcher` gain a raw-string fetch method distinct from the existing DSN-JSON `parse_dsn` path; existing customer-DSN fetch behavior is unchanged

### Outcome KPIs
- **Who:** Sam Chen operating embyr-server for security-policy-constrained customers
- **Does what:** Sources `EMBYR_ADMIN_KEY` from the customer's existing secrets manager instead of a raw deployment-manifest env var
- **By how much:** 100% of new production deployments can avoid a literal admin-key env var in the manifest (0% possible today)
- **Measured by:** Presence/absence of `EMBYR_ADMIN_KEY_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` vs. literal `EMBYR_ADMIN_KEY` in deployment manifests reviewed at audit time
- **Baseline:** 0% — no secrets-manager sourcing exists for `EMBYR_ADMIN_KEY` today

### Technical Notes
- Walking skeleton for this feature — reuses `AwsSecretFetcher`/`GcpSecretFetcher`
  (`crates/embyr-server/src/adapters/{aws,gcp}_secret_fetcher.rs`), currently DSN-JSON-specific
- Fetched once inside `ServerConfig::from_env()` (step 1 of the ADR-017 startup sequence),
  not per-request — no TTL caching needed for this call, unlike the existing per-request
  customer-DSN fetch path
- Depends on generalizing the two fetchers with a raw-string fetch path (see Locked Decision
  D-SM-3 in feature-delta.md)
- `AwsSecretFetcher`/`GcpSecretFetcher` remain `Option<Arc<...>>` in composition-root wiring,
  constructed only when at least one secret-ref var is configured — mirrors the existing
  optional-adapter pattern already used for customer DSN fetching

---

## US-SM-02: Encryption Key Sourced from AWS/GCP Secrets Manager at Startup

**job_id:** JOB-14
**slice:** Release 1

### Elevator Pitch
**Before:** `EMBYR_ENCRYPTION_KEY` — the single key protecting OIDC client secrets, TOTP
secrets, and `direct_pg` backend DSNs — must be a literal value in the deployment manifest,
same audit exposure as `EMBYR_ADMIN_KEY`.
**After:** `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN=arn:...` (or the GCP equivalent) sources the
encryption key at startup with the same hex-format validation applied today, regardless of
source.
**Decision enabled by:** Sam decides whether the encryption key is secrets-manager-sourced
or plain-env-var-sourced per environment, with identical validation guarantees either way.

### Problem
Sam Chen operates embyr-server for the same secrets-manager-policy customers as US-SM-01,
but `EMBYR_ENCRYPTION_KEY` has the same literal-env-var-only limitation. Unlike the admin
key, this key protects three separate encrypted columns
(`oidc_providers.client_secret_enc`, `users.totp_secret_enc`,
`projects.backend_pg_dsn_enc`), so a leaked manifest value is a wider blast radius.

### Who
- Sam Chen (P2) | Service operator deploying embyr-server for secrets-manager-policy
  customers | Needs the encryption key sourced identically to the admin key, with the
  existing 64-hex-char validation applied regardless of source.

### Solution
`ServerConfig::from_env()` gains `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` and
`EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME`, reusing the raw-string fetch method added in
US-SM-01. The fetched value is hex-decoded and length-validated using the exact same
`ConfigError::InvalidEncryptionKey` path already used for the plain-env-var case — validation
does not special-case the source. The three encryption call sites
(`oidc_providers.rs:130`, `auth.rs:255`, `projects.rs:153`) are unmodified: `state.encryption_key`
is populated identically as `[u8; 32]` regardless of where it came from.

### Domain Examples

**1. Happy path (AWS):** `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` points to a secret whose raw
value is `a3f1...` (64 hex chars). Startup fetches and hex-decodes it into `[u8; 32]`,
identical to the plain-env-var path.

**2. Happy path — OIDC encryption still works with a secrets-manager-sourced key:** With
the encryption key sourced from AWS Secrets Manager, an Owner creates an OIDC provider with
`client_secret: "okta-client-secret-9f2c"`. The provider is stored with `client_secret_enc`
encrypted under the fetched key — no different from the plain-env-var case.

**3. Local/CI fallback:** Sam's CI pipeline keeps using plain `EMBYR_ENCRYPTION_KEY` with
neither secret-ref var set — unchanged behavior.

**4. Error — fetched value wrong length:** The AWS secret's raw value is `"tooshort"` (not
64 hex chars). Startup exits 1 with `invalid EMBYR_ENCRYPTION_KEY: expected 64 hex
characters (32 bytes), got 8 characters` — the identical error message the plain-env-var
path already produces today.

**5. Error — ambiguous sourcing:** Sam sets both plain `EMBYR_ENCRYPTION_KEY` and
`EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME`. Startup exits 1 before any I/O.

### UAT Scenarios (BDD)

```gherkin
Scenario: Server starts with the encryption key sourced from AWS Secrets Manager
  Given DATABASE_URL and EMBYR_ADMIN_KEY are set to valid values
  And EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN points to a secret containing a valid 64-hex-char key
  And plain EMBYR_ENCRYPTION_KEY is not set
  When Sam runs "cargo run -p embyr-server"
  Then the server starts successfully and logs "embyr-server ready"

Scenario: OIDC client secret encryption succeeds with a secrets-manager-sourced key
  Given the server started with EMBYR_ENCRYPTION_KEY sourced from AWS Secrets Manager
  When an Owner creates an OIDC provider with client_secret "okta-client-secret-9f2c"
  Then the response is HTTP 201
  And the stored client_secret_enc value is not equal to the plaintext "okta-client-secret-9f2c"

Scenario: Local development keeps working via the plain env var unchanged
  Given DATABASE_URL and EMBYR_ADMIN_KEY are set
  And EMBYR_ENCRYPTION_KEY is set directly to a valid 64-hex-char value
  And no AWS or GCP secret-ref variables are set
  When Sam runs "cargo run -p embyr-server"
  Then the server starts successfully using the plain value as the encryption key

Scenario: Fetched encryption key of invalid length is rejected at startup
  Given EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN points to a secret containing "tooshort"
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr contains "invalid EMBYR_ENCRYPTION_KEY"

Scenario: Startup refuses ambiguous encryption-key sourcing
  Given EMBYR_ENCRYPTION_KEY is set to a literal value
  And EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME is also set
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr names both conflicting variables

Scenario: Startup fails cleanly when the secret cannot be fetched
  Given EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME points to a secret Sam's service account cannot read
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr contains "could not fetch EMBYR_ENCRYPTION_KEY from GCP Secret Manager"
  And no TCP port is bound
```

### Acceptance Criteria
- [ ] `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` (optional) sources the encryption key from AWS Secrets Manager at startup
- [ ] `EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME` (optional) sources the encryption key from GCP Secret Manager at startup
- [ ] Fetched value is hex-decoded and length-validated using the identical `ConfigError::InvalidEncryptionKey` path as the plain-env-var case
- [ ] When neither secret-ref var is set, plain `EMBYR_ENCRYPTION_KEY` behavior is unchanged
- [ ] Setting more than one encryption-key source is a config error; server exits 1 before any I/O
- [ ] Secret fetch failure causes exit 1 with a named error; no port is bound
- [ ] `oidc_providers.rs`, `auth.rs`, `projects.rs` require zero code changes — `state.encryption_key` is populated identically regardless of source
- [ ] The fetched encryption key value never appears in any log line at any level

### Outcome KPIs
- **Who:** Sam Chen operating embyr-server for security-policy-constrained customers
- **Does what:** Sources `EMBYR_ENCRYPTION_KEY` from the customer's existing secrets manager instead of a raw deployment-manifest env var
- **By how much:** 100% of new production deployments can avoid a literal encryption-key env var in the manifest (0% possible today)
- **Measured by:** Presence/absence of `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` vs. literal `EMBYR_ENCRYPTION_KEY` in deployment manifests reviewed at audit time
- **Baseline:** 0% — no secrets-manager sourcing exists for `EMBYR_ENCRYPTION_KEY` today

### Technical Notes
- Builds directly on US-SM-01's raw-string fetch method — no new fetcher capability required
- Hex-decode + length validation happens after fetch, reusing the existing
  `ConfigError::InvalidEncryptionKey` variant (`crates/embyr-server/src/config.rs`)
- The three call sites (`oidc_providers.rs:130`, `auth.rs:255`, `projects.rs:153`) are
  unmodified by this story — preserves the ACL boundary: only `ServerConfig::from_env()`
  and `main.rs` startup know about secrets-manager sourcing
- Depends on US-SM-01 (raw-string fetch method must exist first)

---

## US-SM-03: Rotate EMBYR_ENCRYPTION_KEY Without Orphaning Existing Encrypted Data

**job_id:** JOB-14
**slice:** Release 2

### Elevator Pitch
**Before:** `EMBYR_ENCRYPTION_KEY` is a single global key with no rotation path. Rotating it
— planned, or forced by a leak — permanently orphans every existing
`oidc_providers.client_secret_enc`, `users.totp_secret_enc`, and
`projects.backend_pg_dsn_enc` row, because `Aes256Gcm::new_from_slice(&state.encryption_key)`
uses exactly one key with no fallback.
**After:** Sam sets `EMBYR_ENCRYPTION_KEY_PREVIOUS` to the retiring key alongside the new
`EMBYR_ENCRYPTION_KEY`. Existing TOTP-enrolled users keep signing in throughout the rotation
window — decrypt tries the current key first, falls back to the previous key on
authentication failure.
**Decision enabled by:** Sam decides when to close the rotation window (drop
`EMBYR_ENCRYPTION_KEY_PREVIOUS`) based on his own confidence that every consumer has
re-authenticated at least once, without a forced synchronous re-encryption migration
blocking the rotation itself.

### Problem
Sam Chen needs to rotate `EMBYR_ENCRYPTION_KEY` — either as scheduled security hygiene or in
response to a suspected leak. He has no rotation path to reach for: the codebase already
solved a shape-identical problem for Argon2id project API-key auth (`auth_key_hash` /
`auth_key_hash_2`, checked in order — "dual-hash rotation window"), but nothing analogous
exists for the AES-256-GCM encryption key. Today, rotating `EMBYR_ENCRYPTION_KEY` means every
row encrypted under the old key becomes permanently undecryptable the moment the process
restarts with the new key.

### Who
- Sam Chen (P2) | Service operator responding to a suspected key leak or performing
  scheduled rotation | Needs to rotate `EMBYR_ENCRYPTION_KEY` without a maintenance window
  and without losing access to any previously encrypted row.

### Solution
`EMBYR_ENCRYPTION_KEY_PREVIOUS` (optional, sourceable via the same plain/AWS/GCP mechanism
as `EMBYR_ENCRYPTION_KEY` from US-SM-02) holds the retiring key during a rotation window. A
single shared rotation-aware decrypt function tries the current key first; on AES-GCM
authentication-tag failure (never a false-positive "success" — AEAD makes wrong-key decrypts
fail loudly), it retries with the previous key when configured, before returning the same
decrypt-failure response the system already returns today. All writes continue to encrypt
under the CURRENT key only — no dual-write. `EMBYR_ENCRYPTION_KEY_PREVIOUS` must not equal
`EMBYR_ENCRYPTION_KEY` (rejected at startup). Lazy re-encryption / background migration is
explicitly out of scope for this slice (see Locked Decision D-SM-5) — this mirrors the
established `auth_key_hash_2` precedent, which also has no self-healing rehash, only an
"also check the second value if present" comparison.

### Domain Examples

**1. Happy path — TOTP sign-in during the rotation window:** Sam rotates
`EMBYR_ENCRYPTION_KEY`. Maria Santos enrolled TOTP before rotation; her `totp_secret_enc` is
encrypted under the OLD key. Sam sets `EMBYR_ENCRYPTION_KEY` to the new key and
`EMBYR_ENCRYPTION_KEY_PREVIOUS` to the old key, restarts. Maria signs in with her existing
authenticator app code; sign-in succeeds — decrypt tries the new key (fails: wrong
authentication tag), falls back to the previous key (succeeds).

**2. Edge case — post-rotation enrollment uses the current key only:** Diego Ramirez enrolls
TOTP for the first time AFTER the rotation. His `totp_secret_enc` is encrypted exclusively
under the CURRENT (new) key — no dependency on `EMBYR_ENCRYPTION_KEY_PREVIOUS` at write time.

**3. Error case — genuinely corrupted ciphertext:** A row's `totp_secret_enc` is truncated
(disk corruption, not a stale-key case). Both the current key and the previous key fail AEAD
authentication. The server returns the same decrypt-failure response as today (401 invalid
code) — trying two keys in sequence cannot produce a false-positive "successful" decrypt of
corrupted data, because AES-GCM's authentication tag makes that cryptographically impossible.

**4. Operational boundary — window closed too early:** Sam confirms via his own client
inventory that most users have re-authenticated, and removes `EMBYR_ENCRYPTION_KEY_PREVIOUS`
after 14 days. A user who has not signed in in that window (e.g., on leave) will fail TOTP
verification on their next sign-in after the window closes — this is a documented
operational risk Sam manages by choosing a window long enough for his user base's
re-authentication cadence, not a system defect (see Technical Notes).

### UAT Scenarios (BDD)

```gherkin
Scenario: Existing TOTP secret decrypts via the previous key during the rotation window
  Given Maria Santos enrolled TOTP before rotation; her totp_secret_enc is encrypted under the OLD key
  And EMBYR_ENCRYPTION_KEY is set to the NEW key and EMBYR_ENCRYPTION_KEY_PREVIOUS is set to the OLD key
  When Maria signs in with her existing authenticator app code
  Then sign-in succeeds

Scenario: Post-rotation TOTP enrollment uses the current key only
  Given the server is running with EMBYR_ENCRYPTION_KEY_PREVIOUS configured
  When Diego Ramirez enrolls TOTP for the first time
  Then his totp_secret_enc is encrypted such that it decrypts successfully using ONLY the current key
  And it does not require the previous key to decrypt

Scenario: Behavior is unchanged when no previous key is configured
  Given EMBYR_ENCRYPTION_KEY_PREVIOUS is not set
  When an existing user signs in with a TOTP secret encrypted under the current key
  Then sign-in succeeds using single-key decrypt, identical to pre-feature behavior

Scenario: Corrupted ciphertext fails cleanly under both keys
  Given a totp_secret_enc value is truncated to 8 bytes (below the 12-byte nonce minimum)
  And EMBYR_ENCRYPTION_KEY_PREVIOUS is configured
  When the affected user attempts to sign in with a TOTP code
  Then the response is the same decrypt-failure outcome the system returns today
  And no plaintext is returned to the caller

Scenario: Startup rejects an identical current and previous encryption key
  Given EMBYR_ENCRYPTION_KEY and EMBYR_ENCRYPTION_KEY_PREVIOUS are set to the same value
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr states that EMBYR_ENCRYPTION_KEY_PREVIOUS must differ from EMBYR_ENCRYPTION_KEY

Scenario: The rotation-aware decrypt helper works against both OIDC-shaped and DSN-shaped ciphertext
  Given a ciphertext was produced by encrypting a client_secret-shaped value under an old key
  And a ciphertext was produced by encrypting a DSN-shaped value under the same old key
  When each ciphertext is decrypted using the shared rotation-aware helper with current=new key, previous=old key
  Then both decrypt successfully via the previous-key fallback
```

### Acceptance Criteria
- [ ] `EMBYR_ENCRYPTION_KEY_PREVIOUS` (optional) accepts a second 64-hex-char key, validated identically to `EMBYR_ENCRYPTION_KEY`, sourceable via the same plain/AWS/GCP mechanism as US-SM-02
- [ ] TOTP secret decrypt (`auth.rs` signin) tries `EMBYR_ENCRYPTION_KEY` first; on AEAD authentication failure, tries `EMBYR_ENCRYPTION_KEY_PREVIOUS` (when configured) before returning a decrypt error
- [ ] All new writes (TOTP enrollment, OIDC provider creation, project DSN patch) encrypt exclusively under the CURRENT key — `EMBYR_ENCRYPTION_KEY_PREVIOUS` is never used for encryption
- [ ] The current-then-previous key-selection logic is implemented as a single shared function, not duplicated per call site, so any future decrypt consumer of `client_secret_enc` or `backend_pg_dsn_enc` inherits rotation-safety without new code
- [ ] When `EMBYR_ENCRYPTION_KEY_PREVIOUS` is absent, behavior is byte-for-byte identical to today (single-key decrypt, existing error path on failure)
- [ ] A ciphertext that fails AEAD authentication under BOTH keys returns the existing decrypt-failure behavior — never a false-positive "successful" decrypt
- [ ] `EMBYR_ENCRYPTION_KEY_PREVIOUS` equal to `EMBYR_ENCRYPTION_KEY` is rejected at startup as a config error

### Outcome KPIs
- **Who:** Sam Chen rotating `EMBYR_ENCRYPTION_KEY`
- **Does what:** Completes a rotation with zero rows permanently orphaned during the rotation window
- **By how much:** 0 orphaned rows per rotation performed within the operator-managed window (vs. 100% orphan rate today)
- **Measured by:** Integration test proving pre-rotation-encrypted TOTP secrets decrypt successfully mid-window
- **Baseline:** 100% orphan rate today — a single global key with no fallback means every existing row is undecryptable the instant the key changes

### Technical Notes
- Only one decrypt call site currently exists in the codebase for any of the 3 AES-GCM
  sites: `auth.rs` TOTP verification at signin. `oidc_providers.client_secret_enc` and
  `projects.backend_pg_dsn_enc` are write-only today (no decrypt/connect consumer exists
  yet) — this story cannot end-to-end-test rotation-safety for those two beyond proving the
  shared decrypt helper handles their ciphertext shape correctly in isolation (see UAT
  scenario 6)
- No lazy re-encryption / background re-encrypt-on-successful-decrypt is implemented in this
  slice (Locked Decision D-SM-5). This mirrors the existing `auth_key_hash_2` precedent in
  the codebase, which also performs a static "check both, if present" comparison with no
  self-healing rehash
- Residual risk: a row belonging to a user who never re-authenticates during the entire time
  `EMBYR_ENCRYPTION_KEY_PREVIOUS` remains configured becomes undecryptable once the window
  closes. This is a documented operational constraint for Sam to manage via window duration,
  not a system defect — flagged for DESIGN wave awareness, not blocking DoR
- Depends on US-SM-02 (encryption key sourcing/validation mechanism) for
  `EMBYR_ENCRYPTION_KEY_PREVIOUS` to be sourceable via secrets manager, though a
  plain-env-var-only rotation is independently valuable and does not strictly require
  US-SM-01/02 to ship first

---

## US-SM-04: Rotate EMBYR_ADMIN_KEY Without a Hard Cutover Outage

**job_id:** JOB-14
**slice:** Release 3

### Elevator Pitch
**Before:** `EMBYR_ADMIN_KEY` is compared via a single `token == state.admin_key` check.
Rotating it means every existing operator client (scripts, CI credentials, the `/metrics`
scraper) using the OLD token starts receiving 401 the instant the new value goes live — a
synchronized "flag day" cutover across every consumer.
**After:** Sam sets `EMBYR_ADMIN_KEY_PREVIOUS` to the retiring token alongside the new
`EMBYR_ADMIN_KEY`. Both tokens are valid during the window; Sam updates each client on his
own schedule, then closes the window.
**Decision enabled by:** Sam decides how long the rotation window stays open based on his
own client inventory, rather than being forced into a single synchronized cutover moment.

### Problem
Sam Chen needs to rotate `EMBYR_ADMIN_KEY` — the Bearer token guarding every operator route
and `/metrics` — either as scheduled hygiene or in response to a leak. Today this requires
coordinating every operator client (deployment scripts, CI secrets, Grafana's `/metrics`
scraper, ad hoc curl scripts) to update simultaneously with the restart, because
`operator_auth_middleware` accepts exactly one valid token value.

### Who
- Sam Chen (P2) | Service operator responsible for admin-key rotation | Needs to introduce a
  new admin key and retire the old one on his own schedule, without a single moment where
  old-token clients suddenly break.

### Solution
`EMBYR_ADMIN_KEY_PREVIOUS` (optional, sourceable via the same plain/AWS/GCP mechanism as
`EMBYR_ADMIN_KEY` from US-SM-01) holds the retiring token.
`operator_auth_middleware` accepts a Bearer token equal to EITHER `state.admin_key` OR
`state.admin_key_previous` (when configured) — applied identically to every route it already
gates, including `/metrics`. `EMBYR_ADMIN_KEY_PREVIOUS` must not equal `EMBYR_ADMIN_KEY`
(rejected at startup). The existing hard-cutover option (set only the new key, no previous)
remains available and unchanged for leaked-key emergencies where no grace period is wanted.

### Domain Examples

**1. Happy path — mid-rotation, both tokens valid:** Sam sets
`EMBYR_ADMIN_KEY=new-token-2026` and `EMBYR_ADMIN_KEY_PREVIOUS=old-token-2025`, restarts.
His Grafana scraper (still configured with `old-token-2025`) keeps polling `/metrics`
successfully. His CI pipeline (already updated to `new-token-2026`) also succeeds. Neither
breaks.

**2. Edge case — leaked key, no grace period wanted:** Sam discovers `old-token-2025` leaked
in a log aggregator. He rotates immediately by setting only the new
`EMBYR_ADMIN_KEY=incident-response-token`, with no `EMBYR_ADMIN_KEY_PREVIOUS` — the existing
hard-cutover behavior, unchanged and still available for this scenario.

**3. Error case — stale-token access after the window closes:** Sam finishes rotation,
removes `EMBYR_ADMIN_KEY_PREVIOUS`, restarts. A forgotten cron job still using
`old-token-2025` now receives 401 — expected, and identical to the existing "wrong token"
failure mode, not a new one.

**4. Error case — ambiguous window:** Sam accidentally sets `EMBYR_ADMIN_KEY_PREVIOUS` equal
to `EMBYR_ADMIN_KEY`. Startup exits 1 rather than silently accepting a pointless
single-token "window."

### UAT Scenarios (BDD)

```gherkin
Scenario: Both current and previous admin tokens are accepted during the rotation window
  Given EMBYR_ADMIN_KEY is set to "new-token-2026" and EMBYR_ADMIN_KEY_PREVIOUS is set to "old-token-2025"
  When a request to GET /admin/v1/projects carries "Authorization: Bearer old-token-2025"
  Then the response is HTTP 200
  When a request to GET /admin/v1/projects carries "Authorization: Bearer new-token-2026"
  Then the response is HTTP 200

Scenario: The /metrics endpoint honors the same dual-token window as every other operator route
  Given EMBYR_ADMIN_KEY_PREVIOUS is configured alongside EMBYR_ADMIN_KEY
  When a request to GET /metrics carries "Authorization: Bearer" set to either the current or the previous token
  Then the response is HTTP 200 in both cases

Scenario: Behavior is unchanged when no previous key is configured
  Given EMBYR_ADMIN_KEY_PREVIOUS is not set
  When a request carries "Authorization: Bearer" set to any value other than EMBYR_ADMIN_KEY
  Then the response is HTTP 401, identical to pre-feature behavior

Scenario: Hard cutover with no grace period remains available
  Given Sam rotates EMBYR_ADMIN_KEY to "incident-response-token" without setting EMBYR_ADMIN_KEY_PREVIOUS
  When a request carries "Authorization: Bearer" set to the old, now-retired token
  Then the response is HTTP 401 immediately after restart

Scenario: Startup rejects an identical current and previous admin key
  Given EMBYR_ADMIN_KEY and EMBYR_ADMIN_KEY_PREVIOUS are set to the same value
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr states that EMBYR_ADMIN_KEY_PREVIOUS must differ from EMBYR_ADMIN_KEY

Scenario: Retired admin token is rejected once the rotation window is closed
  Given EMBYR_ADMIN_KEY_PREVIOUS has been removed from the environment and the server restarted
  When a request to GET /metrics carries "Authorization: Bearer old-token-2025"
  Then the response is HTTP 401
```

### Acceptance Criteria
- [ ] `EMBYR_ADMIN_KEY_PREVIOUS` (optional) env var, sourceable via plain var or AWS/GCP secrets manager (reusing US-SM-01's mechanism)
- [ ] `operator_auth_middleware` accepts either `EMBYR_ADMIN_KEY` or `EMBYR_ADMIN_KEY_PREVIOUS` (when configured) as a valid Bearer token
- [ ] `/metrics` accepts both tokens identically to every other operator route during the window (same middleware, no separate auth path)
- [ ] When `EMBYR_ADMIN_KEY_PREVIOUS` is unset, behavior is unchanged — exactly one valid token, matching current production behavior
- [ ] `EMBYR_ADMIN_KEY_PREVIOUS` equal to `EMBYR_ADMIN_KEY` is rejected at startup as a config error
- [ ] A request with neither token, or a token matching neither value, returns 401 — unchanged from today
- [ ] Closing the rotation window (removing `EMBYR_ADMIN_KEY_PREVIOUS` and restarting) immediately invalidates the retired token — no residual grace period beyond the operator's own restart timing

### Outcome KPIs
- **Who:** Sam Chen rotating `EMBYR_ADMIN_KEY`
- **Does what:** Rotates the admin bearer token with zero simultaneous-client-breakage incidents
- **By how much:** 0 unplanned operator-route/`/metrics` outages caused by admin-key rotation (vs. every historical rotation requiring a synchronized flag-day cutover)
- **Measured by:** Integration test proving dual-token acceptance; ops incident tracker tagged `admin-key-rotation`, pre/post feature
- **Baseline:** N/A — no rotation path exists today; every attempted rotation is a hard, simultaneous cutover

### Technical Notes
- `operator_auth_middleware` (`crates/embyr-server/src/admin/middleware/operator_auth.rs`)
  is the single Bearer-comparison site (per ADR-009 auth-middleware-separation) — this story
  extends that one function, no new middleware layer
- `OperatorState` (`crates/embyr-server/src/admin/state.rs`) gains an
  `admin_key_previous: Option<String>` field alongside the existing `admin_key: String`
- No dependency on US-SM-03 beyond sharing the "*_PREVIOUS, reject-if-identical" pattern —
  the two rotation mechanisms (bearer-token OR-comparison vs. dual-key AEAD decrypt) are
  independently implementable and independently shippable
- Depends on US-SM-01 only for the optional AWS/GCP secrets-manager sourcing of
  `EMBYR_ADMIN_KEY_PREVIOUS`; a plain-env-var-only rotation does not require US-SM-01 to ship first

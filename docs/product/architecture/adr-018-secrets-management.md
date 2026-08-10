# ADR-018: Secrets Management and Rotation

## Status

Accepted

## Context

Production readiness (ADR-017) gave `embyr-server` a real `ServerConfig::from_env()`, but two
of its most security-critical values remain literal environment variables with no rotation
path:

1. **`EMBYR_ADMIN_KEY`** — the Bearer token guarding every operator route
   (`provision`/`delete`/`suspend`/`activate` project, `GET /metrics`) and compared via a
   single `token == state.admin_key` check in
   `crates/embyr-server/src/admin/middleware/operator_auth.rs`.

2. **`EMBYR_ENCRYPTION_KEY`** — a single global 32-byte AES-256-GCM key protecting three
   encrypted columns via `Aes256Gcm::new_from_slice(&state.encryption_key)`:
   - `users.totp_secret_enc` (`admin/handlers/auth.rs:255` — the only LIVE decrypt call site)
   - `oidc_providers.client_secret_enc` (`admin/handlers/oidc_providers.rs:130` — write-only today)
   - `projects.backend_pg_dsn_enc` (`admin/handlers/projects.rs:153` — write-only today)

Both are literal deployment-manifest values with no secrets-manager sourcing and no rotation
mechanism. A rotation of `EMBYR_ENCRYPTION_KEY` today permanently orphans every
previously-encrypted row the instant the process restarts with the new key —
`Aes256Gcm::new_from_slice` accepts exactly one key.

The codebase already solves a shape-identical rotation problem for project API-key auth: the
Argon2id **dual-hash rotation window** (`auth_key_hash` / `auth_key_hash_2`, checked in order,
brief.md line 914 — *"Argon2id verify(api_key, auth_key_hash); also check auth_key_hash_2 if
present"*). That precedent has no self-healing rehash — it is a static "check both, if
present" comparison, operator-terminated by clearing the secondary. This ADR mirrors that
shape for both `EMBYR_ADMIN_KEY` and `EMBYR_ENCRYPTION_KEY`, per D-SM-5/D-SM-6/D-SM-7.

`crates/embyr-server/src/adapters/aws_secret_fetcher.rs` and `gcp_secret_fetcher.rs` already
exist, but are DSN-JSON-specific (`parse_dsn()` expects `{"dsn": "..."}`) and, as of today,
have **zero production callers** — `main.rs` always passes `aws_secret_fetcher: None,
gcp_secret_fetcher: None` to `FirestoreService`, and `build_admin_router` is only ever called
with `None, None` in production. Every existing construction of these fetchers is in test
code (`tests/acceptance/us_10_aws_secrets.rs`, `us_11_gcp_secrets.rs`). This feature is the
first to wire either fetcher into a real startup path.

The architecture brief (`docs/product/architecture/brief.md` § Driven Ports + Adapters)
already documents a **target** `SecretFetcher` trait with a `probe()` method — AWS via STS
`GetCallerIdentity`, GCP via a metadata-server workload-identity OIDC token — but this is
aspirational: neither `AwsSecretFetcher` nor `GcpSecretFetcher` implements any trait or
`probe()` method today; `GcpSecretFetcher::new(base_url, token, ttl_secs)` takes an explicit
bearer token string, which the codebase supplies literally in tests (`"test-token"`). This
ADR does not close that gap (out of scope — see Alternatives Considered and Open Questions);
it documents the interim decision needed to make GCP-sourced config work at all.

System constraints locked in DISCUSS (feature-delta.md): no `embyr-core` changes (all work
lives in `embyr-server`), no new workspace crates, no new ports/routes, fetched secret values
must never appear in logs/responses/error messages/DB rows, and for each logical secret at
most one of {plain env var, AWS secret-ref, GCP secret-ref} may be configured — ambiguity is a
startup config error.

## Decision

### 1. `ServerConfig::from_env()` becomes `async`

Resolving a secrets-manager-sourced value requires an AWS SDK call or a GCP HTTPS call.
`ServerConfig::from_env()` changes from `pub fn from_env() -> Result<Self, ConfigError>` to
`pub async fn from_env() -> Result<Self, ConfigError>`. It remains ADR-017 startup Step 1,
called from within `#[tokio::main]`, still before `tracing_subscriber::init()` — this
preserves the existing "config provides `log_level` to tracing" dependency order. Interim
diagnostic lines emitted during secret resolution (the "fetched `EMBYR_ADMIN_KEY` from AWS
Secrets Manager" confirmation) use `eprintln!`, consistent with how `ConfigError` is already
reported pre-tracing-init — not `tracing::info!`, which is not yet installed at this point.

### 2. Fetchers gain a raw-string method; DSN behavior is untouched (D-SM-3)

Both fetchers gain a new method that returns the secret's raw string value with no JSON
shape assumed:

```
impl AwsSecretFetcher {
    pub async fn get_raw_secret(&self, arn: &str) -> Result<String, AwsSecretError>;
}
impl GcpSecretFetcher {
    pub async fn get_raw_secret(&self, resource_name: &str) -> Result<String, GcpSecretError>;
}
```

Internally, each existing `fetch_raw` (DSN path) is refactored to share the network-call
portion (AWS `get_secret_value().send()`, or the GCP HTTP GET + base64 decode) with the new
raw-string path, and only `parse_dsn()` remains DSN-specific. `get_dsn`, `fetch_fresh`, and
the per-request TTL cache are untouched — `get_raw_secret` does not read or write the cache,
per D-SM-4 (startup-only, once, no TTL benefit).

### 3. `ServerConfig` resolves four logical secrets, each from up to three sources

`admin_key`, `admin_key_previous`, `encryption_key`, `encryption_key_previous` are each
resolved from at most one of `{plain var, *_AWS_SECRET_ARN, *_GCP_SECRET_NAME}` using one
shared private resolver (`resolve_secret_source`) inside `config.rs`, called four times. This
avoids quadruplicating the "which source, is it ambiguous, fetch it" logic.

New environment variables (all optional; absence preserves today's plain-env-var-only
behavior byte-for-byte — D-SM-2):

| Variable | Purpose |
|---|---|
| `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` | AWS-sourced admin key (US-SM-01) |
| `EMBYR_ADMIN_KEY_GCP_SECRET_NAME` | GCP-sourced admin key (US-SM-01) |
| `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` | AWS-sourced encryption key (US-SM-02) |
| `EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME` | GCP-sourced encryption key (US-SM-02) |
| `EMBYR_ADMIN_KEY_PREVIOUS` | Plain retiring admin token (US-SM-04) |
| `EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN` | AWS-sourced retiring admin token |
| `EMBYR_ADMIN_KEY_PREVIOUS_GCP_SECRET_NAME` | GCP-sourced retiring admin token |
| `EMBYR_ENCRYPTION_KEY_PREVIOUS` | Plain retiring encryption key (US-SM-03) |
| `EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN` | AWS-sourced retiring encryption key |
| `EMBYR_ENCRYPTION_KEY_PREVIOUS_GCP_SECRET_NAME` | GCP-sourced retiring encryption key |
| `EMBYR_GCP_ACCESS_TOKEN` | Bearer token for `GcpSecretFetcher` construction; required only if any `*_GCP_SECRET_NAME` var above is set (see Open Questions) |

The `AmbiguousSecretSource` ambiguity rule (D-SM's System Constraints named it for the two
primary vars) is extended uniformly to all four logical secrets: each of `admin_key`,
`admin_key_previous`, `encryption_key`, `encryption_key_previous` independently enforces
"at most one of its three possible sources is set."

`admin_key` and `encryption_key` remain **required** overall (at least one of their three
sources must resolve to a value) — identical requiredness to today.
`admin_key_previous`/`encryption_key_previous` remain **optional** — `None` means no rotation
window is open, and every downstream consumer (`operator_auth_middleware`,
`decrypt_with_rotation`) degrades to today's single-key behavior byte-for-byte (D-SM-2 applied
to rotation, not just sourcing).

`AwsSecretFetcher`/`GcpSecretFetcher` are each constructed **at most once** inside
`from_env()`, lazily, only if at least one of the four logical secrets needs that source —
mirroring the existing `Option<Arc<...>>` "constructed only when configured" pattern used
elsewhere in the composition root. Neither fetcher is retained after `from_env()` returns;
D-SM-4 requires no per-request use, so there is nothing to keep alive.

**AWS credential resolution:** standard AWS SDK default chain
(`aws_config::load_from_env().await` — IAM role, instance profile, or
`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`). No new env var needed; this matches domain
example 5 in US-SM-01 ("Sam's IAM role is missing `secretsmanager:GetSecretValue`").

### 4. `EMBYR_ENCRYPTION_KEY` validation is reused for both current and previous (US-SM-02/03)

`ConfigError::InvalidEncryptionKey` gains a `var: String` field (was `{ reason: String }`,
becomes `{ var: String, reason: String }`) so the identical 64-hex-char validation function
can report either `EMBYR_ENCRYPTION_KEY` or `EMBYR_ENCRYPTION_KEY_PREVIOUS` by name — this
satisfies the AC requirement of "the identical `ConfigError::InvalidEncryptionKey` path"
literally (same variant, same validation code) while keeping the operator-facing message
accurate for either variable.

New variants:

```
pub enum ConfigError {
    MissingVars(Vec<String>),
    InvalidEncryptionKey { var: String, reason: String },   // var field added
    InvalidPort { var: String, value: String },
    InvalidRateLimitRps { value: String },
    AmbiguousSecretSource { var_base: String, sources: Vec<String> },   // NEW
    SecretFetchFailed { var: String, source: String, reason: String }, // NEW
    DuplicateRotationKey { var: String, base_var: String },            // NEW
}
```

`DuplicateRotationKey` implements D-SM's "`_PREVIOUS` must never equal the primary" rule for
both `EMBYR_ADMIN_KEY_PREVIOUS`/`EMBYR_ADMIN_KEY` and
`EMBYR_ENCRYPTION_KEY_PREVIOUS`/`EMBYR_ENCRYPTION_KEY`, checked after both values are resolved
(regardless of source) — comparing the *resolved* values, not the env var text, so a plain
current key and an AWS-sourced previous key that happen to resolve to the same string are
still caught.

### 5. Rotation-aware decrypt helper — resolves DoR OQ-1

**New module `crates/embyr-server/src/adapters/encryption.rs`.**

Not `embyr-core`: the DISCUSS-wave System Constraints explicitly scope this feature to
`embyr-server` only, even though the operation itself is pure computation (no IO) and
`embyr-core` already depends on `aes-gcm` for ECIES — the constraint is a feature-scope
boundary, not strictly the IO-prohibition gate, and this ADR honors it rather than reopening
DISCUSS scope. Not an associated function on `ServerConfig`/`UserAdminState`: those types
represent *configuration* and *request state* respectively; a decrypt operation belongs with
the other crypto-adjacent infrastructure helpers already grouped under `adapters/`
(`aws_secret_fetcher.rs`, `gcp_secret_fetcher.rs`), even though — unlike those two — this
helper performs no IO itself. Grouping under `adapters/` keeps one place for "infrastructure
support code handlers depend on but that isn't domain logic," consistent with existing module
organization.

```
pub enum RotationDecryptError {
    Malformed,              // ciphertext shorter than the 12-byte nonce
    AuthenticationFailed,   // AEAD tag check failed under every configured key
}

pub fn decrypt_with_rotation(
    current_key: &[u8; 32],
    previous_key: Option<&[u8; 32]>,
    ciphertext: &[u8],
) -> Result<Vec<u8>, RotationDecryptError>;
```

Tries `current_key` first; on AEAD authentication failure, tries `previous_key` (if
`Some`) before returning `AuthenticationFailed`. Generic over ciphertext bytes — the same
function is correct for `totp_secret_enc`, `client_secret_enc`, and `backend_pg_dsn_enc`
shapes (UAT scenario 6), satisfying the "single shared function" AC for all three sites even
though only `auth.rs` has a live decrypt caller today. `oidc_providers.rs` and `projects.rs`
are unmodified — they are write-only encrypt sites; per D-SM-5 all writes use the current key
only, so they need no rotation-awareness.

`auth.rs`'s existing block:

```
let cipher = Aes256Gcm::new_from_slice(&state.encryption_key)?;
let totp_secret_bytes = match cipher.decrypt(nonce, &enc_bytes[12..]) { ... }
```

is replaced by a call to `decrypt_with_rotation(&state.encryption_key,
state.encryption_key_previous.as_ref(), &enc_bytes)`. The pre-existing response-code mapping
at this call site is **unchanged**: a ciphertext shorter than 12 bytes still maps to 401
(`invalid_code()`); an `AuthenticationFailed` result (both keys exhausted) still maps to the
existing 500 `INTERNAL_SERVER_ERROR` this call site already returns today. This ADR does not
change that status-code choice — it is a pre-existing behavior this feature is not scoped to
fix, and changing it would be an unrelated hardening change outside the locked ACs.

### 6. Dual-token bearer comparison — `OperatorState` and `UserAdminState`

`OperatorState` gains `admin_key_previous: Option<String>`. `operator_auth_middleware`
accepts a Bearer token equal to either `state.admin_key` or `state.admin_key_previous` (when
`Some`) — applied identically to every route the sub-router already guards, including
`/metrics` (AC requirement).

**DESIGN-added consistency fix (beyond the literal ACs):** `dual_auth_middleware`
(`admin/middleware/dual_auth.rs`) independently compares Bearer tokens against
`UserAdminState::admin_key_env` for `GET /admin/v1/projects/:id` — this is a pre-existing
second Bearer-comparison site the System Constraints did not ask this feature to touch
("`operator_auth_middleware` remains the single Bearer-comparison site... is not reopened").
Left unmodified, a mid-rotation operator using the *previous* token would get 401 on this one
route while succeeding on every other operator route — a silent, inconsistent gap. `UserAdminState`
therefore also gains `admin_key_previous_env: Option<String>` (mirroring `admin_key_env`), and
`dual_auth_middleware`'s Bearer arm accepts either value. This does not reopen the
middleware-separation decision (ADR-009) — `operator_auth_middleware` remains the sole
routing-layer gate for pure-operator routes; `dual_auth_middleware` remains its own,
pre-existing, separate comparison, now simply rotation-aware like its sibling.

### 7. `build_admin_router` signature extension

Two new parameters, threaded into `OperatorState`/`UserAdminState`:

```
pub fn build_admin_router(
    system_db: Arc<SystemDb>,
    admin_key: String,
    admin_key_previous: Option<String>,       // NEW
    credential_cache: Arc<CredentialCache>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>, // NEW
    email_sender: Arc<dyn IEmailSender + Send + Sync>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    rate_limit_capacity: f64,
    prometheus_handle: PrometheusHandle,
) -> Router
```

The four backward-compatible test wrappers (`build`, `build_with_aws`, `build_with_gcp`,
`build_with_secret_fetchers`) pass `None, None` for the two new parameters — zero impact on
existing test callers.

### 8. Startup sequence change (main.rs)

Only ADR-017 Step 1 changes shape (`ServerConfig::from_env()` → `.await`); step numbering and
ordering (tracing → prometheus → DB → migrate → probe → bind → serve) are unchanged. Step 10
(`build_admin_router`) passes the two new `cfg.*_previous` fields.

**OQ-3 resolved — startup warning log, no new scheduler (D-SM-7 preserved):** immediately
after Step 2 (tracing init), for each of `admin_key_previous`/`encryption_key_previous` that
is `Some`, emit one `tracing::warn!(rotation_window_open = true, var = "EMBYR_ADMIN_KEY_PREVIOUS")`-shaped
line (and the encryption-key equivalent). This is presence-based, not age-based — no
background timer, no persisted "since when" state, so it adds zero new scheduling
infrastructure, honoring D-SM-7's explicit "no automatic expiry timer" constraint while
giving Sam a nudge on every restart that a window is open.

## Alternatives Considered

### A1: New adapter types (`RotatingAwsSecretFetcher`, etc.) instead of extending existing fetchers

Rejected (resolves the D-SM-3 DoR question). A new type would duplicate SDK-client
construction, error mapping, and the `Option<Arc<...>>` composition-root wiring pattern for
zero behavioral benefit — the only difference from the existing fetchers is *parsing*
(`parse_dsn` vs. raw passthrough), not *fetching*. Extending with `get_raw_secret()` shares
100% of the network-call code and keeps one fetcher per cloud, matching D-SM-1's "reuse the
existing pattern" guidance.

### A2: Put `decrypt_with_rotation` in `embyr-core` as a pure domain function

Rejected. Technically legal under the IO-prohibition (`aes-gcm` is already an `embyr-core`
dependency for ECIES, and this function performs no IO), but the DISCUSS-wave System
Constraints explicitly scope all work in this feature to `embyr-server`. Reopening that
scope boundary without a DISCUSS-wave revisit is out of bounds for DESIGN. If a future
feature needs this helper from `embyr-core` (e.g., a pure-domain rotation policy), it can be
relocated then — moving a pure function later is a mechanical, low-risk refactor.

### A3: Duplicate the current-then-previous check per call site instead of a shared helper

Rejected. Directly contradicts the AC: "single shared function, not duplicated per call
site, so any future decrypt consumer of `client_secret_enc` or `backend_pg_dsn_enc` inherits
rotation-safety without new code." Duplication would also mean the AEAD-failure-mapping
inconsistency already present in the codebase (401 for malformed vs. 500 for auth failure)
would need to be independently re-derived at each of up to three call sites.

### A4: Lazy re-encryption / background re-encrypt-on-successful-decrypt

Rejected — explicitly out of scope per D-SM-5/D-SM-7 and the feature's `Out of Scope`
section. Mirrors the `auth_key_hash_2` precedent, which also has no self-healing rehash.
Adding a background migration job would introduce new scheduling infrastructure this
feature's constraints explicitly forbid, and is orthogonal to closing the "0% rotation path
exists today" gap this feature targets.

### A5: Hard synchronous key-rotation cutover (no dual-key/dual-token window at all)

Rejected — this is the status quo the feature exists to fix. A hard cutover forces every
encrypted row to be re-encrypted in a single atomic operation (impossible without downtime
for `EMBYR_ENCRYPTION_KEY`) or every operator client to update in the same instant (impossible
without coordinated downtime for `EMBYR_ADMIN_KEY`). Both violate the north-star KPI (zero
data permanently orphaned by a rotation) and JOB-14's explicit ask (rotate "without an
outage").

### A6: GCP token acquisition via metadata-server workload identity (OIDC), matching the target `SecretFetcher.probe()` design already in brief.md

Rejected **for this feature**, not rejected in principle. `docs/product/architecture/brief.md`
§ Driven Ports + Adapters already documents workload-identity-based GCP auth as the intended
long-term design, but neither `GcpSecretFetcher` nor any `SecretFetcher` trait exists in code
today — implementing metadata-server token acquisition would add new IO capability beyond
this feature's declared scope ("changes startup config resolution and two existing
auth/decrypt call sites only"; "no new workspace crates"). Retrofitting the full
`SecretFetcher` trait + `probe()` for both fetchers is real, valuable work, but it is a
separate feature-sized change (see Open Questions). This ADR instead reuses
`GcpSecretFetcher`'s existing constructor shape (explicit bearer token string) via a new
`EMBYR_GCP_ACCESS_TOKEN` env var — an interim, honestly-labeled stopgap, not a claim that this
is the target architecture.

## Consequences

### Positive

- Sam can source both `EMBYR_ADMIN_KEY` and `EMBYR_ENCRYPTION_KEY` from AWS or GCP Secrets
  Manager with zero literal credential value in any deployment manifest (US-SM-01, US-SM-02).
- `EMBYR_ENCRYPTION_KEY` rotation no longer permanently orphans existing encrypted rows — the
  dual-key decrypt window brings the encryption key rotation path to parity with the existing
  Argon2id dual-hash precedent (US-SM-03).
- `EMBYR_ADMIN_KEY` rotation no longer requires a synchronized flag-day cutover across every
  operator client, including the `/metrics` scraper (US-SM-04).
- Local/CI/dev environments are provably unaffected: every new env var is optional, and
  absence of all `_AWS_SECRET_ARN`/`_GCP_SECRET_NAME`/`_PREVIOUS*` vars reproduces today's
  exact code path (D-SM-2).
- The single shared `decrypt_with_rotation` function means the two currently-write-only sites
  (`oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`) automatically inherit
  rotation-safety the moment a future feature adds a decrypt consumer for either — no new code
  needed at that point, only a call to the existing helper.
- The AWS-sourcing path requires zero new infrastructure — standard AWS SDK credential chain,
  already proven in this workspace's test suite.

### Negative / Trade-offs

- `ServerConfig::from_env()` becomes `async`, a breaking signature change for any future
  caller (today's only caller, `main.rs`, already runs inside `#[tokio::main]`, so this has no
  practical startup-path impact, but existing unit tests calling `from_env()` synchronously
  must become `#[tokio::test]` — a mechanical DELIVER-wave update).
- `ConfigError::InvalidEncryptionKey` gains a field (`{ reason }` → `{ var, reason }`), a
  breaking change to the existing enum shape; the three existing unit tests referencing this
  variant need mechanical updates.
- 11 new environment variables (Component Decomposition table, brief.md). All are optional
  except `EMBYR_GCP_ACCESS_TOKEN`, which is conditionally required. This is a larger surface
  than a minimal design would prefer, but is the direct, reviewed consequence of honoring
  US-SM-03/US-SM-04's ACs that `_PREVIOUS` values are "sourceable via the same plain/AWS/GCP
  mechanism" as their primary counterparts (already peer-reviewed and approved in DISCUSS —
  see DoR OQ-2 resolution below).
- `EMBYR_GCP_ACCESS_TOKEN` is a static bearer token with no in-process refresh. GCP OAuth2
  access tokens typically expire in ~1 hour. This is safe for THIS feature only because the
  token is used exactly once, at startup, and then discarded (D-SM-4: no per-request reuse) —
  there is no window in which a stale in-memory token could be silently used. It does mean an
  operator restarting the process with an expired token gets a clean, loud startup failure
  (`ConfigError::SecretFetchFailed`, exit 1, no port bound), not a silent one. A durable fix
  (workload-identity token refresh) is tracked as a separate architectural gap — see Open
  Questions.
- `dual_auth_middleware`'s rotation-awareness (§6) is a DESIGN-identified addition beyond the
  literal ACs. It is small (one new `Option<String>` field, one new comparison arm) but
  broadens this feature's diff slightly beyond the DISCUSS-wave story boundaries. Flagged
  explicitly rather than silently included, per Earned Trust discipline — the alternative
  (leaving it inconsistent) is a worse, silent gap.

### DoR Open Questions — Resolved

| ID | Question | Resolution |
|----|----------|------------|
| OQ-1 | Where does the decrypt-with-fallback helper live? | New module `crates/embyr-server/src/adapters/encryption.rs`; NOT `embyr-core` (feature-scope constraint), NOT an associated function on `ServerConfig`/`UserAdminState` (wrong responsibility). See Decision §5. |
| OQ-2 | Does `_PREVIOUS` need its own AWS/GCP ARN/name variants? | Yes. The already-approved US-SM-03/US-SM-04 ACs state `_PREVIOUS` is "sourceable via the same plain/AWS/GCP mechanism" as the primary key. Implemented via one shared resolver function called 4×, keeping the *code* cost low even though the *env var count* grows by 6 (3 per key × 2 keys). See Decision §3. |
| OQ-3 | Operator-facing warning when `_PREVIOUS` is configured? | Yes — a presence-based `tracing::warn!` at startup (not age-based, no new scheduler), preserving D-SM-7. See Decision §8. |

### New Open Question (DESIGN-identified, not in original DoR)

| ID | Question | Impact |
|----|----------|--------|
| OQ-SM-4 | `GcpSecretFetcher` has no `probe()` and no workload-identity token acquisition, despite brief.md already documenting that as the target design for the `SecretFetcher` trait. This feature works around it with a static `EMBYR_GCP_ACCESS_TOKEN`. When should `GcpSecretFetcher` (and `AwsSecretFetcher`) be upgraded to implement the documented `SecretFetcher` trait with real `probe()`? | Not blocking this feature (Earned Trust is still satisfied here — see Enforcement). Recommended as a follow-up feature scoped explicitly around closing the adapter/probe gap for both fetchers, benefiting this feature's GCP path and the pre-existing (unrelated) per-project `gcp_secret` `BackendConfig` variant equally. |

## Enforcement

- Unit tests in `config.rs` assert: ambiguous-source detection for all four logical secrets;
  `DuplicateRotationKey` for both `_PREVIOUS`/primary pairs; byte-for-byte fallback behavior
  when no new var is set; `InvalidEncryptionKey` reports the correct `var` name for both
  `EMBYR_ENCRYPTION_KEY` and `EMBYR_ENCRYPTION_KEY_PREVIOUS`.
- **Breaking-change blast radius (quantified for DELIVER):** the existing `config.rs` test
  module has 9 unit tests; of these, `encryption_key_too_short_returns_error` and
  `config_error_display_invalid_encryption_key` construct `ConfigError::InvalidEncryptionKey`
  directly and require a mechanical one-line update to add the new `var` field. No test
  currently calls `ServerConfig::from_env()` end-to-end (the existing tests exercise only the
  private helpers `collect_required`/`parse_port`/`parse_rate_limit_rps`), so the `async`
  signature change requires zero test-harness changes beyond DELIVER adding `#[tokio::test]`
  to any *new* end-to-end `from_env()` test this feature adds. Total estimated mechanical
  fix-up: 2 existing tests, both single-field additions — not a structural risk.
- **Negative test for "secret value never appears in logs/DB/responses" (System Constraint,
  mandatory):** enforced via a dedicated DISTILL-wave integration test, not a new CI-wide log
  scanner. Pattern: mirrors the existing `EMBYR_AGENT_DB_DSN` precedent (embyr-agent feature,
  Invariant 13) — the test sets a sentinel string as the fetched/plain secret value, drives 5+
  requests through the affected routes (signin, operator routes, `/metrics`), captures the
  full `tracing` output via a test subscriber layer, and asserts the sentinel string is absent
  from captured logs, from every HTTP response body, and from a full-table scan of
  `sessions`/`oidc_providers`/`projects`. No new CI job is introduced; this runs inside the
  existing `test` job (`cargo test --workspace`) as an ordinary integration test.
- Unit tests for `decrypt_with_rotation` assert: current-key success (no previous configured);
  previous-key fallback success; both-keys-fail returns `AuthenticationFailed`; malformed
  (< 12 byte) ciphertext returns `Malformed`; correctness across all three ciphertext shapes
  (TOTP-secret-shaped, client-secret-shaped, DSN-shaped) per UAT scenario 6.
- Integration tests (DISTILL wave) assert the full UAT scenario set verbatim from
  `user-stories.md` for US-SM-01 through US-SM-04, including the mandatory sentinel-string
  negative test (fetched secret value never appears in logs/DB/response).
- **Earned Trust (Principle 12) — why no new `probe()` was added for this feature's use of
  `AwsSecretFetcher`/`GcpSecretFetcher`:** D-SM-4 fetches the admin/encryption key exactly
  once, at startup, and uses that exact fetched value directly — there is no "probe now, use
  later" gap where the environment could lie between the two steps, because there is only one
  step. A fetch failure (wrong ARN, IAM denied, GCP token rejected, malformed secret) is
  itself the hard gate: `ConfigError::SecretFetchFailed` before any port binds, before any
  work is trusted to the value. This is a strictly stronger guarantee than a generic
  credential-validity probe would provide, because the fetch that "probes" the dependency is
  the exact production secret being fetched for actual use — not a proxy check. The AEAD
  authentication tag on `decrypt_with_rotation` provides the equivalent guarantee for the
  rotation-decrypt path: a wrong key cannot produce a false-positive "successful" decrypt,
  cryptographically, so trying two keys in sequence cannot silently corrupt or spoof data —
  the "environment lying" case (corrupted ciphertext) is provably distinguishable from a
  stale-key case by construction, and both correctly fall through to the existing
  decrypt-failure response.
- `cargo deny check` (`deny.toml`, unchanged) continues to enforce `embyr-core`'s IO-free
  invariant — this feature adds zero dependencies to `embyr-core`.
- CI `lint` job (`cargo clippy --workspace -- -D warnings`) covers the new module and
  signature changes.

# ADR-025: Client-Identity Verification Credential — Storage and Rotation

## Status

Accepted

## Context

US-01 requires a project-scoped verification credential (ADR-024: an Ed25519 public
key) to be registered, stored, and never echoed back raw. US-03 (Release 2) requires
rotating that credential without breaking already-signed-in users — DISCUSS's Technical
Notes explicitly point at the existing dual-hash-window precedent:

> "Mirrors the existing D5 pattern (dual-hash window for `api_key` rotation:
> `api_key_hash_current`/`api_key_hash_previous`) ... DESIGN should evaluate reusing the
> identical rotation-window shape for verification-credential rotation rather than
> inventing a new one."

Two structurally similar precedents already exist in the codebase:

1. `projects.api_key_hash_current` / `api_key_hash_previous` (Argon2id hashes,
   `handler.rs::authenticate`) — check current, then previous, either match succeeds.
2. `ServerConfig`'s `admin_key`/`admin_key_previous` and
   `encryption_key`/`encryption_key_previous` (ADR-018) — same current/previous shape,
   generalized to config-sourced secrets with `decrypt_with_rotation()` trying current
   then previous.

Neither precedent's *data* is directly reusable: both store either an Argon2id hash of a
secret or an actual secret key. The new credential is a **public key** (ADR-024) — there
is nothing to hash (hashing a public key would only make verification slower for zero
confidentiality benefit; the value is not secret) and nothing to decrypt.

## Decision

**Reuse the current/previous rotation-window *shape*; do not reuse the *data
representation*, since a public key needs neither hashing nor encryption.**

### New table: `client_identity_credentials`

```sql
CREATE TABLE client_identity_credentials (
    project_id          TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key_current  BYTEA NOT NULL,      -- 32 raw bytes, Ed25519 public key
    public_key_previous BYTEA,                -- NULL when no rotation window is open
    algorithm           TEXT NOT NULL DEFAULT 'EdDSA',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotated_at          TIMESTAMPTZ
);
```

**A new table, not new columns on `projects`.** `projects` already carries 15+
admin-added columns (migration `0015_projects_admin_columns.sql`); `sdk_api_keys` was
already split out as its own table rather than inlined onto `projects` for exactly this
reason (see `admin/handlers/sdk_keys.rs`). `client_identity_credentials` is 1:1 with
`projects` (PK is `project_id` itself, not a surrogate UUID + unique index — there is
exactly one verification credential per project by design, unlike the 1:many
`sdk_api_keys` shape), but it is still a logically distinct entity with its own
lifecycle (`created_at`/`rotated_at`) and is kept out of `projects` for the same
schema-hygiene reason.

### Registration (US-01)

`INSERT` only succeeds when no row exists for `project_id` (enforced by the `PRIMARY
KEY` constraint — a second `INSERT` attempt is a Postgres unique-violation, mapped to
HTTP 409 directing the caller to the rotate action, per AC-16-04). The raw public key is
never returned in the response body — the registration response returns
`{ project_id, algorithm, fingerprint, created_at }` where `fingerprint` is a
BLAKE3-truncated hex string of the public key (16 hex chars, matching the `dc_<16hex>`
NOTIFY-channel-naming precedent's truncation length), giving Alex a way to confirm *which*
key is active without ever transmitting the key material itself back over the wire —
even though the key is not secret, this keeps the response contract identical in shape
to every other "no raw material in the response" convention in the codebase
(`sdk_keys.rs`, `provision.rs`), so a future verification-material type that *is*
sensitive inherits the same safe-by-default response shape with zero additional review
burden.

### Rotation (US-03)

```sql
UPDATE client_identity_credentials
SET public_key_previous = public_key_current,
    public_key_current  = $2,
    rotated_at           = now()
WHERE project_id = $1
RETURNING algorithm, created_at, rotated_at;
```

One UPDATE, one rotation generation retained — identical two-generation-only shape to
ADR-018's `admin_key`/`admin_key_previous` (not an unbounded history). A token signed
under the credential from *three* rotations ago fails verification under both
`public_key_current` and `public_key_previous` and is rejected via the ordinary
`Malformed`/no-longer-valid path (US-03 UAT scenario 3) — there is no separate "stale
credential" error class; a token that fails to verify under either currently-tracked key
is indistinguishable, by design, from a token signed with the wrong key entirely. This
matches DISCUSS's own framing ("rejected as no longer valid — the same 'wrong/stale
credential' class of rejection, not a crash or an internal error").

### Verification order (embyr-core::client_identity, feeds ADR-024's verify function)

Try `public_key_current` first; on signature failure, try `public_key_previous` if
`Some` — identical control-flow shape to `decrypt_with_rotation` (ADR-018 §5) and the
existing `authenticate()` Argon2id current-then-previous check (`handler.rs:195-207`).
This is a **pattern reuse**, not a code-sharing opportunity: Argon2id hash comparison,
AES-GCM decryption, and Ed25519 signature verification are three different
cryptographic primitives with incompatible function signatures. Introducing a generic
"try current then previous" higher-order helper across all three was considered and
rejected — see Alternatives.

### Debug/verify check (US-04)

`POST /admin/v1/projects/:project_id/client_identity_credential/verify` loads the same
`client_identity_credentials` row and calls the identical
`embyr_core::client_identity::verify_client_identity_token()` function used by the
real sign-in path (ADR-026) — the single-shared-verification-routine constraint
(DISCUSS § System Constraints, "shared artifact integration risk") is satisfied
structurally: there is exactly one function in the codebase that performs Ed25519
verification against a `ClientIdentityCredential`, and both call sites (sign-in,
debug-verify) invoke it with the same arguments derived from the same table row. The
debug-verify handler never writes to any session or credential state — it is read-only
by construction (loads the credential, calls the pure verify function, returns the
result), which is what makes "no live session created" (AC-16-14) true by construction
rather than by convention.

## Alternatives Considered

### Alternative 1: Generic `try_current_then_previous<T, E>(current, previous, check_fn)` helper shared across Argon2id/AES-GCM/Ed25519

**Rejected.** The three checks have incompatible signatures (`bool` return for Argon2id,
`Result<Vec<u8>, E>` for AES-GCM, `Result<Claims, E>` for JWT/Ed25519) and different
per-arm side effects (Argon2id runs on a blocking thread pool via `spawn_blocking`; the
other two do not). A generic wrapper would need a trait or closure abstraction whose
indirection costs more than the ~5 lines of duplicated "if current fails, try previous"
control flow it would save at each of the three call sites. The *shape* (two
generations, current-then-previous, no unbounded history) is the reusable asset, not the
code.

### Alternative 2: Hash the public key before storage (mirroring `api_key_hash_current`)

**Rejected.** Hashing exists to protect a *secret* at rest so that a database read alone
cannot recover it. A public key has no such property to protect — by ADR-024's own
security argument, its disclosure is harmless. Hashing it would only prevent embyr's own
verifier from using it (Ed25519 signature verification requires the actual public key
point, not a hash of it) — this is not merely unnecessary but would break verification
entirely; it is not a viable option, included here only because a superficial pattern
-match to `api_key_hash_current`'s naming might otherwise suggest it.

### Alternative 3: Encrypt the public key at rest (ECIES, mirroring `backend_pg_dsn_enc`)

**Rejected** for the same reason as Alternative 2 — ECIES protects confidentiality of a
value that must remain secret from anyone without the decryption key. The public key has
no confidentiality requirement. Encrypting it would add a decrypt step (and a key
-management dependency on the very `api_key`-derived ECIES material this feature is
explicitly not allowed to entangle with a second credential class — see § System
Constraints, "no re-architecting `api_key`'s three roles") for zero security benefit.

## Consequences

### Positive

- Storage is the simplest possible representation of the actual security property
  (public data, needs neither hashing nor encryption) — no cognitive dissonance for a
  future reader wondering why a "credential" table has plaintext key material.
- Rotation-window shape is instantly recognizable to anyone familiar with ADR-018 or the
  existing `api_key` rotation — no new mental model introduced for "how does credential
  rotation work in this codebase."
- Registration's uniqueness constraint (`PRIMARY KEY`) makes the "already registered →
  409" behavior (AC-16-04) a database-enforced invariant, not an application-level
  check that could drift from the schema.

### Negative / Trade-offs

- `algorithm` column is currently always `'EdDSA'` (ADR-024 locks the v1 algorithm) —
  effectively unused variance today. Retained for forward-compatibility (a future
  algorithm addition does not require a schema migration), consistent with the project's
  existing convention of storing a `backend_mode`-style discriminator column even when
  only one variant is live at launch time (mirrors `admin.key`/`encryption.key`'s
  `_previous` columns existing before ADR-018's feature shipped).
- A future "customer wants a JWKS-hosted key instead of a directly-registered one"
  upgrade (ADR-024 Option B, deferred) would require a schema change (a URL column, a
  refresh-cache concept) rather than fitting into this table's current shape — accepted
  as a V2-scoped concern, not a V1 design flaw.

## Enforcement

- Unit tests assert: registration is rejected with a unique-violation-mapped 409 on a
  second attempt for the same project; rotation correctly shifts current → previous;
  verification succeeds under a token signed with the immediately-previous key and fails
  (as `Malformed`) under a key from two rotations ago.
- Integration test (DISTILL wave): a token signed under a credential rotated away 5
  minutes ago still verifies; the same token after a *second* rotation no longer
  verifies — directly exercises US-03's UAT scenarios against real rotation state.

## References

- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-018-secrets-management.md` §§ 5, 6 (rotation-window
  precedent)
- `crates/embyr-server/src/grpc/handler.rs::authenticate` (Argon2id current/previous
  precedent)
- `crates/embyr-server/src/admin/handlers/sdk_keys.rs` (own-table-not-inline-columns
  precedent; no-raw-material-in-response precedent)
- `docs/feature/client-auth/feature-delta.md` §§ User Stories US-01, US-03

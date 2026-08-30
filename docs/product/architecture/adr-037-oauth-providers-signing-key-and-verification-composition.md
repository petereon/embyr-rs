# ADR-037: OAuth Providers — Disjoint Signing-Key Custody, Google ID-Token
# Verification, and a Third Widening of Verification-Time Credential Routing

## Status

Accepted

## Context

`oauth-providers` (JOB-19) adds a third identity-establishment mechanism —
Google sign-in via a Google-issued ID token — alongside `client-auth`'s
customer-minted custom tokens (JOB-16, ADR-024/025/026) and
`client-auth-hosted-identity`'s embyr-hosted email/password (JOB-18,
ADR-036). Two decisions are LOCKED by DISCUSS and orchestrator-confirmed, not
reopened here (`docs/feature/oauth-providers/feature-delta.md` § Job
Discovery Framing Resolution, Resolutions 2 and 5):

1. **No `backend_mode=agent` gating** — Google sign-in is available to every
   `backend_mode` (Resolution 2, confirmed 2026-08-30).
2. **Extends BC-1 Tenant Management** — no new BC-6 (Resolution 5, confirmed
   2026-08-30).

What DISCUSS left to DESIGN, and what this ADR resolves as one coherent
design (mirroring ADR-036's own bundling precedent): exact endpoint shapes,
storage schema, `end_user_id` derivation mechanics, Google ID-token
verification mechanism, JWKS caching, and — the central, DESIGN-level
finding this ADR documents — **where the embyr-owned signing key that mints
this feature's tokens comes from**, since DISCUSS's own Slice 01 brief
explicitly assumed "no signing-key generation... embyr generates nothing
here," an assumption this ADR corrects with evidence (§ Decision 2).

## Decision Drivers

1. **Resolution 2/5 compliance** — no `backend_mode` gate, BC-1 placement —
   non-negotiable inputs, not choices.
2. **Hard constraint (feature-delta.md Handoff flag 4)**: signing/session
   material minting this feature's tokens MUST be structurally disjoint from
   `client_identity_credentials` (ADR-024/025), for the identical
   non-impersonation reason `client-auth-hosted-identity`'s own Resolution 3
   established. Whether to REUSE `hosted_identity_signing_keys` is flagged as
   a legitimate DESIGN-level reuse question, not locked either way.
3. **Reuse over reinvention** — every primitive this feature needs
   (`mint_client_identity_token`, `verify_client_identity_token`, the
   RS256/JWKS primitives `admin/handlers/auth.rs::oidc_callback` already
   uses, the AES-256-GCM-under-`EMBYR_ENCRYPTION_KEY` pattern
   `admin/handlers/oidc_providers.rs`/`projects.rs` already use) already
   exists in the codebase in a directly reusable shape.
4. **Zero changes to `verify_client_identity_token()` and
   `mint_client_identity_token()`** — both remain byte-for-byte unchanged;
   this feature is purely a new caller of each.
5. **ADR-002's Option-D three-part test governs bounded-context placement**,
   applied fresh a third time (§ Decision 1).

## Decision 1 — Bounded-Context Placement: Confirms Resolution 5, Extends BC-1

Applying ADR-002's Option-D three-part test directly to the one candidate
entity this feature's locked v1 scope actually introduces:

| Test | `OAuthProviderCredential` (`project_id, provider → client_id`) |
|---|---|
| Entity with identity of its own? | Yes — `(project_id, provider)`. |
| Lifecycle of its own? | Yes, but thin — register → redefine (idempotent upsert) → [deferred: deregister]. Identical shape to `AccessRule`'s own define/redefine lifecycle (ADR-036's own cited precedent). |
| Invariants of its own? | Yes, but thin — `client_id` non-empty, `provider` constrained to `"google"` in v1 (enforced structurally by the URL path, not a value the request body can widen — § Decision 4), uniqueness per `(project_id, provider)`. |

This is the **identical shape** `client_identity_credentials` already has —
project-scoped, System-DB-resident, no-confidentiality-property auth
material — and that entity was never treated as warranting its own bounded
context. **Confirms Resolution 5: this feature extends BC-1 Tenant
Management.** No new BC-6. `docs/product/architecture/adr-002-bounded-contexts.md`
§ Changed Assumptions is appended with a short note recording that the
Option-D test was applied fresh a third time and — unlike the BC-4/BC-5
precedents — did NOT produce a new context, direct evidence the test is
being applied per-case, not by inertia (mirrors Resolution 5's own framing).

## Decision 2 — Signing-Key Custody: a NEW, Disjoint `oauth_signing_keys`
Table, Encrypted Under `EMBYR_ENCRYPTION_KEY`, Generated at Registration Time

**This is the central finding of this ADR — a correction to DISCUSS's own
Slice 01 assumption, not an open escalation.** Slice 01's brief states "no
signing-key generation (unlike hosted identity, embyr generates nothing
here)." Tracing `mint_client_identity_token(signing_key_seed, ...)`'s exact
signature (`crates/embyr-core/src/client_identity/mod.rs`) shows this cannot
hold: minting always requires a 32-byte embyr-owned Ed25519 seed. Slice 02
cannot mint a `VerifiedEndUserIdentity` for a Google-authenticated end user
without SOME such key existing. Three candidate sources were evaluated:

| Option | Description | Verdict |
|---|---|---|
| **(A) Reuse `hosted_identity_signing_keys`** | Both are "embyr independently establishes identity and mints its own token" flows — Handoff flag 4 names this as a legitimate reuse question. | **Rejected.** `hosted_identity_signing_keys` is populated ONLY by `enable_hosted_identity` (US-01 of the sibling feature). Reusing it would make Google sign-in silently DEPEND on a project having ALSO enabled hosted identity — an undocumented cross-feature coupling that contradicts this feature's own § System Constraints ("coexists with, does not replace... no linking") and would make Google-only projects (JOB-19's own core positioning: "no password to create, no custom-token backend of Trailmark's own required") unable to use the feature they registered for. |
| **(B) A single, server-wide (not per-project) embyr signing key** | One key mints every project's OAuth-derived tokens. | **Rejected.** Regresses the blast-radius containment ADR-036 Decision 2 established (`PRIMARY KEY(project_id)`, one key per project) — a leak of a global key forges identities fleet-wide instead of for one project. No evidence justifies this regression. |
| **(C) A NEW, project-scoped `oauth_signing_keys` table, generated at Slice 01 registration time** | Structurally disjoint from BOTH `client_identity_credentials` (hard constraint, satisfied trivially — it is its own table) AND `hosted_identity_signing_keys` (avoids (A)'s coupling) — one embyr-owned Ed25519 key per project, generated the moment Alex activates Google sign-in for that project. | **Accepted.** |

**Encryption-at-rest mechanism — the second half of this finding.**
`hosted_identity_signing_keys` encrypts its private seed with ECIES, keyed
off the project's `api_key` (ADR-036 Decision 2), because every call site
that decrypts it already carries the `api_key` for an unrelated reason
(resolving a Customer DB connection). **That precondition does not hold
here** — Slice 02's sign-in endpoint has no Customer DB dependency at all
(Resolution 3(B), stateless), so forcing an `api_key` onto it purely to
satisfy ECIES decryption would be a net-new requirement with no other
justification, and would contradict Slice 02's own brief ("no `?key=` query
param is structurally required here... since no Customer DB adapter needs
resolving").

Instead, `oauth_signing_keys.private_key_enc` is encrypted with **AES-256-GCM
under `EMBYR_ENCRYPTION_KEY`** (12-byte nonce prefix, `decrypt_with_rotation`
on read) — the **already-established** pattern for embyr's own control-plane
secrets that do NOT need per-project `api_key`-scoped access, directly
evidenced by two existing call sites: `admin/handlers/auth.rs`'s TOTP-secret
decryption and `admin/handlers/oidc_providers.rs`/`projects.rs`'s inline
AES-256-GCM encryption of `client_secret_enc`/DSN material (ADR-018 §5).
`EMBYR_ENCRYPTION_KEY` is already present on `UserAdminState` (`encryption_key`,
`encryption_key_previous`) and is threaded into the new REST-side
`OAuthProviderState` (§ Decision 6) — no new key-management surface, no new
secret to provision.

**Positive consequence, named explicitly**: because encryption uses the
server-wide `EMBYR_ENCRYPTION_KEY` rather than an api_key-derived ECIES
pubkey, Slice 01's registration handler needs **no `api_key` field in its
request body at all** — it never faces the gap ADR-036 Decision 5 had to
patch after-the-fact ("no raw api_key at enablement time"). This is a
structural advantage of choosing AES-256-GCM/`EMBYR_ENCRYPTION_KEY` over
ECIES/`api_key` for this specific key, not a coincidence — it exists
precisely because the two threat models differ (embyr's own server-wide
secret vs. a `direct_pg` customer's own DBA-administered database).

**Generation timing**: at Slice 01 registration (`POST
.../oauth_providers/google`), not lazily at first sign-in. A fresh Ed25519
keypair is generated, AES-256-GCM-encrypted, and inserted via the identical
`INSERT ... ON CONFLICT (project_id) DO NOTHING RETURNING` + fallback
`SELECT` idempotency shape `enable_hosted_identity` already established — a
Client-ID redefinition (AC-19-02) never regenerates or touches the signing
key. The provider-credential upsert and the signing-key idempotent-insert
are wrapped in one `SystemDb` transaction so a partial failure can never
leave a project with a registered Client ID but no signing key to mint with.

```sql
CREATE TABLE oauth_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,        -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,        -- AES-256-GCM: 12-byte nonce || ciphertext, key = EMBYR_ENCRYPTION_KEY
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ(6) NOT NULL DEFAULT now()
);
```

Not keyed by `provider` — one embyr-owned signing key per project serves
every current and future "embyr independently mints" OAuth-derived identity,
regardless of which provider authenticated the end user (the claims shape
minted is identical either way — `sub`/`aud`/`exp`). A future GitHub slice
reuses this SAME table with zero new migration.

## Decision 3 — Storage: `oauth_provider_credentials` (System DB)

```sql
CREATE TABLE oauth_provider_credentials (
    project_id  VARCHAR(63)    NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider    TEXT           NOT NULL,
    client_id   TEXT           NOT NULL,
    created_at  TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, provider)
);
```

`client_id` stored plaintext — it is public, non-confidential (Resolution
1: "nothing confidential... mirrors `client_identity_credentials`' own
no-confidentiality-property public key"). `provider` is a real column (not
hardcoded) so a future GitHub slice adds rows, not a schema migration — but
v1's only writer is the `/oauth_providers/google` endpoint (§ Decision 4),
so no non-`"google"` value can reach this table today; no `CHECK` constraint
is added (YAGNI — the endpoint path IS the constraint; a `CHECK` would need
altering the moment a second provider ships, for zero present benefit).

Confirms Resolution 5's own storage lean: System DB, mirroring
`client_identity_credentials`' placement exactly.

## Decision 4 — Driving Port, Slice 01: Admin `:9090`, Provider-in-Path

```
POST /admin/v1/projects/:project_id/oauth_providers/google
  Body:  { "client_id": "123456789-abc.apps.googleusercontent.com" }
  201:   { "project_id": "...", "provider": "google", "client_id": "...", "created_at": "..." }  -- first registration
  200:   (identical body shape) -- idempotent redefine (AC-19-02)
  401:   (session middleware — missing/invalid Bearer, AC-19-03, unchanged shape)
  403:   Viewer role (mirrors register_client_identity_credential's in-handler gate)
  404:   project does not exist / deleted / owned by a different account (AC-19-04)
```

**Why provider-in-path, not provider-in-body**: DISCUSS's own § System
Constraints instructs DESIGN not to "silently expand into GitHub." A body
field `{"provider": "google", ...}` would accept (and 400-reject) arbitrary
provider strings — implying a generic, extensible surface DISCUSS
explicitly did not authorize. A literal path segment
(`/oauth_providers/google`) makes v1's Google-only scope a structural fact of
the route table (any other segment 404s via ordinary routing, never reaches
handler logic) — the same discipline Resolution 1's own "ship concrete
mechanisms before generalizing" reasoning already established for this
feature. A future GitHub slice adds a sibling route
(`/oauth_providers/github`), not a body-field branch.

No `api_key` field required (§ Decision 2's positive consequence) — Owner/Admin
session auth and project ownership are the only preconditions, mirroring
`register_client_identity_credential`'s exact role-gate shape, simpler than
`enable_hosted_identity`'s (no ECIES, no api_key verification step).

Handler composes: `verify_project_ownership` (EXTEND, reused unchanged) +
role gate (EXTEND, reused unchanged pattern) + one new transactional
`SystemDb` call (`register_oauth_provider`, new) that performs the
provider-credential UPSERT and the signing-key idempotent-INSERT together.

## Decision 5 — Google ID-Token Verification: New Pure Module `embyr_core::oauth_identity`

Google ID-token verification is **not** `client_identity`'s mirror
operation the way hosted-identity's minting was (ADR-036 Decision 3):
different algorithm (RS256, not EdDSA), different credential source
(Google's own live JWKS, not a single registered/embyr-owned key), different
claims shape (`iss`/`aud`/`sub`/`exp`, Google's own, not
`ClientIdentityClaims`). Folding it into `client_identity` would repeat the
exact "structurally different concern crammed into an unrelated module"
mistake ADR-036 Decision 9 already reasoned through when it gave
`validate_password_strength` its own module rather than forcing it into
`client_identity` or `auth::argon2`. **A new, small, pure (zero-IO) module is
justified** — the identical reasoning, applied fresh.

```rust
// crates/embyr-core/src/oauth_identity.rs

pub struct VerifiedOAuthIdentity {
    pub sub: String,
    pub email: Option<String>,
    pub expires_at_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthIdentityVerifyError {
    MissingToken,
    Malformed,        // bad structure, unknown kid, signature fails under every JWKS key
    Expired,
    AudienceMismatch,  // aud != the project's registered Client ID
}

pub fn verify_google_id_token(
    id_token: Option<&str>,
    expected_client_id: &str,
    jwks: &jsonwebtoken::jwk::JwkSet,
) -> Result<VerifiedOAuthIdentity, OAuthIdentityVerifyError>;

/// Resolution 3(B): deterministic, stateless end_user_id derivation.
pub fn derive_end_user_id(provider: &str, subject: &str) -> String {
    format!("{provider}:{subject}")
}
```

Reuses the exact RS256/JWKS primitives `oidc_callback` already proved out
(`jsonwebtoken::DecodingKey::from_jwk`, `kid` matching,
`Validation::new(Algorithm::RS256)`) — as a **pattern**, not shared code
(`oidc_callback`'s own flow is per-account, session-shaped, implicit-flow;
this function is pure, per-request, project-scoped — DISCUSS's own reading
already confirmed no code-level reuse is possible, only the primitives).
`jsonwebtoken` performs signature verification before trusting `exp`/`aud`
claims (library-level guarantee, mirrors `verify_client_identity_token`'s own
signature-first discipline) — a token with an invalid signature is always
`Malformed`, never `Expired`/`AudienceMismatch`, regardless of what the
(unverified) claim values say.

`derive_end_user_id("google", sub)` gives AC-19-06 ("the same Google account
signing in again resolves to the identical `end_user_id`") for free — pure
function of Google's own stable `sub` claim, no storage, no lookup.

## Decision 6 — Driving Port, Slice 02: REST `:8081`, No `?key=`

```
POST /v1/projects/{project_id}/accounts:signInWithIdp
  Body:  { "idToken": "<google-issued ID token>" }
  200:   { "localId": "google:<sub>", "idToken": "<embyr-minted token>", "expiresIn": "<seconds>" }
  400:   { "reason": "MISSING_ID_TOKEN" | "GOOGLE_SIGN_IN_NOT_ENABLED"
                    | "INVALID_ID_TOKEN" | "ID_TOKEN_EXPIRED" | "AUDIENCE_MISMATCH" }
  503:   { "reason": "GOOGLE_JWKS_UNREACHABLE" }
```

Endpoint name (`accounts:signInWithIdp`) mirrors `OQ-OAP-01`'s own naming
hint (real Identity Toolkit's generic third-party-IdP sign-in verb) — exact
wire shape (whether the Firebase JS SDK can be pointed at this shape at all)
remains `OQ-OAP-01`'s own required pre-DELIVER spike (§ Open Questions,
unchanged from DISCUSS).

**400, not 401, for every verification-failure reason** — mirrors
`rest/sign_in.rs::sign_in_with_custom_token`'s own established convention
(the "token in the body IS the credential, stateless" REST family; confirmed
by direct read: that handler returns `StatusCode::BAD_REQUEST` for
`MissingToken`/`Malformed`/`Expired`/`ProjectMismatch` alike), not
`sign_in_with_password.rs`'s 401 (that shape is oracle-protected
password-auth-specific — a materially different mechanism this feature does
not share). `503`, not `400`, for JWKS-unreachable — a genuinely different
failure class (infrastructure availability, not a caller-supplied-bad-token
case), and matches AC-19-11's own "retryable-sounding" language more
precisely than a 4xx would.

**No `?key=` query parameter** — confirms Slice 02's own brief lean. Neither
Customer DB resolution (Resolution 3(B), stateless) nor signing-key
decryption (§ Decision 2, `EMBYR_ENCRYPTION_KEY`-based, not `api_key`-based)
requires it.

New composition-root state, mirroring `HostedIdentityState`'s minimality
discipline:

```rust
pub struct OAuthProviderState {
    pub system_db: Arc<SystemDb>,
    pub encryption_key: [u8; 32],
    pub encryption_key_previous: Option<[u8; 32]>,
    pub google_jwks_cache: Arc<GoogleJwksCache>,
}
```

Wired into the SAME `AccountsBridgeState` / `accounts_bridge_dispatch`
mechanism `signUp`/`signInWithPassword`/`sendOobCode`/`resetPassword`
already use (`crates/embyr-server/src/lib.rs`) — a fifth `match` arm,
`"signInWithIdp"`, added to the existing dispatcher; no new listener, no new
route-registration mechanism (the `matchit`-conflict reasoning that forced
the single-shared-capture-name dispatcher design already documented in
`lib.rs` applies identically here).

## Decision 7 — JWKS Fetch/Cache Adapter, and Its Earned-Trust Answer

`crates/embyr-server/src/adapters/google_jwks_cache.rs` (new): fetches
`https://www.googleapis.com/oauth2/v3/certs`, caches for a **fixed 6-hour
TTL**, with a **bounded 5-second request timeout** — unlike `oidc_callback`'s
existing `reqwest::get(&jwks_url)` call, which has no timeout at all today (a
real, pre-existing gap this feature does not inherit).

**Earned Trust answer for this adapter** (per this feature's own security
boundary — verifying a third-party-issued token): the fault-injection
scenarios this adapter must survive — DNS failure, TCP connect timeout/
refusal, TLS handshake failure (including a corporate MITM proxy substituting
a certificate chain), HTTP non-2xx, HTTP 200 with a malformed/schema-invalid
body, and a response slower than the 5s bound — all fold into the SAME
distinguishable `GOOGLE_JWKS_UNREACHABLE` rejection (AC-19-11): never a
crash, never a silently-accepted unverified token. This codebase has no
existing `probe()`/composition-root wire-then-probe enforcement mechanism
(no adapter read during this feature's own reuse analysis —
`AwsSecretFetcher`, `GcpSecretFetcher`, `CredentialCache` alike — implements
one); introducing a first-of-its-kind three-layer ArchUnit-style enforcement
framework for a single new adapter would be a disproportionate, unjustified
new mechanism for this codebase's own established practice. Instead, AC-19-11
itself is this adapter's enforcement mechanism — a required acceptance
scenario simulating JWKS-unreachable, functioning as the "gold test" Earned
Trust doctrine calls for, at the granularity this codebase's own testing
discipline already uses (Gherkin ACs, not framework-level probes).

**Deliberately not probed at server startup**: Google's reachability is a
per-request soft dependency (only Slice 02's sign-in path needs it) — the
server must start and serve every OTHER capability (Firestore, admin,
hosted-identity, custom-token) regardless of Google's live status. Gating
startup on Google's reachability would be a strictly worse availability
posture than today's, for no benefit.

**Simplification named explicitly (ponytail discipline)**: fixed 6-hour TTL,
no `Cache-Control` header parsing, no stale-serve-on-fetch-failure fallback.
Ceiling: a Google key rotation inside the TTL window is not observed until
the next refetch — never a security hole (a rotated-out key simply fails
signature verification, ordinary `Malformed`), only a bounded availability
corner case. Upgrade path: parse `Cache-Control: max-age` if operational data
ever shows this ceiling causing real sign-in failures.

## Decision 8 — Verification-Time Credential Routing: a Third Widening of `attach_client_identity_if_present`

**Load-bearing finding, not optional.** AC-19-05 requires "her subsequent
Firestore call succeeds carrying her verified identity" and AC-19-10 is a
regression guardrail on the SAME code path. `attach_client_identity_if_present`
(`crates/embyr-server/src/grpc/handler.rs`) was already widened once by
ADR-036 Decision 4 (`client_identity_credentials` → `hosted_identity_signing_keys`
fallback chain). It must be widened a **third** time to also try
`oauth_signing_keys`, or a Google-signed-in end user's subsequent `getDoc`
call — the walking skeleton's own required scenario — silently fails to
attach identity, an unnoticed regression against the feature's own North
Star KPI.

The current function's tail short-circuits on the second (hosted-identity)
attempt via `.ok()??` (returns `None` immediately on lookup failure or
absence) — this shape must change from "hard-return on absence" to
"fall-through on absence," matching the FIRST attempt's own
`if let Some(row) = ...` shape, so a third attempt can follow:

```
[4] x-embyr-client-identity present?
      absent → unchanged, zero calls into embyr_core::client_identity (AC-16-08(c) preserved)
      present →
        client_identity_credentials row exists? verify → success: DONE / failure or absent: fall through
        hosted_identity_signing_keys row exists?  verify → success: DONE / failure or absent: fall through
        oauth_signing_keys row exists?             verify → success: DONE / failure or absent: fall through
        none matched / all failed → attach nothing (UNCHANGED failure behavior — never rejects the request)
```

All three attempts call the **identical, unchanged** `verify_client_identity_token()`
— the same "try credential source A, then B, then C" widening ADR-036
Decision 4 already normalized one level up from ADR-025's own
current/previous-key retry. For a project with none of the three tables
populated (every project that uses no identity mechanism, or only
`api_key`), all three lookups return `None` and step 4 reduces to its
pre-`client-auth` behavior — the "physically unreachable, not merely
unexercised" structural guarantee (ADR-026, extended by ADR-036, extended
again here) holds a third time.

**Enforcement**: unit test — a project with only `oauth_signing_keys`
populated verifies an oauth-minted token and rejects a
`client-identity`/hosted-identity-shaped forgery as `Malformed`; a project
with all three sources populated verifies each token type correctly
regardless of check order; a project with none never queries any of the
three tables for a request with no `x-embyr-client-identity` header
(structural-unreachability regression test, third iteration).

## Consequences

### Positive

- Zero changes to `verify_client_identity_token()`, zero changes to
  `mint_client_identity_token()`, zero new crate dependency (`jsonwebtoken`,
  `reqwest`, `aes-gcm` — all already workspace dependencies).
- Slice 01's registration handler needs no `api_key` field — avoids the
  exact "gap found and closed during pre-DELIVER review" class of defect
  ADR-036 Decision 5 had to patch reactively; this feature avoids it by
  construction (§ Decision 2).
- `oauth_signing_keys` is structurally disjoint from BOTH
  `client_identity_credentials` (hard constraint) AND
  `hosted_identity_signing_keys` (DESIGN's own reasoned rejection of reuse) —
  Google sign-in works for a project that has never touched hosted identity.
- Slice 02 needs no `?key=` query parameter — a genuinely stateless,
  Customer-DB-free data-plane endpoint, the simplest of the three Identity-track
  sign-in shapes.

### Negative / Trade-offs

- `attach_client_identity_if_present` now tries up to three credential
  sources sequentially on every request carrying `x-embyr-client-identity` —
  bounded, cheap (Ed25519 verify is microseconds, ADR-024 § Consequences),
  but a third sequential DB lookup in the worst case (a project using none of
  the three mechanisms for a given token never reaches this cost, since the
  header itself must be present first).
- `oauth_signing_keys` and `hosted_identity_signing_keys` are now two,
  independently-encrypted (different key-derivation source), independently-keyed
  (by `project_id` in both, but disjoint tables) embyr-owned signing-key
  stores. A future feature needing a THIRD "embyr mints its own token" flow
  should evaluate reusing `oauth_signing_keys`'s AES-256-GCM/
  `EMBYR_ENCRYPTION_KEY` shape (the more broadly reusable of the two, since it
  has no Customer-DB-derived precondition) rather than defaulting to a fourth
  table.
- `OQ-OAP-01` (SDK wire-format: whether `signInWithPopup()`/`GoogleAuthProvider`
  can be pointed at `accounts:signInWithIdp`'s shape) remains empirically
  unconfirmed — required pre-DELIVER spike, mirroring `OQ-CA-01`/`OQ-CHI-01`'s
  own precedent, not a blocker to this ADR's logical contract.

## References

- `docs/feature/oauth-providers/feature-delta.md` §§ Job Discovery Framing
  Resolution, System Constraints, Handoff Package
- `docs/product/architecture/adr-002-bounded-contexts.md` § Option D, §
  Changed Assumptions
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
- `docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`
- `docs/product/architecture/adr-036-hosted-identity-bounded-context-and-storage.md`
  (direct structural precedent for this ADR's own bundling, storage-split-style
  reasoning, and verification-time-routing-widening methodology)
- `docs/product/architecture/adr-018-secrets-management.md` (`EMBYR_ENCRYPTION_KEY`,
  `decrypt_with_rotation`)
- `crates/embyr-core/src/client_identity/mod.rs`,
  `crates/embyr-core/src/hosted_identity.rs` (new sibling:
  `crates/embyr-core/src/oauth_identity.rs`)
- `crates/embyr-server/src/admin/handlers/{client_identity,hosted_identity,oidc_providers,shared}.rs`,
  `crates/embyr-server/src/rest/{sign_in,sign_up,sign_in_with_password}.rs`,
  `crates/embyr-server/src/adapters/{encryption,project_auth}.rs`,
  `crates/embyr-server/src/grpc/handler.rs::attach_client_identity_if_present`,
  `crates/embyr-server/src/lib.rs::accounts_bridge_dispatch`

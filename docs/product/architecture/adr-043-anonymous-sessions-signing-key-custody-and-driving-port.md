# ADR-043: Anonymous Sessions — Signing-Key Custody, Stateless Minting,
# Driving-Port Composition, and a Fourth Widening of Verification-Time
# Credential Routing

## Status

Accepted

## Context

`anonymous-sessions` (JOB-20) adds a fourth identity-establishment mechanism
— a real, server-issued `VerifiedEndUserIdentity` obtainable with **zero**
prior credential — alongside `client-auth`'s customer-minted custom tokens
(JOB-16, ADR-024/025/026), `client-auth-hosted-identity`'s embyr-hosted
email/password (JOB-18, ADR-036), and `oauth-providers`' Google sign-in
(JOB-19, ADR-037). DISCUSS (`docs/feature/anonymous-sessions/feature-delta.md`)
locked two user stories (US-01 enable, US-02 sign-in) with a full AC set and
left two escalations plus three recommendations for DESIGN. This ADR resolves
the recommendations (Resolutions 2/3/4) as one coherent design, mirroring
ADR-036's and ADR-037's own bundling precedent. The two escalations
(refresh-token/TTL; `backend_mode=agent` gating) are resolved in their own
ADRs — ADR-045 and ADR-044 respectively — per this session's explicit
instruction to give each its own decision record.

## Decision Drivers

1. **Resolution 2's own high-confidence recommendation, grounded in ADR-037's
   own forward-looking guidance**: ADR-037 § Consequences names its
   `oauth_signing_keys` AES-256-GCM/`EMBYR_ENCRYPTION_KEY` shape, verbatim,
   as the reference class for "a future feature needing a THIRD ['embyr
   mints its own token'] flow." This is that feature.
2. **Resolution 3's stateless-minting recommendation** — no Customer DB
   Account row, tied to linking/upgrade being out of scope for this slice.
3. **Resolution 4's structural-separation requirement** — same
   `accounts:signUp`-compatible wire aim as hosted-identity's own signup,
   but a genuinely separate handler/enablement gate, never literally
   extending `sign_up.rs`.
4. **Zero changes to `mint_client_identity_token()` / `verify_client_identity_token()`**
   — both remain byte-for-byte unchanged; this feature is purely a fourth
   caller/verifier source.
5. **Ground-truth verification, not DISCUSS-inertia**: every reuse claim
   below was confirmed by directly reading the actual shipped code
   (`crates/embyr-server/src/admin/handlers/{hosted_identity,oauth_providers}.rs`,
   `crates/embyr-server/src/rest/{sign_up,sign_in_with_idp}.rs`,
   `crates/embyr-server/src/adapters/{system_db,encryption}.rs`,
   `crates/embyr-server/src/grpc/handler.rs::attach_client_identity_if_present`,
   `crates/embyr-server/src/lib.rs::accounts_bridge_dispatch`), not assumed
   from the ADR-036/037 prose alone.

## Decision 1 — Bounded-Context Placement: No New Context, Extends BC-1

Applying ADR-002's Option-D three-part test fresh (per this codebase's own
established discipline, not by inertia) to the one candidate entity this
feature introduces:

| Test | `anonymous_signing_keys` row `(project_id)` |
|---|---|
| Entity with identity of its own? | Yes — `project_id`, identical shape to `oauth_signing_keys`/`hosted_identity_signing_keys`. |
| Lifecycle of its own? | Thin — create (enable) only. No redefine, no rotation, no disable in this feature's locked v1 scope. Thinner than `OAuthProviderCredential`'s own thin register/redefine lifecycle (ADR-037 Decision 1). |
| Invariants of its own? | Thin — one row per project, uniqueness enforced by `PRIMARY KEY(project_id)`. No content invariant (no `client_id`, no password, no email) at all. |

This is thinner than `oauth_signing_keys`'s own already-thin BC-1 extension
(ADR-037 Decision 1, itself already confirmed as "no new BC-6"). **Decision:
extends BC-1 Tenant Management, identical to `oauth_signing_keys`'s own
placement. No new bounded context.** `docs/product/architecture/adr-002-bounded-contexts.md`
§ Changed Assumptions is appended with a short note recording the Option-D
test applied fresh a fourth time, again not producing a new context — direct
evidence of per-case application, not inertia (mirrors ADR-037's own framing
of its own third application).

## Decision 2 — Storage: `anonymous_signing_keys` (System DB), AES-256-GCM
Under `EMBYR_ENCRYPTION_KEY` — Confirms Resolution 2

**Confirms Resolution 2 as recommended, no deviation.** Ground-truth
evidence, not the ADR-037 prose alone: `crates/embyr-server/src/admin/handlers/oauth_providers.rs`
was read in full. `register_google_oauth_provider` generates a fresh Ed25519
key with `SigningKey::generate(&mut OsRng)`, encrypts the seed inline with
`Aes256Gcm::new_from_slice(&state.encryption_key)` (12-byte random nonce
prefix), and stores it via a transactional idempotent INSERT — exactly the
shape ADR-037 Decision 2 documents. Anonymous sign-in's own precondition
profile is identical to oauth's, not hosted-identity's: under Resolution 3
(stateless), this feature never resolves a Customer DB connection, so it
never carries an `api_key` on its data-plane call for an unrelated reason —
the exact precondition ECIES needs and does not have here (the same gap
ADR-037 Decision 2 already named for oauth). AES-256-GCM under
`EMBYR_ENCRYPTION_KEY` has no such precondition — it is server-wide,
threaded through composition already, zero new secret to provision.

```sql
-- migrations/0031_anonymous_signing_keys.sql
CREATE TABLE anonymous_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,        -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,        -- AES-256-GCM: 12-byte nonce || ciphertext, key = EMBYR_ENCRYPTION_KEY
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Byte-for-byte identical shape to `oauth_signing_keys` (ADR-037 Decision 2),
deliberately a **fourth, structurally disjoint** table — never colocated
with `client_identity_credentials`, `hosted_identity_signing_keys`, or
`oauth_signing_keys` (Resolution 2's own hard constraint, satisfied
trivially, mirroring `oauth_signing_keys`'s own disjointness from
`hosted_identity_signing_keys`). **Rejected**: reusing
`hosted_identity_signing_keys` — identical rejection reasoning to ADR-037's
own Option (A) rejection (undocumented cross-feature coupling: a
Google-only or anonymous-only project must not be forced to also enable
hosted identity). **Rejected**: reusing `oauth_signing_keys` itself — despite
the identical encryption shape, sharing the table would let an anonymous
sign-in verify against a key generated for Google sign-in (or vice versa),
collapsing two independently-toggleable providers' custody boundaries into
one row; the *shape* is reusable, the *table* is not, mirroring exactly why
`oauth_signing_keys` itself was not folded into `hosted_identity_signing_keys`
despite sharing no content-level need for separate columns.

**Generation timing**: at US-01 enablement, not lazily at first sign-in —
mirrors `register_oauth_provider`'s identical timing decision (ADR-037
Decision 2).

## Decision 3 — Admin Enablement Handler: New File, No `api_key` Field, No
`backend_mode` Check (ADR-044)

`crates/embyr-server/src/admin/handlers/anonymous_identity.rs` (new):

```
POST /admin/v1/projects/:project_id/anonymous_identity/enable
  Body:  {}  -- no fields at all; no api_key (Decision 2's positive
              consequence, mirrors ADR-037 Decision 2's identical
              "no api_key field" outcome), no client_id, nothing
  201:   { "project_id": "...", "algorithm": "EdDSA", "created_at": "..." }
  401:   session middleware — missing/invalid Bearer (unchanged shape)
  403:   Viewer role (mirrors enable_hosted_identity's/
         register_google_oauth_provider's identical in-handler gate)
  404:   project does not exist / deleted / owned by a different account
```

Composes **exclusively pre-existing, already-independently-callable
primitives** — `SessionContext` extractor (401), `verify_project_ownership`
(404/403, reused unchanged), `SigningKey::generate(&mut OsRng)` +
`Aes256Gcm` inline encryption (reused pattern from `oauth_providers.rs`,
not extracted into a shared helper — the identical "duplicated shape, not
primitives, judged cheaper than a shared abstraction" reasoning ADR-036
Decision 7 already applied once). No `backend_mode` lookup anywhere in this
handler — see ADR-044 for the full reasoning; ground-truth confirmed by
reading `oauth_providers.rs` in full, which has none either.

Response mirrors `EnableHostedIdentityResponse`'s exact narrow shape (no
`public_key`, nothing derived from signing material — AC-16-01's "never
echo raw material" discipline, applied a fourth time) rather than
`RegisterGoogleOAuthProviderResponse`'s shape (which additionally echoes
`client_id`, a field this feature has none of).

**Idempotency (AC-20-02)**: `enable_anonymous_identity` (new `SystemDb`
method) mirrors `enable_hosted_identity`'s exact
`INSERT ... ON CONFLICT (project_id) DO NOTHING RETURNING` + fallback
`SELECT` shape — not `register_oauth_provider`'s `xmax = 0`/201-vs-200
dance, because there is no redefinable field (no `client_id` to update) to
distinguish first-registration from redefinition; every enablement call is
either "insert a fresh row" or "return the existing row unchanged," and
AC-20-01 locks 201 for both (mirrors `enable_hosted_identity`'s own
always-201 convention exactly, not oauth's own 201-vs-200 convention).

## Decision 4 — Stateless Minting: No Customer DB, No Account Row — Confirms
Resolution 3

**Confirms Resolution 3 as recommended, no deviation.** `end_user_id` is
`uuid::Uuid::new_v4()`, generated in Rust at sign-in time, never persisted
anywhere. This is the SAME UUIDv4 scheme `hosted_identity_accounts.end_user_id`
already uses (there, via Postgres's `gen_random_uuid()` DEFAULT; here, no row
exists to default it from, so it is generated directly) — collision
probability is the identical, already-accepted UUIDv4 birthday-bound this
codebase already relies on for `hosted_identity_accounts`, not a new
statistical risk (AC-20-08). No Customer DB adapter is ever resolved by this
feature — `resolve_customer_db_adapter` (ADR-036 Decision 7) is never called,
confirmed by this ADR's own driving-port design (Decision 5) needing no
Customer DB connection at any step.

**Rejected**: persisting an Account row mirroring `hosted_identity_accounts`
— identical rejection reasoning to Resolution 3's own DISCUSS analysis
(linking/upgrade is out of scope for this slice; no admin listing exists to
serve; a persisted row buys this feature nothing while reopening the exact
storage-growth/cleanup concern the raw ask named as a real cost).

## Decision 5 — Driving Port: REST `:8081`, `accounts:signUp`-Compatible Aim,
Structurally Separate Handler — Confirms Resolution 4, Resolves `OQ-AS-01`'s
Dispatch Mechanics

**Confirms Resolution 4's locked shape**: same URL-shape aim as
hosted-identity's `accounts:signUp` (per real Firebase's own overload of
that endpoint for `signInAnonymously()`), but `crates/embyr-server/src/rest/sign_in_anonymously.rs`
is a **new file**, never adding a branch inside `sign_up.rs::sign_up`'s own
function body — that handler, its `HostedIdentityState`, and its
`hosted_identity_not_enabled()` gate remain untouched end to end, exactly as
Resolution 4 requires.

```
POST /v1/projects/{project_id}/accounts:signInAnonymously?key={api_key}
  (provisional path — see "Dispatch mechanics" below for the OQ-AS-01-
   contingent alternative)
  200:   { "localId": "<end_user_id>", "idToken": "<embyr-minted token>", "expiresIn": "<seconds>" }
  400:   { "reason": "ANONYMOUS_AUTH_NOT_ENABLED" }
  401:   { "reason": "INVALID_API_KEY" }
```

**Why `?key=` is REQUIRED, structurally, not optional-but-conventional**
(resolves the DISCUSS-flagged "DESIGN's call" — AC-20-07 already locks this
as required, this ADR records the reasoning): anonymous sign-in is the
**only** one of the four identity mechanisms presenting literally zero
credential of any kind — a custom token needs a valid signature; hosted
identity needs a correct password; Google sign-in needs a real Google
consent flow and ID token. `signInWithIdp` (ADR-037 Decision 6) can safely
omit `?key=` precisely because the Google ID token itself is the admission
credential. Anonymous sign-in has no equivalent — `?key=` is the **only**
admission bar this endpoint has, making it structurally required here even
though (per Decision 2 above) it is not decryption-load-bearing the way it
is for hosted-identity's signup (ADR-036 Decision 6). Verified via
`SystemDb::get_project_for_auth` + `embyr_core::auth::argon2::verify_api_key`
— the identical two primitives `resolve_customer_db_adapter` and
`enable_hosted_identity` already use for the "identify the caller" step —
called directly, inline, with **no** Customer DB adapter constructed (unlike
`resolve_customer_db_adapter`, which this feature deliberately does not
call at all — Decision 4).

**200, not 201**: mirrors `sign_in_with_idp`'s own convention (a "mint and
go," nothing-created REST semantics), not `sign_up`'s 201 (which creates a
real, persisted Account row this feature deliberately has none of).

**Response shape**: `AnonymousSignInSuccessResponse { local_id, id_token,
expires_in }` — mirrors `SignInWithIdpSuccessResponse`'s exact three-field
shape (no `email` field, unlike `SignUpSuccessResponse` — there is no email
here).

**Dispatch mechanics — the genuinely open piece, contingent on `OQ-AS-01`**:
`accounts_bridge_dispatch` (`crates/embyr-server/src/lib.rs`) currently
matches on the literal `action` segment captured from the URL
(`signUp`/`signInWithPassword`/`sendOobCode`/`resetPassword`/`signInWithIdp`).
DISCUSS's own recollection (moderate confidence, `OQ-AS-01`, required
pre-DELIVER spike) is that real Firebase's `signInAnonymously()` may POST to
the SAME `accounts:signUp` action segment as hosted-identity's signup, with
`email`/`password` both omitted — not a distinct action verb. This ADR locks
the LOGICAL contract above regardless of the spike's outcome (mirroring
`OQ-CA-01`/`OQ-CHI-01`/`OQ-OAP-01`'s own established "not a blocker to the
ADR's logical contract" precedent) and specifies both dispatch mechanics so
DELIVER implements the correct one without re-deriving this decision:

- **If `OQ-AS-01` confirms a shared `accounts:signUp` action verb**:
  `accounts_bridge_dispatch`'s existing `"signUp"` match arm gains one
  minimal, structural peek at the raw JSON body — BEFORE constructing
  `SignUpBody` — checking whether both `email` and `password` are
  absent/null. If so, dispatch to `sign_in_anonymously` (its own state, its
  own gate) INSTEAD of `rest::sign_up::sign_up`. This changes ONLY the
  dispatcher function in `lib.rs` — Resolution 4's rejection was specifically
  about not adding a conditional INSIDE `sign_up`'s own function body/gate,
  which this preserves exactly (`sign_up()` gains zero lines, zero new
  branches, remains hosted-identity-only).
- **If `OQ-AS-01` confirms (or DELIVER's own empirical check shows) a
  distinct action verb** (the simpler, default case): a sixth
  `accounts_bridge_dispatch` match arm, `"signInAnonymously"`, calling the
  new handler directly — mirrors `"signInWithIdp"`'s own addition exactly
  (one new match arm, one new state field, zero changes to any existing arm).
  **This is DESIGN's recommended default** if the spike is inconclusive by
  DELIVER time — the lower-risk, more conventional REST shape, and the one
  requiring zero changes to any existing dispatch arm.

Either branch: the handler function itself, `AnonymousIdentityState`, and
the enablement/api-key checks inside it are byte-identical — only the
dispatch entry point differs.

**New composition-root state**, mirroring `OAuthProviderState`'s minimality
discipline (no JWKS cache needed — there is no third-party token to verify):

```rust
pub struct AnonymousIdentityState {
    pub system_db: Arc<SystemDb>,
    pub encryption_key: [u8; 32],
    pub encryption_key_previous: Option<[u8; 32]>,
}
```

Wired into the SAME `AccountsBridgeState`/`accounts_bridge_dispatch`
mechanism every other `accounts:<verb>` endpoint already uses — a new field,
threaded through `spawn_all_servers` reusing the SAME `encryption_key`/
`encryption_key_previous` parameters oauth's own state already threads (no
new key material, no new `Cargo.toml` entry).

## Decision 6 — Verification-Time Credential Routing: a Fourth Widening of
`attach_client_identity_if_present`

**Load-bearing, not optional** — identical reasoning to ADR-037 Decision 8's
own "load-bearing finding." AC-20-05/AC-20-09 require Maria's subsequent
Firestore call to carry her verified identity and be evaluated identically
to any other identity. Ground-truth read of
`crates/embyr-server/src/grpc/handler.rs::attach_client_identity_if_present`
confirms its current shape already fall-throughs cleanly across three
sources (`client_identity_credentials` → `hosted_identity_signing_keys` →
`oauth_signing_keys`, each an `if let Some(row) = ... { ... }` block that
falls through on `None`/verification failure). A **fourth**, identically
shaped block is appended, trying `anonymous_signing_keys`:

```
[4] x-embyr-client-identity present?
      absent → unchanged, zero calls into embyr_core::client_identity (AC-16-08(c) preserved)
      present →
        client_identity_credentials row exists?    verify → success: DONE / else: fall through
        hosted_identity_signing_keys row exists?    verify → success: DONE / else: fall through
        oauth_signing_keys row exists?               verify → success: DONE / else: fall through
        anonymous_signing_keys row exists?            verify → success: DONE / else: fall through
        none matched / all failed → attach nothing (UNCHANGED failure behavior — never rejects the request)
```

All four attempts call the **identical, unchanged** `verify_client_identity_token()`
— the fourth iteration of the same "try credential source A, then B, then C,
then D" widening ADR-036 Decision 4 and ADR-037 Decision 8 already
established and normalized. For a project with none of the four tables
populated, all four lookups return `None` and step 4 reduces to its
pre-`client-auth` behavior — the "physically unreachable, not merely
unexercised" structural guarantee holds a fourth time (KPI #2's own
regression guardrail).

**Enforcement**: unit test — a project with only `anonymous_signing_keys`
populated verifies an anonymous-minted token and rejects a
`client-identity`/hosted-identity/oauth-shaped forgery as `Malformed`; a
project with all four sources populated verifies each token type correctly
regardless of check order; a project with none never queries any of the
four tables for a request with no `x-embyr-client-identity` header
(structural-unreachability regression test, fourth iteration).

## Decision 7 — REST Rate-Limiting: a Pre-Existing Gap This Feature Does Not
Introduce, But Materially Sharpens

DISCUSS's own § System Constraints claims anonymous sign-in abuse is
"bounded today only by this codebase's existing generic per-project
request-rate limiting (JOB-11, distributed token bucket)," and asked DESIGN
to judge whether that limit is a sufficient v1 mitigation. **This claim does
not hold, and this ADR corrects it with evidence rather than accept it.**
`grep` across `crates/embyr-server/src/rest/` for `RateLimiter`/`rate_limit`
returns **zero matches** — `middleware::rate_limit::RateLimiter` is called
exclusively from `crates/embyr-server/src/grpc/handler.rs` (gRPC data-plane
methods: `GetDocument`, `RunQuery`, etc.). None of the five `accounts:<verb>`
REST endpoints (`signUp`, `signInWithPassword`, `signInWithCustomToken`,
`signInWithIdp`, and this feature's own `signInAnonymously`) are rate-limited
today, at all — a pre-existing gap across the entire REST identity surface,
not one this feature introduces (mirrors ADR-036 Decision 11's identical
"pre-existing gap, not a gap this feature introduces" finding for REST
observability).

**v1 decision**: do **not** build a bespoke rate limiter scoped only to
`signInAnonymously` — instrumenting one of five identically-exposed REST
endpoints while the other four (including the two that already mint
embyr-owned tokens) stay unlimited would produce an inconsistent, arbitrarily
partial mitigation, the identical reasoning ADR-036 Decision 11 already used
to reject partial REST observability. **Named explicitly as an elevated,
not-fully-mitigated risk specific to this feature** (unlike Decision 5's
`?key=` requirement, which is a real but partial bar — anonymous sign-in
still requires strictly less proof of identity than any of the other three
mechanisms, since a valid `api_key` is the only gate and no per-call secret
of the caller's own is checked). **Recommended follow-up, not built here**:
extend `middleware::rate_limit`'s existing Postgres-backed distributed
token-bucket mechanism (ADR-015) to wrap the REST `accounts:<verb>` bridge
generally — a cross-cutting initiative spanning all five endpoints, not just
this feature's own new one, mirroring ADR-036 Decision 11's identical
"REST-wide, not per-feature" framing for observability.

## Consequences

### Positive

- Zero changes to `verify_client_identity_token()`, zero changes to
  `mint_client_identity_token()`, zero changes to `sign_up.rs::sign_up`'s
  own function body, zero new crate dependency (`aes-gcm`, `uuid`,
  `ed25519-dalek` all already workspace dependencies).
- `anonymous_signing_keys` is structurally disjoint from all three existing
  signing-key tables — anonymous sign-in works for a project that has never
  touched hosted identity or Google sign-in.
- No `api_key` field in the enablement request body — avoids the exact
  "gap found and closed during pre-DELIVER review" class of defect ADR-036
  Decision 5 had to patch reactively; this feature avoids it by
  construction, identically to how `oauth-providers` already avoided it
  (ADR-037 Decision 2's own positive consequence, confirmed shipped).
- This feature needed zero new abstractions: every primitive it composes
  (`SigningKey::generate`, inline `Aes256Gcm` encryption,
  `decrypt_with_rotation`, `verify_api_key`, `get_project_for_auth`,
  `mint_client_identity_token`) already exists, already proven by three
  prior identity features.

### Negative / Trade-offs

- `attach_client_identity_if_present` now tries up to four credential
  sources sequentially on every request carrying `x-embyr-client-identity`
  — bounded, cheap (Ed25519 verify is microseconds), a fourth sequential DB
  lookup in the worst case only for a project using none of the four
  mechanisms for a given token (which never reaches this cost, since the
  header itself must be present first).
- `OQ-AS-01` (SDK wire-format: whether the real Firebase JS SDK's
  `signInAnonymously()` reuses `accounts:signUp`'s action verb or a distinct
  one) remains empirically unconfirmed — required pre-DELIVER spike,
  mirroring `OQ-CA-01`/`OQ-CHI-01`/`OQ-OAP-01`'s own precedent, not a
  blocker to this ADR's logical contract (Decision 5 specifies both
  dispatch-mechanics outcomes).
- REST-wide rate limiting remains an un-addressed gap (Decision 7) — named,
  not silently inherited, and materially sharper for this feature than for
  its three siblings (zero credential beyond `api_key`).

## References

- `docs/feature/anonymous-sessions/feature-delta.md` §§ Job Discovery
  Framing Resolution, System Constraints, Handoff Package
- `docs/product/architecture/adr-002-bounded-contexts.md` § Option D, §
  Changed Assumptions
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
- `docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`
- `docs/product/architecture/adr-036-hosted-identity-bounded-context-and-storage.md`
- `docs/product/architecture/adr-037-oauth-providers-signing-key-and-verification-composition.md`
  (direct structural precedent for this ADR's storage/routing-widening
  methodology; § Consequences' own forward-looking guidance directly
  motivates Decision 2)
- `docs/product/architecture/adr-044-anonymous-sessions-no-backend-mode-gating.md`
- `docs/product/architecture/adr-045-anonymous-sessions-token-ttl-reuse-no-refresh.md`
- `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md`
- `crates/embyr-core/src/client_identity/mod.rs`
- `crates/embyr-server/src/admin/handlers/{hosted_identity,oauth_providers}.rs`,
  `crates/embyr-server/src/rest/{sign_up,sign_in_with_idp}.rs`,
  `crates/embyr-server/src/adapters/{system_db,encryption}.rs`,
  `crates/embyr-server/src/grpc/handler.rs::attach_client_identity_if_present`,
  `crates/embyr-server/src/lib.rs::accounts_bridge_dispatch`

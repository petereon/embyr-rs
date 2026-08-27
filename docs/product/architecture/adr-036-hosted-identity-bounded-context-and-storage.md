# ADR-036: Hosted Identity — Bounded Context, Storage Split, and Composition With `client-auth`

## Status

Accepted

## Context

`client-auth-hosted-identity` (JOB-18) reverses `client-auth`'s own locked
out-of-scope line and adds embyr-hosted email/password signup, signin, and
password reset for Trailmark's end users who have no backend of their own to
mint a custom token from (feature-delta.md § Job Discovery Framing
Resolution). Two decisions are LOCKED by DISCUSS and orchestrator-confirmed,
not reopened here:

1. **Storage**: hosted-identity data lives in **Customer DB**, project-scoped,
   and hosted identity is **structurally gated to `backend_mode ∈
   {direct_pg, aws_secret, gcp_secret}`** — `backend_mode=agent` projects are
   refused enablement outright (Resolution 2, orchestrator-confirmed
   2026-08-27).
2. **Signing/session material** MUST be a structurally disjoint entity from
   `client_identity_credentials` (ADR-024/025) — never colocated, never
   sharing a row or custody boundary (Resolution 3).

What DISCUSS left to DESIGN: bounded-context placement (flag 6), exact schema
(table names, column shapes, which database each table lives in), the
`backend_mode` enforcement mechanism, the password-reset mechanism, the
Argon2id-reuse mechanism, and the driving-port/protocol-surface shape for
Maria's (not Alex's) actions. This ADR resolves all of them as one coherent
design — they are tightly coupled (the storage split drives the composition
shape, which drives the driving-port shape), mirroring ADR-025/026's and
ADR-029's own precedent of bundling closely-related decisions into one ADR
rather than fragmenting a single coherent design across several.

## Decision Drivers

1. **Resolution 2/3 compliance** — Customer DB for account data, structural
   disjointness for signing material — non-negotiable inputs, not choices.
2. **Reuse over reinvention** (Reuse Analysis, mandatory) — every primitive
   this feature needs (Argon2id, ECIES, credential caching, backend-mode
   branching, `verify_client_identity_token()`, `IEmailSender`) already
   exists in the codebase in a directly reusable shape.
3. **Zero changes to `verify_client_identity_token()`** (Resolution 3, locked)
   — the pure verification function's signature and body must not change;
   only its caller's credential-resolution step may change.
4. **Structural, not conventional, `backend_mode=agent` refusal** — mirrors
   ADR-026's own "physically unreachable, not merely unexercised" discipline
   applied to US-01's enablement gate.
5. **ADR-002's Option-D three-part test governs bounded-context placement**,
   applied fresh, not by inertia (the same discipline ADR-029/BC-4 already
   established).

## Decision 1 — Bounded-Context Placement: BC-5 Hosted Identity (new)

Applying ADR-002's Option-D three-part test directly to the candidate Account
entity:

| Test | Result |
|---|---|
| Entity with identity of its own? | **Yes** — `(project_id, email)` uniquely identifies a hosted-identity Account, distinct from every BC-1 `Project` and every BC-2 `Document`. |
| Lifecycle of its own? | **Yes** — create (signup) → reset (password change) → [deferred: disable/delete]. A real, if currently short, lifecycle — not a stateless translation. |
| Invariants of its own? | **Yes** — email uniqueness per project, password-strength rules, single-use/expiring reset tokens. None of these invariants exist anywhere else in the codebase today. |

This is the **identical pattern** BC-4 Access Control passed (ADR-029) and
Credential Resolution failed (ADR-002 § Option D, still correctly rejected —
nothing here reopens that). Folding hosted identity into BC-1 (Tenant
Management, System-DB-only by its own storage boundary — § BC-1) or BC-2
(Document Storage, whose entire vocabulary is `Document`/`Transaction`/
`Index`, none of which describes an Account) would repeat the exact reasoning
gap ADR-002's own `Changed Assumptions` amendment already corrected once for
BC-4.

**Decision: a fifth bounded context, BC-5 Hosted Identity, is added.**

**BC-5 responsibility**: owns the lifecycle of a hosted-identity `Account`
(email, Argon2id password hash) and its password-`ResetToken`s. Owns the
embyr-generated signing key used to mint tokens for accounts it authenticates.
**Storage boundary — split, not single**: `Account`/`ResetToken` live in
Customer DB (Resolution 2); the signing key lives in System DB (Decision 2,
below) — the first bounded context in this codebase whose storage boundary is
genuinely split across both databases, a new fact this ADR names explicitly
rather than silently inheriting BC-1's or BC-2's single-database assumption.

**Context Map addition** (additive, mirrors BC-4's own addition to ADR-002 §
Context Map Summary — not a rewrite of BC-1/BC-2/BC-3/BC-4's existing
entries):

```
BC-5 Hosted Identity
    → BC-1 Tenant Management  [read-only: Project.backend_mode gate at enablement (US-01)
                                and at signup/signin resolution (Decision 4); reads and
                                decrypts its OWN signing key from System DB]
    → BC-2 Document Storage   [shares the SAME Customer DB connection BC-2 already
                                resolves per request via the SAME PostgresBackendAdapter
                                type — not a new kind of database access, a second
                                CONSUMER of an already-established one]
    → (produces) VerifiedEndUserIdentity [same output type as BC-1's client-auth path —
                                zero new consumer-facing type]
```

This directly de-risks the DISCUSS-flagged concern ("a new subsystem writing
into Customer DB is a genuinely new *kind* of inter-context relationship"):
at the **mechanism** level it is not new — BC-5 writes to Customer DB through
the identical `PostgresBackendAdapter` struct and the identical
`migrations/customer/` single-embed-point (ADR-022) BC-2 already established;
only new inherent methods and two new migration files are added, zero new
connection-resolution code and zero new migration-embed macro invocation. At
the **bounded-context-relationship** level it is new — BC-5 is the first
context besides BC-2 to depend on Customer DB directly — and this ADR states
that plainly rather than glossing over it.

## Decision 2 — Storage Split: Account/ResetToken in Customer DB, Signing Key in System DB

Two structurally disjoint entities, deliberately stored in **different**
databases for two **different** reasons:

### `hosted_identity_accounts` (Customer DB, new table)

```sql
CREATE TABLE hosted_identity_accounts (
    project_id     VARCHAR(63)    NOT NULL,
    end_user_id    UUID           NOT NULL DEFAULT gen_random_uuid(),
    email          TEXT           NOT NULL,
    password_hash  TEXT           NOT NULL,
    created_at     TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, email)
);
```

- `project_id` carried explicitly, mirroring `documents`' own shape
  (`migrations/customer/0001_documents.sql`) even though a single `direct_pg`
  Customer DB is conventionally one project's own database — the existing
  `aws_secret`/`gcp_secret` modes can, in principle, point more than one
  project at the same physical Postgres, so BC-2's own convention of never
  assuming database-level single-tenancy is reused here, not reinvented.
- `PRIMARY KEY (project_id, email)` makes "email already registered" (AC-18-06)
  a database-enforced unique-violation → mapped to a `409 EMAIL_ALREADY_IN_USE`
  response — the identical "constraint, not app-level check" discipline
  ADR-025 established for `client_identity_credentials`' own registration.
- `end_user_id` (a UUID, not the email) is what gets minted into the `sub`
  claim (Decision 3) — mirrors real Firebase's own `localId` being distinct
  from the email address, and avoids ever putting an end user's email address
  inside a bearer token that subsequently travels on every Firestore request
  header.
- **Rejected**: storing `password_hash` in System DB (Resolution 2's own
  rejected Option A) — reopens the exact PII-commingling concern DISCUSS
  already rejected; not reconsidered here.

### `hosted_identity_reset_tokens` (Customer DB, new table, Slice 04)

```sql
CREATE TABLE hosted_identity_reset_tokens (
    project_id   VARCHAR(63)    NOT NULL,
    email        TEXT           NOT NULL,
    token_hash   BYTEA          NOT NULL,
    expires_at   TIMESTAMPTZ(6) NOT NULL,
    used_at      TIMESTAMPTZ(6),
    created_at   TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, token_hash)
);
CREATE INDEX hosted_identity_reset_tokens_lookup
    ON hosted_identity_reset_tokens (project_id, email)
    WHERE used_at IS NULL;
```

- `token_hash` is `BLAKE3(raw token)` — the raw token is never stored,
  directly reusing `admin/handlers/auth.rs::signin`'s own
  `sessions.token_hash`/`mfa_recovery_codes.code_hash` convention.
- Single-use enforced by an atomic
  `UPDATE ... SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL
  RETURNING email` — a row is only ever consumed once, race-free, mirroring
  `mfa_recovery_codes`' own used_at IS NULL check-then-mark shape but fused
  into one statement (no separate SELECT-then-UPDATE race window).
- Expiry: 1 hour (`RESET_TOKEN_EXPIRY_SECS = 3600`), a DESIGN-level default
  (DISCUSS did not lock a number); chosen as a conventional, unsurprising
  password-reset window. Expiry check is `expires_at < now()` at confirm time,
  independent of `used_at`, giving the three distinguishable rejection
  reasons AC-18-16 requires (expired vs. already-used vs. malformed/unknown
  token) without a fourth error class.

### `hosted_identity_signing_keys` (**System DB**, new table — deliberately NOT Customer DB)

```sql
CREATE TABLE hosted_identity_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,        -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,        -- ECIES-encrypted 32-byte Ed25519 seed
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Why System DB, not Customer DB, even though Resolution 2 put the Account
data in Customer DB**: the signing key is not Trailmark's data — it is
embyr's own key material, and the whole point of the Ed25519 design (ADR-024)
is that **embyr, and only embyr, may hold anything that mints a token that
verifies as authentic**. In `backend_mode=direct_pg`, the customer's own DBA
holds full DDL/DML privilege over their own Customer DB (ADR-022's own
context: the customer applies `migrations/customer/` under elevated,
one-time credentials before handing embyr a lower-privilege connection
string). Placing embyr's own private signing key inside a database the
customer administers would hand that DBA a standing, silent,
audit-trail-free way to mint a valid identity for **any** end user of their
own project — a materially stronger and stealthier capability than what
Resolution 2 already, knowingly, accepted for the Account row itself (a DBA
who edits `password_hash` directly still has to drive a real, logged sign-in
attempt through Argon2id verification to use it). System DB — a database only
embyr's own SaaS operates — is where BC-1 already keeps every other
project-scoped control-plane secret (`api_key_hash_current`,
`ecies_encrypted_dsn`, `client_identity_credentials`), and this key is
exactly that kind of thing: a control-plane secret, not project data.

**`PRIMARY KEY (project_id)`, no `_previous`/rotation columns** — YAGNI: no
story in this feature's locked v1 scope (US-01–US-04) requires rotating the
embyr-owned signing key (unlike ADR-025's customer-registered credential,
which US-03 of `client-auth` explicitly rotates). If a future feature needs
it, ADR-025's exact dual-generation shape (`_current`/`_previous`,
current-then-previous verification order) is the established, directly
reusable pattern — noted here as the upgrade path, not built now.

**`private_key_enc`, never plaintext at rest**: encrypted with
`embyr_core::auth::ecies::encrypt(&ecies::derive_public_key(api_key), private_key_seed)`
— the **identical** primitive and the **identical** "key material re-derived
from the project's own `api_key`, never itself stored" pattern
`ecies_encrypted_dsn` already uses (CLAUDE.md's own stated invariant: "private
key re-derived from API key"). This is possible because, unlike the
customer-registered public key (ADR-025, which has no confidentiality
property and is stored plaintext by design), embyr's own signing key IS a
secret with a confidentiality property to protect, and — critically — every
call site that needs to *use* it (signup, signin, reset-confirm; Decision 4)
already requires the project's `api_key` for an unrelated reason (resolving
the Customer DB adapter), so decryption costs zero new key-management
surface: the exact same `api_key` bytes already on the request unlock both
the DSN and the signing key. A raw System DB backup/leak reveals neither.
**Verification** (an ordinary Firestore call, ADR-026 step 4) needs only
`public_key` — never `private_key_enc`, never touches the ECIES path at all,
so the existing "Ed25519 verify is cheap, no cache required" cost profile
(ADR-024 § Consequences) is unaffected for the read side.

**Rejected**: storing `private_key_enc` alongside `client_identity_credentials`
(Resolution 3's own named-and-rejected shortcut) — not reconsidered.
**Rejected**: a bespoke new encryption scheme for the signing key — ECIES is
already a vetted, workspace-resident primitive; a second scheme would be pure
duplication for no security benefit.

## Decision 3 — Token Minting: Extend `embyr_core::client_identity`, Not a New Module

`embyr_core::client_identity` currently only **verifies** (customer mints,
embyr verifies). Hosted identity requires embyr itself to **mint**. The new
pure function is added to the *same* module, not a new one:

```
pub fn mint_client_identity_token(
    signing_key_seed: &[u8; 32],
    end_user_id: &str,
    project_id: &str,
    expires_at_unix: i64,
) -> String
```

**Why EXTEND, not a new module**: minting is the exact mirror operation of
`verify_client_identity_token()` — same `ClientIdentityClaims` shape
(`sub`/`aud`/`exp`), same JWT/EdDSA wire format (ADR-024), same crate
(`jsonwebtoken`, `ed25519-dalek`, both already workspace dependencies, both
already imported by this module's own test helpers — see
`client_identity/mod.rs::tests::mint_token`, which already hand-rolls this
exact encoding for test fixtures; production minting promotes that shape out
of `#[cfg(test)]` into the module's public surface rather than duplicating
it a third time). A separate `embyr_core::hosted_identity_minting` module
would fragment one small, coherent "client-identity token" concern (claims
shape + algorithm + encoding) across two modules for no isolation benefit —
both still produce/consume the identical wire format, verified by the
identical `verify_client_identity_token()`. Zero changes to
`verify_client_identity_token()` itself (Resolution 3, locked) — `mint_*` is
additive.

**No custom claims support at mint time in v1** — `ClientIdentityClaims`
already supports `#[serde(flatten)] extra` (ADR-034/`custom-claims`) on the
*verification* side; hosted-identity signup (US-02) mints with an empty extra
map, matching every pre-existing token — zero behavior change, and
consistent with this feature's own locked v1 scope (no custom-claims story
in US-01–US-04).

## Decision 4 — Verification-Time Credential Routing (extends ADR-026 step 4)

A project may have `client_identity_credentials` (customer-minted),
`hosted_identity_signing_keys` (embyr-minted), both, or neither (System
Constraints: "coexists with, does not replace"). ADR-026 step 4's existing
code, unchanged in shape, tries the ORIGINAL `client_identity_credentials`
lookup first (100% backward-compatible — a `client-auth`-only project's
successful-verification code path is untouched, byte-for-byte, and costs it
nothing new). Only on absence or verification failure does it now also
attempt `hosted_identity_signing_keys`:

```
[4] x-embyr-client-identity present?
      absent → unchanged (no VerifiedEndUserIdentity attached)
      present →
        client_identity_credentials row exists?
          yes → verify_client_identity_token(token, project_id, that credential)
                success → attach identity, DONE
                failure → fall through
        hosted_identity_signing_keys row exists?
          yes → verify_client_identity_token(token, project_id,
                  ClientIdentityCredential{ public_key_current: hosted.public_key,
                                            public_key_previous: None })
                success → attach identity, DONE
                failure → fall through
        neither matched / both failed → attach nothing (UNCHANGED failure behavior —
                                          still never rejects the request, ADR-026)
```

Both attempts call the **identical, unchanged**
`verify_client_identity_token()` — this is a straightforward widening of
ADR-025's own "try current key, then try previous key" pattern one level up
("try credential *source* A, then source B"), not a new mechanism: a
correctly-formed token from either source will only ever verify against its
own signer's public key; a wrong-source attempt fails deterministically
(same cost profile as ADR-025's own rotation-window retry, tens of
microseconds). For a project with **neither** table populated (today's
default, and every project that never uses either identity mechanism), both
lookups return `None` and step 4 reduces exactly to its pre-`client-auth`
behavior — the regression guardrail (AC-16-08/AC-18-13) holds by the same
"physically unreachable, not merely unexercised" structural argument ADR-026
already established, now extended to a second credential source without
weakening it.

**Enforcement**: unit test — a project with only `hosted_identity_signing_keys`
populated verifies a hosted-identity-minted token and rejects a
`client-auth`-shaped forgery exactly as `Malformed`; a project with **both**
populated verifies each token type correctly regardless of which credential
is checked first; a project with **neither** never queries either table for a
request with no `x-embyr-client-identity` header (proves the branch remains
structurally unreachable, mirroring ADR-026's own enforcement test).

## Decision 5 — `backend_mode` Enablement Gate (US-01)

`backend_mode` is already read once per admin-adjacent flow via
`SystemDb::get_project_for_auth` (auth/data-plane) and inline in
`admin/handlers/provision.rs` (provisioning). Neither is reachable/appropriate
from a *session*-authenticated admin action (`client_identity.rs`'s own
handlers use `verify_project_ownership`, which does not surface
`backend_mode`). A new, narrow `SystemDb` method is added:

```
pub async fn get_project_backend_mode(
    &self,
    project_id: &str,
    account_id: Uuid,
) -> Result<Option<ProjectBackendModeRow>, CoreError>
// SELECT backend_mode FROM projects WHERE id = $1 AND account_id = $2 AND status != 'deleted'
```

folding ownership + backend_mode into one query (mirrors
`verify_project_ownership`'s own WHERE-clause shape, extended with one column
and one predicate) rather than two round trips. `enable_hosted_identity`
(new handler, `admin/handlers/hosted_identity.rs`) calls this; `None` → 404
(mirrors `verify_project_ownership`); `Some(row)` with
`row.backend_mode == "agent"` → **`403 Forbidden` with a named reason**
(`HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE`) — a hard rejection at the
enablement action itself, exactly as DISCUSS requires ("not a warning, not an
opt-in override").

**Defense in depth, not solely convention**: Decision 2's structural choice —
BC-5's Customer DB resolver (`resolve_customer_db_adapter`, Decision 6) only
ever constructs a `PostgresBackendAdapter`, and its `backend_mode == "agent"`
branch returns an error *before* attempting to build any adapter at all —
means even a hypothetical future project that changed `backend_mode` to
`agent` *after* enabling hosted identity (a scenario this feature does not
build a migration path for, and does not need to: enablement and every
signup/signin/reset call each independently re-check `backend_mode` from the
live `projects` row, never a cached decision) cannot reach Customer DB writes
for hosted identity — refused at both the admin action and, independently, at
every subsequent data-plane call.

## Decision 6 — Driving Port: REST `:8081`, `?key=<api_key>` Query Param (extends ADR-026)

Maria's (not Alex's) actions — signup, signin, reset-request, reset-confirm —
are **not** admin-session actions (Maria has no admin session; only Alex
does, for US-01). They are data-plane, end-user-facing actions, following
`client-auth`'s own precedent of a state struct scoped to exactly what the
route needs, not `UserAdminState` (ADR-026, `SignInState`).

**Unlike `signInWithCustomToken()`** (ADR-026: "no auth header of its own —
the token in the body IS the credential"), hosted-identity's four endpoints
**require the project's own `api_key`**, because — per Decision 1's Context
Map — BC-5 must resolve a **Customer DB** connection to read/write
`hosted_identity_accounts`/`hosted_identity_reset_tokens`, and every existing
Customer-DB-resolution path in this codebase (`grpc/handler.rs::authenticate`)
requires the project's `api_key` to Argon2id-authenticate the caller and
ECIES-decrypt the DSN. This is a genuine, evidenced difference from the
custom-token bridge (which is stateless and needs no Customer DB access at
all) — not an arbitrary inconsistency. It also happens to match real
Firebase's own Identity Toolkit REST surface, which **does** carry `?key=`
on every one of `accounts:signUp`/`accounts:signInWithPassword`/
`accounts:sendOobCode`/`accounts:resetPassword` (unlike the ADR-026 bridge,
which mirrors a *different*, credential-only-shaped real endpoint) —
additional, though secondary, evidence for `OQ-CHI-01` (see § Open Questions).

Proposed contract (provisional pending `OQ-CHI-01`'s required pre-DELIVER
spike, exactly as ADR-026 flags `OQ-CA-01` — the *logical* contract below is
implementation-ready regardless of the spike's outcome):

```
POST /v1/projects/{project_id}/accounts:signUp?key={api_key}
  Body:  { "email": "...", "password": "..." }
  200:   { "localId": "<end_user_id>", "email": "...", "expiresIn": "<seconds>" }
  400:   { "reason": "MISSING_FIELD" | "WEAK_PASSWORD" | "HOSTED_IDENTITY_NOT_ENABLED" }
  409:   { "reason": "EMAIL_ALREADY_IN_USE" }

POST /v1/projects/{project_id}/accounts:signInWithPassword?key={api_key}
  Body:  { "email": "...", "password": "..." }
  200:   { "localId": "<end_user_id>", "email": "...", "expiresIn": "<seconds>" }
  400:   { "reason": "HOSTED_IDENTITY_NOT_ENABLED" }
  401:   { "reason": "INVALID_LOGIN_CREDENTIALS" }   -- oracle-protected (AC-18-11): identical
                                                          for wrong password AND unknown email

POST /v1/projects/{project_id}/accounts:sendOobCode?key={api_key}
  Body:  { "email": "..." }
  200:   { "message": "if this account exists, a reset was sent" }  -- ALWAYS this shape (AC-18-14)

POST /v1/projects/{project_id}/accounts:resetPassword?key={api_key}
  Body:  { "oobCode": "<reset token>", "newPassword": "..." }
  200:   { "email": "..." }
  400:   { "reason": "WEAK_PASSWORD" }
  401:   { "reason": "RESET_TOKEN_EXPIRED" | "RESET_TOKEN_INVALID" }  -- covers already-used/malformed/unknown, one class
```

Endpoint names (`accounts:signUp`, `accounts:signInWithPassword`,
`accounts:sendOobCode`, `accounts:resetPassword`) are chosen to mirror the
real, well-known Identity Toolkit REST surface (the same reasoning ADR-026's
own Handoff flag 5 already names as "well-known," making `OQ-CHI-01` "likely
more acute" — i.e., more confidently guessable, not less).

**New composition-root state**, mirroring `SignInState`'s own minimality
discipline:

```
pub struct HostedIdentityState {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
}
```

Wired in `lib.rs::spawn_all_servers` alongside `sign_in_state`, merged into
the same `:8081` `axum_app` — no new listener, no new port (feature-delta.md
§ Driving Ports already confirms this).

## Decision 7 — Customer DB Adapter Resolution: New, Narrow Function, Zero Changes to `authenticate()`

A new function, `crates/embyr-server/src/adapters/project_auth.rs`
(new file):

```
pub async fn resolve_customer_db_adapter(
    system_db: &SystemDb,
    credential_cache: &CredentialCache,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    project_id: &str,
    api_key: &str,
) -> Result<Arc<PostgresBackendAdapter>, ProjectAuthError>
```

composes **exclusively pre-existing, already-independently-callable
primitives** — `SystemDb::get_project_for_auth`, `argon2::verify_api_key`,
`ecies::decrypt`, `PostgresBackendAdapter::new`, `AwsSecretFetcher::get_dsn`,
`GcpSecretFetcher::get_dsn`, `CredentialCache::get`/`insert` — in the
identical order and with the identical status/Argon2id-fast-path discipline
`grpc/handler.rs::authenticate` already established. **Zero lines of
`authenticate()` change.** This is a new, small (~70-line) function, not an
extraction/refactor of the existing one — chosen over refactoring
`authenticate()` to return a generic resolver both callers share, because
`authenticate()`'s return type and error type (`tonic::Status`) are
gRPC-specific, and forcing a shared abstraction across a `tonic::Status`
world and an `axum::http::StatusCode` world was judged to cost more
indirection than the ~70 lines of duplicated *shape* (not primitives) it
would save — the identical reasoning ADR-025's own Alternative 1 already
applied to "try current then previous" across three cryptographically
distinct checks.

**Structurally cannot resolve `backend_mode=agent`**: the function's
`agent` branch returns `Err(ProjectAuthError::HostedIdentityUnavailable)`
immediately — it never constructs an `AgentBackendAdapter` at all. This is a
type-level guarantee (`Arc<PostgresBackendAdapter>` is the function's only
`Ok` shape), not a runtime convention — mirroring Earned Trust's "wire then
probe then use" discipline applied to a resolution function rather than a
startup probe.

## Decision 8 — Argon2id Reuse: Thin Named Wrappers, Not `hash_api_key(password.as_bytes())`

`embyr_core::auth::argon2` gains two additive functions:

```
pub fn hash_password(password: &[u8]) -> Result<String, CoreError>
pub fn verify_password(password: &[u8], phc_hash: &str) -> Result<bool, CoreError>
```

Bodies are byte-for-byte identical to `hash_api_key`/`verify_api_key` — both
call the same private `argon2_instance()` (the single source of truth for
`memory=65536KiB, iter=3, par=4`, CLAUDE.md's own stated standard). **Calling
`hash_api_key(password.as_bytes())` directly at a hosted-identity call site
was considered and rejected** — it compiles and behaves identically, but
reads misleadingly (a password is not an API key) at every future call site
a maintainer edits. A ~10-line named wrapper costs nothing at runtime (the
compiler inlines it), shares the identical `argon2_instance()` constant (so
the two parameter sets can never silently drift apart — the actual risk this
feature's own § System Constraints names: "a divergent parameter set for
end-user vs. admin passwords would be an unjustified, un-evidenced technology
choice"), and is cheaper than either duplicating the Argon2id parameters a
second time or accepting a misleading call site. This is the **shared
helper**, not duplicated-with-identical-constants, answer DISCUSS's own
Technical Notes explicitly asked DESIGN to choose between.

## Decision 9 — Password Strength Rule

**Minimum 8 characters, no composition rules** (no forced uppercase/digit/
symbol). New pure function, `embyr_core::hosted_identity::validate_password_strength(password: &str) -> Result<(), PasswordTooWeak>`
— the one genuinely new pure module this feature adds to `embyr-core`
(justified: password-strength validation is a hosted-identity-specific
domain rule that exists nowhere else in the codebase; it does not belong in
`client_identity` (token concerns) or `auth::argon2` (hashing mechanics, not
policy)). Rationale: NIST SP 800-63B's current guidance favors length over
composition-rule complexity (composition rules measurably push users toward
predictable substitutions without improving effective entropy). `Result`,
not a bare `bool`, so `AC-18-07`'s "naming the specific requirement" is a
structural consequence of the type, not a string built ad hoc at the call
site.

## Decision 10 — Customer DB Migration Mechanism: Zero New Mechanism Needed

`migrations/customer/0003_hosted_identity_accounts.sql` and
`migrations/customer/0004_hosted_identity_reset_tokens.sql` are added to the
**existing** `migrations/customer/` directory. ADR-022 already established
`PostgresBackendAdapter::migrate()` (backed by one static
`sqlx::migrate!("../../migrations/customer")` `Migrator`) as the **sole**
embed point in the entire workspace, consumed identically by `embyr-server`'s
provisioning path and by the standalone `embyr-db-prep` binary. New files
dropped into that directory are picked up automatically by both consumers —
**this feature requires no new migration mechanism, no new embed site, and
does not touch `embyr-db-prep` at all.** This is an explicit finding, not an
assumption: ADR-022's single-sourcing already generalizes to any future
Customer DB table, including this feature's.

`migrations/0028_hosted_identity_signing_keys.sql` is added to the existing,
unrelated System DB `migrations/` directory (`SystemDb::migrate()`,
`sqlx::migrate!("../../migrations")`) — a second, pre-existing, independent
mechanism, unchanged by this feature.

## Consequences

### Positive

- Zero changes to `verify_client_identity_token()`, zero changes to
  `authenticate()`, zero changes to `IEmailSender`, zero new crate
  dependency, zero new migration mechanism.
- Every new secret (the embyr-owned private signing key) is encrypted at
  rest with the exact same proven primitive and key-derivation discipline
  the existing DSN already uses — no new cryptographic surface to review.
- `backend_mode=agent` refusal is enforced twice, independently, at two
  different layers (admin enablement gate; Customer-DB-adapter resolution
  function that cannot construct an agent adapter) — neither depends on the
  other remaining correct.
- BC-5's Customer DB write dependency reuses BC-2's exact adapter/migration
  mechanism — no new inter-context *mechanism*, only a new inter-context
  *relationship*, named explicitly rather than conflated.

### Negative / Trade-offs

- BC-5 is the first bounded context in this codebase with a storage boundary
  split across both databases (Account/ResetToken in Customer DB, signing key
  in System DB) — a genuinely new fact about this system's architecture,
  documented here rather than left implicit.
- `resolve_customer_db_adapter` duplicates the *shape* (not the primitives)
  of `authenticate()`'s backend-mode branching in ~70 lines — an accepted,
  named trade-off (Decision 7), not an oversight.
- `OQ-CHI-01` (endpoint paths, `?key=` query-param placement, whether the
  Firebase JS SDK's Auth methods can be pointed at a non-Google backend the
  same way `signInWithCustomToken()`/Firestore already proved redirectable)
  remains empirically unconfirmed — required pre-DELIVER spike, mirroring
  `OQ-CA-01`'s own precedent, not a blocker to this ADR's logical contract.
- Password reset (US-04) ships v1 on `NoopEmailSender` (log-only, per
  `IEmailSender`, ADR-011) — Maria's reset flow is functionally complete
  end-to-end (token minted, persisted, consumed) but no actual email is
  delivered to her until the separately-tracked `SmtpEmailSender` V2 slice
  lands. This is a **named shipping-gate assumption**, not an oversight: US-04
  is DoD-complete against the port contract, not against real-world delivery.
  Any go-live decision for hosted identity must treat email delivery as a
  distinct, still-open dependency.

## Decision 11 — Observability: Structured Logging Only in v1, No New Metrics Subsystem

Direct code read of `rest/sign_in.rs` (`sign_in_with_custom_token`, the exact
REST precedent this feature's own driving port mirrors, Decision 6) confirms
it carries **zero** observability instrumentation today — no metrics, no
tracing, nothing. This is a **pre-existing gap across this codebase's entire
REST surface** (ADR-016's own Prometheus metrics only ever instrumented
gRPC, via `obs_helpers::record_grpc_call`), not a gap this feature
introduces.

**v1 decision**: hosted identity's 4 new REST handlers (signup, signin,
reset-request, reset-confirm) each emit a `tracing::info!`/`tracing::warn!`
structured log line on success/failure (operation name, project id, outcome
— never the password, never the reset token), matching the observability
posture every OTHER REST handler in this codebase already has (none —
structured logs are this codebase's actual current REST-observability
floor, not a lowered bar invented for this feature).

**Explicitly deferred, not this feature's job**: building a REST-equivalent
of `obs_helpers`'s gRPC counter/histogram family (a genuine, cross-cutting
REST-observability initiative spanning every existing REST handler, not
just hosted identity's own 4 new ones) is named here as a candidate
follow-up — instrumenting only hosted identity's own endpoints while every
other REST handler stays dark would produce an inconsistent, arbitrarily
partial metrics surface, a worse outcome than the current uniform silence.

## References

- `docs/feature/client-auth-hosted-identity/feature-delta.md` §§ Job Discovery
  Framing Resolution, System Constraints, Handoff Package
- `docs/product/architecture/adr-002-bounded-contexts.md` § Option D, §
  Changed Assumptions
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
- `docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`
- `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`
  (BC-4 precedent for this ADR's own bounded-context-placement methodology)
- `docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`
- `docs/product/architecture/adr-011-email-sender-port.md`
- `crates/embyr-core/src/client_identity/mod.rs`,
  `crates/embyr-core/src/auth/{argon2,ecies}.rs`,
  `crates/embyr-server/src/grpc/handler.rs::authenticate`,
  `crates/embyr-server/src/rest/sign_in.rs`,
  `crates/embyr-server/src/admin/handlers/{client_identity,provision,shared}.rs`,
  `crates/embyr-pg-storage/src/backend_adapter.rs`

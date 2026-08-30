# ADR-044: Anonymous Sessions — No `backend_mode=agent` Gating; Amends
# ADR-036 Decision 5's Scope Language

## Status

Accepted

## Context

DISCUSS's Escalation 2 (`docs/feature/anonymous-sessions/feature-delta.md`
§ Handoff Package) asks whether anonymous sign-in needs the same
`backend_mode=agent` refusal `client-auth-hosted-identity` structurally
required at enablement (ADR-036 Decision 5), and notes this is the same
question `oauth-providers`' own DISCUSS independently raised and — per that
feature's ADR — already resolved: **no gating** (ADR-037 § Context: "No
`backend_mode=agent` gating... Resolution 2, confirmed 2026-08-30"). DISCUSS
asks DESIGN to resolve this fresh for anonymous-sessions, not by inertia,
and to settle the general question if the reasoning generalizes.

This ADR does two things: (1) verifies, by reading the actual shipped code
rather than trusting the ADR-037 prose, that oauth-providers really shipped
with zero `backend_mode` gating; (2) states precisely *why* — not just
*that* — hosted-identity needed the gate and oauth-providers did not, so the
rule generalizes correctly to anonymous-sessions instead of being copied by
surface analogy.

## Ground-Truth Verification (Not Assumed)

Two files were read in full:

- `crates/embyr-server/src/admin/handlers/hosted_identity.rs`
  (`enable_hosted_identity`) — confirmed to call
  `state.system_db.get_project_backend_mode(&project_id, session.account_id)`
  and hard-reject with `403 HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE`
  when `backend_mode == "agent"`, BEFORE any signing-key generation.
- `crates/embyr-server/src/admin/handlers/oauth_providers.rs`
  (`register_google_oauth_provider`) — confirmed to call ONLY
  `verify_project_ownership` (which reads `account_id`, never
  `backend_mode`) before generating and storing its signing key. **No
  `backend_mode` lookup of any kind exists in this handler.**

Additionally, `crates/embyr-server/src/rest/sign_in_with_idp.rs`
(`sign_in_with_idp`, oauth's own sign-in path) was read in full: it also
performs no `backend_mode` check anywhere — its only preconditions are the
existence of an `oauth_provider_credentials` row and a valid, verifiable
Google ID token.

**oauth-providers shipped with genuinely zero `backend_mode` gating, at
either its enablement or its sign-in call site — confirmed directly, not
inferred from the ADR.**

## Why the Gate Exists at All: It Protects `resolve_customer_db_adapter`,
Not "Hosted Identity" as a Concept

Re-reading ADR-036 Decision 5/7 together makes the actual mechanism precise:
the `backend_mode=agent` refusal exists because
`resolve_customer_db_adapter` (`crates/embyr-server/src/adapters/project_auth.rs`,
ADR-036 Decision 7) is **structurally incapable of constructing an adapter
for `backend_mode=agent` projects** — its `agent` branch returns
`Err(ProjectAuthError::HostedIdentityUnavailable)` immediately, never
attempting to build an `AgentBackendAdapter`. Hosted identity's signup/signin
call this function to resolve a Customer DB connection to read/write
`hosted_identity_accounts`. Enabling hosted identity for an agent-mode
project would therefore let Alex enable a feature whose every subsequent
call is *guaranteed* to fail at Customer DB resolution — the gate exists to
fail fast, at enablement, instead of failing confusingly later, on every
signup attempt.

**The gate is therefore not "a property of hosted-identity-shaped
features."** It is a property of **any feature whose runtime path calls
`resolve_customer_db_adapter` (or otherwise requires a direct Customer DB
Postgres connection)**. A feature that never calls that function has no
failure mode for the gate to prevent — there is nothing to fail fast on.

## Applying the Rule to `oauth-providers` (Retroactive Confirmation) and
`anonymous-sessions` (Fresh Application)

`oauth-providers`: confirmed by ground truth (above) to never call
`resolve_customer_db_adapter` anywhere — Google ID-token verification and
signing-key decryption are both wholly System-DB/`EMBYR_ENCRYPTION_KEY`-based
(ADR-037 Decision 2), never touching a Customer DB connection. Zero gating
is therefore the *correct* outcome, not merely the *shipped* one — this ADR
retroactively confirms ADR-037's own Resolution 2 was right for the reason
this ADR now states precisely, not simply asserted.

`anonymous-sessions`: under ADR-043 Decision 4 (stateless minting, confirms
DISCUSS Resolution 3) and ADR-043 Decision 2 (AES-256-GCM/
`EMBYR_ENCRYPTION_KEY`, confirms Resolution 2), this feature **never calls
`resolve_customer_db_adapter`, at either its enablement handler
(`anonymous_identity.rs`, ADR-043 Decision 3) or its sign-in handler
(`sign_in_anonymously.rs`, ADR-043 Decision 5)**. Both call sites verify the
project's `api_key` directly via `SystemDb::get_project_for_auth` +
`argon2::verify_api_key` — never constructing, or attempting to construct, a
Customer DB adapter of any kind. The precondition the gate exists to protect
against (a feature enabled for a project whose subsequent calls are
guaranteed to fail at Customer DB resolution) **structurally cannot occur**
for this feature, for the identical reason it cannot occur for
oauth-providers.

**Decision: `anonymous-sessions` requires NO `backend_mode=agent` gating, at
either US-01's enablement handler or US-02's sign-in handler. This applies
uniformly to every `backend_mode` value (`direct_pg`, `aws_secret`,
`gcp_secret`, `agent`) — an agent-mode project may enable and use anonymous
sign-in exactly as freely as any other project, matching oauth-providers'
own precedent exactly.**

## Amendment to ADR-036 Decision 5

ADR-036 Decision 5's own text is **not reopened or rewritten** — hosted
identity's own gate remains correct and unchanged, for its own,
still-valid reason (it genuinely calls `resolve_customer_db_adapter`). This
ADR adds a scoping clarification, appended to ADR-036 as a note under its
own Decision 5, so a future reader does not generalize "hosted identity
needs this gate" into "every embyr-mints-its-own-token feature needs this
gate" (the exact over-generalization this Escalation asked DESIGN to avoid):

> **Scope clarification (added by ADR-044, `anonymous-sessions`)**: this
> gate is a property of `resolve_customer_db_adapter`'s own structural
> incapability for `backend_mode=agent` projects, not a property of "embyr
> mints its own token" as a category. Confirmed against `oauth-providers`
> (ADR-037, ships with zero gating) and `anonymous-sessions` (ADR-044,
> ships with zero gating) — both call sites verified, by direct code read,
> to never call `resolve_customer_db_adapter`. A future feature needing this
> gate should ask one question: does any call site in this feature's own
> runtime path call `resolve_customer_db_adapter` (or otherwise require a
> direct Customer DB Postgres connection)? If no, no gate is needed,
> regardless of how similar the feature otherwise looks to hosted identity.

## Consequences

### Positive

- Settles the identical open question `oauth-providers`' own DISCUSS raised
  for itself, with a precise, generalizable mechanism-level rule rather than
  a second feature-by-feature guess.
- `anonymous-sessions`' enablement and sign-in handlers are simpler than
  hosted-identity's own (no `get_project_backend_mode` call, no
  `403 UNAVAILABLE_FOR_BACKEND_MODE` branch) — one fewer DB round trip, one
  fewer failure mode to test.
- Agent-mode projects — a real, already-shipped `backend_mode` this codebase
  supports — get full anonymous-sign-in parity with every other
  `backend_mode`, matching real Firebase's own `signInAnonymously()`, which
  has no analogous backend-topology restriction at all.

### Negative / Trade-offs

- None identified. There is no scenario under Resolution 2/3 (AES-256-GCM,
  stateless) in which an agent-mode project's anonymous sign-in could fail
  for a Customer-DB-resolution reason — the gate would be pure friction with
  no corresponding safety benefit.

## References

- `docs/feature/anonymous-sessions/feature-delta.md` § Handoff Package,
  Escalation 2
- `docs/product/architecture/adr-036-hosted-identity-bounded-context-and-storage.md`
  Decisions 5, 7
- `docs/product/architecture/adr-037-oauth-providers-signing-key-and-verification-composition.md`
  § Context, Resolution 2
- `docs/product/architecture/adr-043-anonymous-sessions-signing-key-custody-and-driving-port.md`
  Decisions 2, 3, 4, 5
- `crates/embyr-server/src/admin/handlers/{hosted_identity,oauth_providers}.rs`,
  `crates/embyr-server/src/rest/sign_in_with_idp.rs`,
  `crates/embyr-server/src/adapters/project_auth.rs::resolve_customer_db_adapter`

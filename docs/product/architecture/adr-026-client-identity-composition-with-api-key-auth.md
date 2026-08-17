# ADR-026: Composing Client-Identity Verification With the Existing api_key Auth Path

## Status

Accepted

## Context

This is the single highest-consequence design risk named in DISCUSS's Handoff Package:
"an ordinary Firestore data call from a session that never signed in must keep working
exactly as it does today. Do not silently make identity mandatory on data-plane calls."
(AC-16-08, KPI #3 guardrail — a regression here would trade a narrow identity-gap-fix for
a broad regression across all 72 existing `embyr-rs` acceptance scenarios.)

The existing auth path (`handler.rs::extract_api_key` / `authenticate`,
confirmed by full read during DISCUSS and re-confirmed here) does exactly one thing per
request: extract a bearer credential from the gRPC `authorization` metadata key, treat it
as the project `api_key`, and use it for three simultaneous roles — project
identification, Argon2id-verified authorization, and ECIES decryption key material for
the stored customer DSN. There is exactly one `authorization` metadata slot per request in
the current wire protocol.

US-02 requires that a *different* credential — a client-identity token (ADR-024) —
somehow also reach embyr on the same request path, without touching or replacing that
existing slot, and requires the resulting `VerifiedEndUserIdentity` to be available to
"embyr's own request handling for at least the duration of that signed-in session"
(AC-16-09) for a follow-up Security Rules epic to consume.

A second, related question this ADR must answer: does embyr mint its own second-order
session artifact after the first successful verification (mirroring Firebase's real
two-tier custom-token → ID-token model), or does the client simply resend the original
customer-minted token on every call?

## Decision Drivers

1. **Structural, not tested-only, regression safety.** AC-16-08 is the guardrail metric
   (KPI #3); the design must make "an unsigned-in session is unaffected" true by
   construction — the new code path must be physically unreachable for a request that
   never presents client-identity material — not merely true because nobody happened to
   trigger the new branch in the acceptance suite.
2. **No re-architecture of `api_key`'s three roles** (System Constraints, non
   -negotiable).
3. **Simplicity precedent** — AD-03 already rejected inventing a session-token layer for
   the existing `api_key` check ("adds token issuance, rotation, and revocation logic not
   present in SPEC"). The same reasoning applies to a hypothetical embyr-issued
   client-identity session token.
4. **Shared verification routine** (System Constraints) — sign-in (US-02) and
   debug-verify (US-04) must not maintain two copies of verification logic.

## Considered Options — Carrying Identity to Subsequent Calls

### Option A: embyr mints its own short-lived session JWT after first successful verify

Mirrors Firebase's actual two-tier model (customer-minted custom token → Firebase
-minted ID token). embyr would sign a new JWT (a new embyr-owned signing key,
`embyr_client_session_key` or similar) at sign-in, and the SDK would attach that
token on subsequent calls.

**Rejected.** Introduces an entirely new embyr-owned secret (yet another key needing its
own rotation story, on top of `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`/project
`api_key`/the new ADR-025 credential) purely to avoid re-verifying an Ed25519 signature
that is already cheap (tens of microseconds, no KDF cost — unlike Argon2id) on every
call. This is exactly the class of complexity AD-03 already rejected for the simpler
`api_key` case, for a weaker reason (Argon2id *is* expensive; Ed25519 verify is not).

### Option B: Stateless per-request re-verification of the original customer-minted token — Accepted

The client resends the *same* Trailmark-minted token (from ADR-024) on every subsequent
Firestore call, in a new, separate header/metadata key. embyr re-verifies it against
`client_identity_credentials` (ADR-025) on every call, exactly as it already re-verifies
`api_key` via Argon2id on every call (cache aside).

**Accepted.** No new embyr-owned signing key. No session-issuance, rotation, or
revocation logic to design (there is nothing to revoke — a rotated-away customer key
naturally stops verifying once its rotation window closes, per ADR-025). Directly
extends the existing "authenticate per request, cache the *resolved adapter*, not the
authentication decision" model (AD-03) to a second, independent credential dimension.
Trivially satisfies US-04's "no live session created" requirement — there is no session
object anywhere in this design to create.

## Decision — Wire Composition

**A new, separate, optional gRPC metadata key / HTTP header carries the client-identity
token. The existing `authorization` metadata key is untouched.**

- gRPC metadata key: `x-embyr-client-identity` (lowercase, per gRPC metadata key
  constraints), value `Bearer <token>`.
- REST/HTTP header: `X-Embyr-Client-Identity: Bearer <token>`.

The auth interceptor (`handler.rs::authenticate`, called on every gRPC/REST data-plane
request) is extended, **not replaced**, with one additional, independent step that runs
*after* the existing `api_key` check completes exactly as it does today:

```
[1] Extract project_id, load Project, check status, Argon2id verify api_key
    (UNCHANGED — exact existing code path, exact existing error surface)
[2] Rate limit check (UNCHANGED)
[3] AdapterForProject (UNCHANGED)
[4] NEW, additive: is `x-embyr-client-identity` present on this request?
      absent → proceed with no VerifiedEndUserIdentity attached (UNCHANGED BEHAVIOR)
      present → verify_client_identity_token() (ADR-024/025)
                 success → attach VerifiedEndUserIdentity to request context
                 failure → attach nothing; DOES NOT reject the request
                           (rejection only happens at the dedicated sign-in action,
                           US-02 AC-16-07 — never on an ordinary data call, per the
                           locked guardrail)
```

Step 4's failure branch deliberately does not reject the request. This is a specific,
load-bearing design choice: an *expired* client-identity token presented alongside a
perfectly valid `api_key` on an ordinary `getDoc` call must not turn a previously-working
call into a failure — Security Rules (the dependent follow-up epic) is what will
eventually decide whether missing/expired identity should gate access to a *specific*
document; this feature must not pre-empt that decision by silently making step 4 a hard
gate. This directly implements the DISCUSS constraint: "the only place a missing token is
rejected is the sign-in/verification action itself... not downstream data calls."

**Why this structurally guarantees the regression guardrail, not just tests it:** an
unsigned-in session, by definition, never sends `x-embyr-identity`/
`X-Embyr-Client-Identity`. Step 4's new code is gated on that header's presence. There is
no code path by which a request lacking the header can reach the new verification logic
at all — the guardrail holds even for test scenarios nobody thought to write, because the
new branch is unreachable, not merely unexercised.

### Sign-in action (US-02) — front-loaded verification for a clean SDK-facing signal

A dedicated action performs the identical `verify_client_identity_token()` call
up front and returns a clear success/failure signal, giving `signInWithCustomToken()`'s
promise a definite resolve/reject before the SDK attempts any Firestore call — matching
the real SDK's existing UX contract (sign-in either succeeds or fails with a specific
reason, independent of any subsequent data call). Proposed contract (REST port `:8081`,
since Firebase's real Auth REST API is always HTTP/JSON regardless of Firestore's own
gRPC/REST split):

```
POST /v1/projects/{project_id}/accounts:signInWithCustomToken
Body:     { "token": "<trailmark-minted token>" }
200:      { "localId": "<end_user_id>", "expiresIn": "<seconds-until-exp>" }
400:      { "reason": "MISSING_TOKEN" | "MALFORMED_TOKEN" | "TOKEN_EXPIRED" | "PROJECT_MISMATCH" }
```

This endpoint and the debug-verify endpoint (US-04, ADR-025) both call the identical
`embyr_core::client_identity::verify_client_identity_token()` — satisfying the
single-shared-verification-routine constraint structurally (one function, two call
sites), not by convention.

## Open Question (flagged, not resolved — mirrors OQ-02/OQ-03 precedent)

**OQ-CA-01**: The exact request path shown above (`POST
/v1/projects/{project_id}/accounts:signInWithCustomToken`, plus a new
`x-embyr-client-identity` header attached by the SDK to subsequent Firestore calls)
is this ADR's best-evidence proposal, but it depends on the real Firebase JS SDK's
actual, closed-source wire behavior in two specific ways that cannot be confirmed from
this codebase alone:

1. Does `signInWithCustomToken()`, when pointed at a non-Google backend (via emulator
   -style host override, the same mechanism the rest of `embyr-rs` already relies on for
   Firestore itself), POST to a URL path embyr controls the shape of, or a fixed
   Identity-Toolkit-specific path embyr must replicate exactly?
2. Once signed in, does the Firestore SDK's subsequent gRPC/REST calls attach the
   resulting credential in a header embyr can freely define (this ADR's assumption), or
   does the SDK's internal call-credentials machinery insist on reusing the *same*
   `Authorization` slot already used for the `api_key` — which would require a different,
   header-collision-aware design?

This is squarely the same class of uncertainty as OQ-02 (gRPC-Web/CORS fidelity) and
OQ-03 (BrowserChannel protocol completeness) already logged in the Application
Architecture section — "only discoverable by running the actual Firebase JS SDK." Per
those precedents, this is not blocking DESIGN's architecture (the *logical* contract —
what gets verified, against what, with what rejection taxonomy — is fully specified and
implementation-ready regardless of the answer), but it is a required DISTILL/DELIVER
-wave empirical spike before the sign-in transport can be considered final. If (2)
resolves against this ADR's assumption, the fallback is a `dual_auth`-style
precedence check on the single `Authorization` header (try `api_key` Argon2id first,
unchanged; only on failure attempt client-identity verification) — directly reusable from
ADR-009's `dual_auth_middleware` "tries session then operator" shape — rather than a
second header.

## Consequences

### Positive

- Zero lines of the existing `authenticate()` happy path change. The new logic is
  additive and appended, not interleaved with the existing three-role `api_key` check.
- No new embyr-owned signing key, no session-issuance/rotation/revocation surface.
- The guardrail (AC-16-08) is enforced by the *shape* of the code (new branch gated on a
  header's presence), which the DISTILL-wave regression suite can assert directly by
  running the existing 72 `embyr-rs` scenarios completely unmodified alongside the new
  ones.

### Negative / Trade-offs

- Re-verifies the client-identity token's signature on every call rather than caching the
  verification result. Acceptable for V1 given Ed25519's low per-call cost (Decision
  Driver 3); a `CredentialCache`-shaped optimization (keyed by token hash, matching the
  existing `(project_id, BLAKE3(api_key))` pattern) is a straightforward, non-architecture
  -changing follow-up if profiling later shows it is warranted.
- OQ-CA-01 means the exact transport is not yet empirically validated against the real
  SDK — flagged explicitly rather than asserted as final, per Earned Trust discipline.

## Enforcement

- Integration test (DISTILL wave, mandatory): run the full existing 72-scenario
  `embyr-rs` acceptance suite unmodified against a build that includes this feature,
  asserting 0 regressions (directly operationalizes KPI #3).
- Integration test: a request carrying a valid `api_key` and an *expired*
  `x-embyr-client-identity` token succeeds on an ordinary `getDoc` call (proves step 4's
  failure branch does not reject).
- Unit test: `x-embyr-client-identity` absent → no `VerifiedEndUserIdentity` attached,
  zero calls into `embyr_core::client_identity` (proves the branch is structurally
  unreachable, not just untriggered).

## References

- `docs/feature/client-auth/feature-delta.md` §§ System Constraints, Handoff Package
  (flag 2)
- `docs/product/architecture/adr-024-client-identity-verification-mechanism.md`
- `docs/product/architecture/adr-025-client-identity-credential-storage-rotation.md`
- `docs/product/architecture/adr-009-auth-middleware-separation.md` (dual_auth_middleware
  precedence-check precedent, referenced as OQ-CA-01's fallback shape)
- `crates/embyr-server/src/grpc/handler.rs::authenticate`
- `docs/product/architecture/brief.md` § Open Questions (OQ-02, OQ-03 — same class of
  SDK-fidelity uncertainty)
